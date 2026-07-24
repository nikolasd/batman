//! `fake-worker`: a deterministic stand-in vendor process used by
//! supervisor and adapter conformance tests. Never calls a model; every
//! mode is a small, real protocol/behavior fixture selected with
//! `--mode <name>`.
//!
//! Modes:
//! - `jsonl`: reads newline-delimited JSON from stdin, echoes one
//!   acknowledgement line per input line.
//! - `jsonrpc`: a minimal JSON-RPC 2.0 responder (generic `initialize` +
//!   echo).
//! - `acp`: a minimal Agent Client Protocol responder that always
//!   negotiates protocol version 1, regardless of what the caller
//!   requested (mirroring the real Copilot/OMP-RPC probe behavior the
//!   design spec documents).
//! - `omp-rpc`: emits the `{"type":"ready"}` handshake frame before
//!   reading anything, then distinguishes prompt acknowledgement from
//!   prompt completion.
//! - `flood`: writes one stdout line far larger than any adapter's
//!   bounded frame limit, then floods stderr well past any rotating
//!   capture cap, to prove both bounds hold. Self-terminates after a
//!   bounded amount of output so a forgotten `terminate()` never leaves
//!   an immortal process.
//! - `ignore-term`: ignores SIGINT and SIGTERM (only SIGKILL ends it), to
//!   prove cancellation escalation actually reaches SIGKILL.
//! - `crash-after-ack`: writes one acknowledgement frame, then exits
//!   immediately with a distinctive nonzero code.

use std::io::{BufRead, Write};

use clap::Parser;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    mode: Mode,
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum Mode {
    Jsonl,
    Jsonrpc,
    Acp,
    OmpRpc,
    Flood,
    IgnoreTerm,
    CrashAfterAck,
    /// Supervisor-test-only helper (not part of the plan's adapter-facing
    /// mode list): reports the *names* of every environment variable this
    /// process can see, never a value, so `Supervisor::spawn`'s
    /// environment isolation can be proven end to end rather than only at
    /// the `EnvironmentPolicy::build` unit level.
    EnvProbe,
}

fn main() {
    let args = Args::parse();
    match args.mode {
        Mode::Jsonl => run_jsonl(),
        Mode::Jsonrpc => run_jsonrpc(),
        Mode::Acp => run_acp(),
        Mode::OmpRpc => run_omp_rpc(),
        Mode::Flood => run_flood(),
        Mode::IgnoreTerm => run_ignore_term(),
        Mode::CrashAfterAck => run_crash_after_ack(),
        Mode::EnvProbe => run_env_probe(),
    }
}

fn write_line(out: &mut impl Write, value: &serde_json::Value) {
    let text = serde_json::to_string(value).expect("fixture values always serialize");
    let _ = out.write_all(text.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
}

fn run_jsonl() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for (seq, line) in stdin.lock().lines().enumerate() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let parsed: serde_json::Value =
            serde_json::from_str(&line).unwrap_or_else(|_| serde_json::json!({ "raw": line }));
        write_line(
            &mut stdout,
            &serde_json::json!({ "echo": parsed, "seq": seq }),
        );
    }
}

fn run_jsonrpc() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let result = if method == "initialize" {
            serde_json::json!({
                "protocolVersion": 1,
                "serverInfo": { "name": "fake-worker", "version": "0.1.0" }
            })
        } else {
            serde_json::json!({ "method": method, "echoedParams": request.get("params") })
        };
        write_line(
            &mut stdout,
            &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        );
    }
}

fn run_acp() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let method = request.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let result = match method {
            // Always negotiates protocol 1, regardless of the caller's
            // requested version -- matching the real probe behavior
            // documented for Copilot 1.0.73 and OMP 17.0.7.
            "initialize" => serde_json::json!({
                "protocolVersion": 1,
                "agentCapabilities": { "loadSession": true },
                "agentInfo": { "name": "fake-worker", "version": "0.1.0" }
            }),
            "session/new" => serde_json::json!({ "sessionId": "fake-session-1" }),
            "session/list" => serde_json::json!({ "sessions": [] }),
            other => serde_json::json!({ "method": other, "echoedParams": request.get("params") }),
        };
        write_line(
            &mut stdout,
            &serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        );
    }
}

fn run_omp_rpc() {
    let mut stdout = std::io::stdout();
    // The ready-frame handshake happens before anything is read.
    write_line(&mut stdout, &serde_json::json!({ "type": "ready" }));

    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let command = request
            .get("command")
            .and_then(|c| c.as_str())
            .unwrap_or("");
        match command {
            "prompt" => {
                // Prompt acceptance is a distinct frame from completion.
                write_line(
                    &mut stdout,
                    &serde_json::json!({ "type": "prompt_ack", "id": id }),
                );
                write_line(
                    &mut stdout,
                    &serde_json::json!({ "type": "prompt_result", "id": id, "data": { "agentInvoked": false } }),
                );
            }
            "get_state" => {
                write_line(
                    &mut stdout,
                    &serde_json::json!({ "type": "state", "id": id, "data": { "status": "idle" } }),
                );
            }
            other => {
                write_line(
                    &mut stdout,
                    &serde_json::json!({ "type": "result", "id": id, "data": { "command": other } }),
                );
            }
        }
    }
}

/// One stdout line far larger than any adapter's bounded frame limit
/// (`4 MiB`), then stderr spam well past any rotating capture cap
/// (`25 MiB`). Self-terminates after a bounded total so a forgotten
/// `terminate()` call never leaves an immortal process.
fn run_flood() {
    let mut stdout = std::io::stdout();
    let big_line = "A".repeat(8 * 1024 * 1024);
    let _ = stdout.write_all(big_line.as_bytes());
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();

    let mut stderr = std::io::stderr();
    let chunk = "E".repeat(1024 * 1024);
    let started = std::time::Instant::now();
    let mut written = 0usize;
    const SAFETY_CAP_BYTES: usize = 64 * 1024 * 1024;
    while written < SAFETY_CAP_BYTES && started.elapsed() < std::time::Duration::from_secs(5) {
        if stderr.write_all(chunk.as_bytes()).is_err() {
            break;
        }
        let _ = stderr.write_all(b"\n");
        let _ = stderr.flush();
        written += chunk.len();
    }
}

fn run_ignore_term() {
    // SAFETY: installing a signal handler via a well-defined, static
    // libc-level handler (SIG_IGN) is the documented, sound use of
    // `nix::sys::signal::signal`; no callback closure or unwind-across-FFI
    // concern applies here.
    unsafe {
        let _ = nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGINT,
            nix::sys::signal::SigHandler::SigIgn,
        );
        let _ = nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTERM,
            nix::sys::signal::SigHandler::SigIgn,
        );
    }
    // Emitted only after the handlers are installed, so a caller can
    // deterministically wait for this frame before sending any signal --
    // removing the startup race between "process forked" and "signal
    // handlers installed" that a fixed sleep would only approximate.
    let mut stdout = std::io::stdout();
    write_line(&mut stdout, &serde_json::json!({ "ready": true }));
    // Blocks on stdin, harmlessly, until SIGKILL (which cannot be
    // ignored) ends the process.
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        line.clear();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}

fn run_crash_after_ack() {
    let mut stdout = std::io::stdout();
    write_line(&mut stdout, &serde_json::json!({ "ack": true }));
    std::process::exit(17);
}

fn run_env_probe() {
    let mut names: Vec<String> = std::env::vars().map(|(name, _)| name).collect();
    names.sort();
    let mut stdout = std::io::stdout();
    write_line(&mut stdout, &serde_json::json!({ "envNames": names }));
}
