//! Raw, pre-normalization Claude Code `stream-json` wire shapes.
//!
//! Grounded against the installed `claude` 2.1.219 CLI's documented
//! `--output-format stream-json` behavior (`claude --help`, `claude auth
//! status`) plus the Agent SDK's own published message-type reference
//! (<https://code.claude.com/docs/en/headless>,
//! <https://code.claude.com/docs/en/agent-sdk/typescript> -- the
//! `SDKMessage` union and its variants), never invented ad hoc. Only the
//! fields [`super::normalize`] actually consumes are typed here. No
//! struct below uses `deny_unknown_fields`: the vendor protocol is
//! explicitly forward-compatible (the SDK docs repeatedly say to "ignore
//! values you don't recognize"), so an unrecognized sibling field or
//! `type`/`subtype` must never be a hard parse error.

use serde::Deserialize;
use serde_json::Value;

/// One parsed line of Claude's `stream-json` output.
///
/// Dispatch on `type`/`subtype` is manual (via [`RawFrame::parse`])
/// rather than a serde internally-tagged enum, so a structurally valid
/// but unrecognized shape falls through to [`RawFrame::Unrecognized`]
/// instead of a hard deserialize error -- forward-compatible with new
/// message types the vendor CLI adds later.
#[derive(Debug, Clone)]
pub enum RawFrame {
    SystemInit(RawSystemInit),
    HookStarted(RawHookLifecycle),
    /// Parsed and dispatched on (`hook_event`), but its payload is
    /// never read further: a `hook_progress` frame carries no
    /// approval-relevant signal this adapter normalizes -- see
    /// `super::normalize`.
    #[allow(dead_code)]
    HookProgress(RawHookLifecycle),
    HookResponse(RawHookLifecycle),
    Assistant(RawChatMessage),
    User(RawChatMessage),
    StreamEvent(RawStreamEvent),
    Result(RawResult),
    /// A structurally valid frame whose `type`/`subtype` this adapter
    /// does not (yet) normalize. Never an error.
    Unrecognized,
}

impl RawFrame {
    /// Parses one `stream-json` line.
    ///
    /// # Errors
    /// Returns `Err` only for bytes that are not valid JSON, or that
    /// deserialize to a recognized `type`/`subtype` but are missing a
    /// field this adapter requires for it (a genuinely malformed frame).
    /// An unrecognized `type`/`subtype` is `Ok(Self::Unrecognized)`, not
    /// an error.
    pub fn parse(line: &[u8]) -> Result<Self, serde_json::Error> {
        let value: Value = serde_json::from_slice(line)?;
        let kind = value.get("type").and_then(Value::as_str).unwrap_or("");
        match kind {
            "system" => {
                let subtype = value.get("subtype").and_then(Value::as_str).unwrap_or("");
                match subtype {
                    "init" => Ok(Self::SystemInit(serde_json::from_value(value)?)),
                    "hook_started" => Ok(Self::HookStarted(serde_json::from_value(value)?)),
                    "hook_progress" => Ok(Self::HookProgress(serde_json::from_value(value)?)),
                    "hook_response" => Ok(Self::HookResponse(serde_json::from_value(value)?)),
                    _ => Ok(Self::Unrecognized),
                }
            }
            "assistant" => Ok(Self::Assistant(serde_json::from_value(value)?)),
            "user" => Ok(Self::User(serde_json::from_value(value)?)),
            "stream_event" => Ok(Self::StreamEvent(serde_json::from_value(value)?)),
            "result" => Ok(Self::Result(serde_json::from_value(value)?)),
            _ => Ok(Self::Unrecognized),
        }
    }
}

/// `SDKSystemMessage` (`subtype: "init"`). Only `session_id` is needed --
/// the adapter's own `probe()`/vendor-fact reporting covers tools/model/
/// mcp_servers/skills/plugins separately, and those ambient fields are
/// exactly why `ProbeResult::inventory_incomplete` is `true`.
#[derive(Debug, Clone, Deserialize)]
pub struct RawSystemInit {
    pub session_id: String,
}

/// Common shape of `SDKHookStartedMessage`/`SDKHookProgressMessage`/
/// `SDKHookResponseMessage`. `outcome`/`output` are only ever present on
/// a `hook_response` frame; `stdout`/`stderr` (present on progress and
/// response too) are intentionally not modeled here since this adapter's
/// only interest in hook lifecycle is the `PermissionRequest` approval
/// signal, not general hook stdout capture.
#[derive(Debug, Clone, Deserialize)]
pub struct RawHookLifecycle {
    pub hook_id: String,
    pub hook_name: String,
    pub hook_event: String,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub output: Option<String>,
}

/// `SDKAssistantMessage`/`SDKUserMessage`. `message.usage` (per-message
/// token usage) is deliberately not modeled: this adapter only reports
/// usage at run granularity, from the final `result` frame (see
/// [`AdapterCapabilities::usage`]'s `Aggregate` declaration in
/// `super::mod`).
#[derive(Debug, Clone, Deserialize)]
pub struct RawChatMessage {
    pub session_id: String,
    #[serde(default)]
    pub parent_tool_use_id: Option<String>,
    pub message: RawMessageBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawMessageBody {
    #[serde(default)]
    pub content: Vec<RawContentBlock>,
}

/// A single Anthropic Messages API content block, as carried by
/// `message.content`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RawContentBlock {
    Text {
        text: String,
    },
    Thinking {
        /// Deserialized (so the block still parses structurally) but
        /// deliberately never read anywhere -- `super::normalize`
        /// discards every `thinking` block before it could ever become
        /// part of an `AdapterEvent`. See that module's doc.
        #[allow(dead_code)]
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        /// The tool's raw arguments -- deserialized for structural
        /// completeness but not read: this adapter's `ToolStarted`
        /// event carries only the tool's id/name, never its arguments.
        #[serde(default)]
        #[allow(dead_code)]
        input: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Unrecognized,
}

/// `SDKPartialAssistantMessage` (`type: "stream_event"`, only present
/// when `--include-partial-messages` is set, which this adapter's
/// command line always passes). Stream events are only ever emitted for
/// the main session (`parent_tool_use_id` is always `null` per the
/// vendor docs), so no subagent correlation is needed here.
#[derive(Debug, Clone, Deserialize)]
pub struct RawStreamEvent {
    pub event: RawStreamDelta,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawStreamDelta {
    #[serde(default)]
    pub delta: Option<RawTextDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawTextDelta {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
}

/// `SDKResultMessage`. `result` (the run's final answer text) is present
/// only on the `subtype: "success"` arm; `total_cost_usd`/`usage` are
/// present on every arm. `subtype`/`is_error` discriminate the error
/// arms (`error_max_turns`, `error_during_execution`, ...) — both are
/// optional because a success arm may omit them (R12).
#[derive(Debug, Clone, Deserialize)]
pub struct RawResult {
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub subtype: Option<String>,
    #[serde(default)]
    pub is_error: Option<bool>,
    pub total_cost_usd: f64,
    pub usage: RawUsage,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
