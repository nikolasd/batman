//! Per-adapter coordination MCP launch helpers: the argv/env/config each
//! adapter's command builder injects to give its supervised vendor
//! process access to the worker coordination tools (`batman_task`,
//! `batman_send`, etc.) via a `batcave coordination-mcp` subprocess the
//! vendor CLI itself spawns as its own MCP server -- see
//! `crate::coordination::mcp` for that subprocess, and
//! `crate::coordination::mcp_protocol` for the tool schemas it serves.
//!
//! Every adapter's own native MCP/plugin/skill/hook discovery stays on:
//! nothing here ever adds a flag that suppresses or replaces it, only
//! one additional named server (`"batman"`) alongside whatever the
//! vendor CLI already loads from the user/project's own config.
//!
//! OMP-RPC has no separate MCP subprocess of its own to inject this
//! into at all: `omp --mode rpc`'s "host tools" are invoked over the
//! *same* RPC channel the adapter already owns (a `host_tool_call`
//! frame on its stdout, answered with a `host_tool_result` on its
//! stdin -- see `crate::adapter::omp_rpc`'s own host-tool bridge), so
//! it never goes through this module or the scope-token-authenticated
//! socket at all: the runtime process making that in-process call is
//! the vendor's own parent, never a descendant of it, so it could not
//! authenticate over that socket even if it tried (ancestry is checked
//! in the wrong direction). `CoordinationBroker::execute_tool_call`
//! (`crate::coordination::broker`) is the shared, in-process
//! counterpart both paths ultimately resolve to.

use std::collections::HashMap;
use std::path::PathBuf;

use batman_protocol::RunId;
use serde_json::{Value, json};

/// Everything a supervised vendor process's command builder needs to
/// wire up the coordination MCP server: where the verified `batcave`
/// binary lives, this run's state/repository paths, and the run it's
/// scoped to.
#[derive(Debug, Clone)]
pub struct McpLaunchContext {
    pub batcave_path: PathBuf,
    pub state_dir: PathBuf,
    pub repository: PathBuf,
    pub run_id: RunId,
}

/// The `coordination-mcp` subcommand argv, as separate arguments --
/// never shell-joined, so no argument can be split or injected by
/// embedded whitespace in a path.
#[must_use]
pub fn coordination_mcp_argv(context: &McpLaunchContext) -> Vec<String> {
    vec![
        "coordination-mcp".to_string(),
        "--state-dir".to_string(),
        context.state_dir.display().to_string(),
        "--repo".to_string(),
        context.repository.display().to_string(),
        "--run-id".to_string(),
        context.run_id.to_string(),
    ]
}

/// The environment addition for the supervised vendor process (never
/// this runtime's own): only `BATMAN_WORKER_SCOPE_TOKEN`. The vendor
/// process inherits it into whatever MCP-server child it spawns for
/// `coordination-mcp`; that subprocess reads and removes the variable
/// from its own environment immediately (see
/// `crate::coordination::mcp::ScopeTokenSource`), before it ever
/// touches the socket.
#[must_use]
pub fn coordination_mcp_env(scope_token: &str) -> HashMap<String, String> {
    let mut env = HashMap::new();
    env.insert(
        "BATMAN_WORKER_SCOPE_TOKEN".to_string(),
        scope_token.to_string(),
    );
    env
}

/// The MCP server config block (`{"command":...,"args":[...]}`) every
/// stdio-MCP-consuming adapter embeds under a `"batman"` server name --
/// shaped identically for both Claude and Copilot; only how each
/// adapter *delivers* the surrounding document (a file path vs. an
/// inline JSON argument) differs.
#[must_use]
fn coordination_mcp_server_config(context: &McpLaunchContext) -> Value {
    json!({
        "command": context.batcave_path.display().to_string(),
        "args": coordination_mcp_argv(context),
    })
}

/// The full MCP config document `{"mcpServers":{"batman":{...}}}` both
/// Claude's `--mcp-config` file and Copilot's `--additional-mcp-config`
/// inline argument carry -- identical shape, different delivery.
#[must_use]
pub fn coordination_mcp_config_document(context: &McpLaunchContext) -> Value {
    json!({ "mcpServers": { "batman": coordination_mcp_server_config(context) } })
}

/// The two `-c` override arguments Codex's `codex app-server` command
/// line receives to register the same server as `mcp_servers.batman`
/// without a config file, preserving every other loaded Codex config.
/// Codex's `-c key=value` overrides parse `value` as a TOML value, not
/// JSON -- a TOML basic string for the command, a TOML array of basic
/// strings for args.
#[must_use]
pub fn codex_mcp_overrides(context: &McpLaunchContext) -> Vec<String> {
    let command_value = toml_basic_string(&context.batcave_path.display().to_string());
    let args_value = toml_basic_string_array(&coordination_mcp_argv(context));
    vec![
        "-c".to_string(),
        format!("mcp_servers.batman.command={command_value}"),
        "-c".to_string(),
        format!("mcp_servers.batman.args={args_value}"),
    ]
}

/// TOML's basic-string escape table (spec: every control character must
/// be escaped; a basic string cannot contain one literally). A binary
/// path is exceptionally unlikely to contain a raw newline or other
/// control character, but this never assumes it can't -- every value
/// this module ever needs to embed (a filesystem path, an argv value)
/// is escaped completely, not just for the two characters common paths
/// happen to use.
fn toml_basic_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\u{8}' => escaped.push_str("\\b"),
            '\t' => escaped.push_str("\\t"),
            '\n' => escaped.push_str("\\n"),
            '\u{c}' => escaped.push_str("\\f"),
            '\r' => escaped.push_str("\\r"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                escaped.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => escaped.push(c),
        }
    }
    escaped.push('"');
    escaped
}

/// A TOML array of basic string literals (`["a", "b"]`).
fn toml_basic_string_array(values: &[String]) -> String {
    let items: Vec<String> = values.iter().map(|v| toml_basic_string(v)).collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> McpLaunchContext {
        McpLaunchContext {
            batcave_path: PathBuf::from("/opt/batman/bin/batcave"),
            state_dir: PathBuf::from("/tmp/batman-state"),
            repository: PathBuf::from("/tmp/my-repo"),
            run_id: RunId::new(),
        }
    }

    #[test]
    fn argv_is_separate_arguments_never_shell_joined() {
        let context = context();
        let argv = coordination_mcp_argv(&context);
        assert_eq!(
            argv,
            vec![
                "coordination-mcp",
                "--state-dir",
                "/tmp/batman-state",
                "--repo",
                "/tmp/my-repo",
                "--run-id",
                &context.run_id.to_string(),
            ]
        );
    }

    #[test]
    fn env_carries_only_the_scope_token() {
        let env = coordination_mcp_env("a-token");
        assert_eq!(env.len(), 1);
        assert_eq!(
            env.get("BATMAN_WORKER_SCOPE_TOKEN"),
            Some(&"a-token".to_string())
        );
    }

    #[test]
    fn config_document_matches_the_mcp_server_config_shape_claude_and_copilot_both_expect() {
        let context = context();
        let document = coordination_mcp_config_document(&context);
        assert_eq!(
            document["mcpServers"]["batman"]["command"],
            "/opt/batman/bin/batcave"
        );
        let args = document["mcpServers"]["batman"]["args"].as_array().unwrap();
        assert_eq!(args[0], "coordination-mcp");
        assert_eq!(args.len(), 7);
        // Exactly one server entry -- an adapter merges this under its
        // own already-loaded servers, never replacing them.
        assert_eq!(document["mcpServers"].as_object().unwrap().len(), 1);
    }

    #[test]
    fn codex_overrides_are_two_dash_c_pairs_with_toml_value_syntax() {
        let context = context();
        let overrides = codex_mcp_overrides(&context);
        assert_eq!(overrides.len(), 4);
        assert_eq!(overrides[0], "-c");
        assert_eq!(
            overrides[1],
            "mcp_servers.batman.command=\"/opt/batman/bin/batcave\""
        );
        assert_eq!(overrides[2], "-c");
        assert!(overrides[3].starts_with("mcp_servers.batman.args=[\"coordination-mcp\", "));
        assert!(overrides[3].ends_with(']'));
    }

    #[test]
    fn codex_overrides_escape_toml_special_characters_in_paths() {
        let context = McpLaunchContext {
            batcave_path: PathBuf::from("/opt/batman \"quoted\"/bin/batcave"),
            state_dir: PathBuf::from("/tmp/batman-state"),
            repository: PathBuf::from("/tmp/my-repo"),
            run_id: RunId::new(),
        };
        let overrides = codex_mcp_overrides(&context);
        // The escaped value must still be a single valid TOML basic
        // string: exactly one unescaped opening and one unescaped
        // closing quote around the whole path.
        assert_eq!(
            overrides[1],
            "mcp_servers.batman.command=\"/opt/batman \\\"quoted\\\"/bin/batcave\""
        );
    }

    #[test]
    fn codex_overrides_escape_control_characters_not_just_backslash_and_quote() {
        let context = McpLaunchContext {
            batcave_path: PathBuf::from("/opt/batman\n\t/bin/batcave"),
            state_dir: PathBuf::from("/tmp/batman-state"),
            repository: PathBuf::from("/tmp/my-repo"),
            run_id: RunId::new(),
        };
        let overrides = codex_mcp_overrides(&context);
        assert_eq!(
            overrides[1],
            "mcp_servers.batman.command=\"/opt/batman\\n\\t/bin/batcave\""
        );
        // The escaped value never contains a raw control character --
        // every byte from here on is a printable TOML basic-string body.
        assert!(!overrides[1].contains('\n'));
        assert!(!overrides[1].contains('\t'));
    }
}
