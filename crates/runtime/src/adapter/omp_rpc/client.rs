//! The OMP-RPC stdio client: the `{"type":"ready",...}` handshake, request/
//! response correlation by `id`, and pure command-frame builders.
//!
//! The wire shapes below are grounded against the real installed `omp
//! 17.1.1` binary, not invented:
//! - The ready handshake (`omp --mode rpc --model <selector> --no-session
//!   --allow-home < /dev/null`) actually emits
//!   `{"type":"ready","protocolVersion":1,"supportedProtocolVersions":[1,2],
//!   "maxFrameBytes":1048576,"maxReassembledFrameBytes":67108864}` before
//!   reading anything.
//! - A `{"type":"get_state","id":"1"}` request against the real binary
//!   returns `{"id":"1","type":"response","command":"get_state",
//!   "success":true,"data":{...,"sessionId":"<uuid>",
//!   "sessionFile":"<path>",...}}` -- the OMP session id/file the plan's
//!   Interfaces section calls out as this adapter's `VendorSessionRef`.
//! - `{"type":"get_session_stats","id":"1"}` returns
//!   `data.tokens.{input,output,...}` and `data.cost`, an aggregate
//!   (session-lifetime, not per-turn) usage shape.
//! - The command names and their parameter field names below are read
//!   directly out of the installed binary's own (minified) RPC dispatch
//!   switch, e.g. `case "prompt": { const H = await kI1(A, E.message,
//!   E.streamingBehavior) ... }`, `case "steer": { await A.steer(E.message,
//!   ...) }`, `case "follow_up": { await A.followUp(E.message, ...) }`,
//!   `case "set_model": { ... E.provider ... E.modelId ... }`, and `case
//!   "set_subagent_subscription": { ... uNw(E.level) ...
//!   z.setSubscriptionLevel(E.level) ... }` -- confirming the real
//!   parameter names are `message`, `provider`/`modelId`, and `level`
//!   respectively, not the plan text's unqualified prose.
//!
//! Real, unsolicited event frames (e.g. `extension_ui_request`,
//! `available_commands_update`) can arrive interleaved with a pending
//! response; [`OmpRpcClient::read_response`] queues anything that is not
//! the awaited response into `pending_events` rather than discarding it,
//! and a malformed (non-JSON) stdout line is always skipped, never fatal.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Map, Value};

use batman_runtime::adapter::AdapterError;
use batman_runtime::supervisor::ManagedProcess;

/// A parsed `{"type":"response",...}` frame.
#[derive(Debug, Clone)]
pub struct RpcResponse {
    pub id: String,
    pub command: String,
    pub success: bool,
    pub data: Value,
    pub error: Option<String>,
}

/// The ready-frame handshake + command/response client over one
/// [`ManagedProcess`]'s stdio.
pub struct OmpRpcClient {
    process: ManagedProcess,
    next_id: AtomicU64,
    /// Unsolicited frames observed while waiting for a specific
    /// correlated response (or before the ready handshake completed),
    /// preserved in arrival order rather than discarded.
    pending_events: VecDeque<Value>,
}

impl OmpRpcClient {
    #[must_use]
    pub fn new(process: ManagedProcess) -> Self {
        Self {
            process,
            next_id: AtomicU64::new(1),
            pending_events: VecDeque::new(),
        }
    }

    fn fresh_id(&self) -> String {
        self.next_id.fetch_add(1, Ordering::SeqCst).to_string()
    }

    /// Reads stdout frames until the `{"type":"ready"}` handshake frame
    /// arrives. A malformed (non-UTF8 or non-JSON) line is skipped, never
    /// fatal; any other well-formed frame seen before `ready` is queued
    /// into `pending_events` rather than discarded (the real binary can
    /// emit `extension_ui_request` immediately after `ready`, so this
    /// adapter treats "something before ready" as merely unusual, not a
    /// protocol violation).
    ///
    /// # Errors
    /// Returns [`AdapterError::process`] if stdout closes before a ready
    /// frame is ever observed.
    pub async fn wait_for_ready(&mut self) -> Result<Value, AdapterError> {
        loop {
            let Some(bytes) = self.process.next_stdout_frame().await else {
                return Err(AdapterError::process(
                    "ompRpc",
                    "waitForReady",
                    "process stdout closed before a ready frame was observed",
                ));
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(text.trim()) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) == Some("ready") {
                return Ok(value);
            }
            self.pending_events.push_back(value);
        }
    }

    /// Writes one `{"type": command, "id": <fresh>, ...params}` request
    /// frame to stdin and returns the id it was sent with.
    ///
    /// # Errors
    /// Returns [`AdapterError::process`] if the write fails (e.g. stdin
    /// already closed).
    pub async fn send_command(
        &mut self,
        command: &str,
        params: Map<String, Value>,
    ) -> Result<String, AdapterError> {
        let id = self.fresh_id();
        let mut frame = params;
        frame.insert("type".to_string(), Value::String(command.to_string()));
        frame.insert("id".to_string(), Value::String(id.clone()));
        let mut line = Value::Object(frame).to_string();
        line.push('\n');
        self.process
            .write_stdin(line.as_bytes())
            .await
            .map_err(|e| {
                AdapterError::process(
                    "ompRpc",
                    command,
                    format!("failed to write {command} command: {e}"),
                )
            })?;
        Ok(id)
    }

    /// Reads frames until the `{"type":"response","id":<id>,...}` frame
    /// correlated to `id` is found, queuing every other well-formed frame
    /// seen along the way (drainable via [`Self::drain_events`]).
    /// Malformed lines are skipped, never fatal.
    ///
    /// # Errors
    /// Returns [`AdapterError::process`] if stdout closes before the
    /// correlated response is observed.
    pub async fn read_response(&mut self, id: &str) -> Result<RpcResponse, AdapterError> {
        if let Some(pos) = self
            .pending_events
            .iter()
            .position(|value| is_response_for(value, id))
        {
            let value = self
                .pending_events
                .remove(pos)
                .expect("position was just found in the same deque");
            return Ok(parse_response(&value));
        }
        loop {
            let Some(bytes) = self.process.next_stdout_frame().await else {
                return Err(AdapterError::process(
                    "ompRpc",
                    "readResponse",
                    format!("process stdout closed before a response for id {id} arrived"),
                ));
            };
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let Ok(value) = serde_json::from_str::<Value>(text.trim()) else {
                continue;
            };
            if is_response_for(&value, id) {
                return Ok(parse_response(&value));
            }
            self.pending_events.push_back(value);
        }
    }

    /// Reads and returns exactly the next well-formed frame, whatever it
    /// is (a response or an unsolicited event), pulling first from
    /// anything already queued. Malformed lines are skipped, never fatal.
    /// Returns `None` once stdout has closed.
    pub async fn next_frame(&mut self) -> Option<Value> {
        if let Some(value) = self.pending_events.pop_front() {
            return Some(value);
        }
        loop {
            let bytes = self.process.next_stdout_frame().await?;
            let Ok(text) = std::str::from_utf8(&bytes) else {
                continue;
            };
            if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
                return Some(value);
            }
        }
    }

    /// Drains every event queued while waiting for a correlated response,
    /// in arrival order.
    pub fn drain_events(&mut self) -> Vec<Value> {
        self.pending_events.drain(..).collect()
    }

    /// The underlying supervised process, for termination/signal control.
    pub fn process_mut(&mut self) -> &mut ManagedProcess {
        &mut self.process
    }
}

fn is_response_for(value: &Value, id: &str) -> bool {
    value.get("type").and_then(Value::as_str) == Some("response")
        && value.get("id").and_then(Value::as_str) == Some(id)
}

fn parse_response(value: &Value) -> RpcResponse {
    RpcResponse {
        id: value
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        command: value
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        success: value
            .get("success")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        data: value.get("data").cloned().unwrap_or(Value::Null),
        error: value
            .get("error")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

// --------------------------------------------------------- command builders

/// `case "prompt": { const H = await kI1(A, E.message, E.streamingBehavior) }`.
#[must_use]
pub fn prompt_command(message: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("message".to_string(), Value::String(message.to_string()));
    params
}

/// `case "steer": { await A.steer(E.message, E.images) }`.
#[must_use]
pub fn steer_command(message: &str) -> Map<String, Value> {
    prompt_command(message)
}

/// `case "follow_up": { await A.followUp(E.message, E.images) }`.
#[must_use]
pub fn follow_up_command(message: &str) -> Map<String, Value> {
    prompt_command(message)
}

/// `case "abort": { await A.abort({ reason: Yj }) }` -- no caller-supplied
/// parameters.
#[must_use]
pub fn abort_command() -> Map<String, Value> {
    Map::new()
}

/// `case "get_state": { ... }` -- no parameters.
#[must_use]
pub fn get_state_command() -> Map<String, Value> {
    Map::new()
}

/// `case "get_messages": { ... }` -- no parameters.
#[must_use]
pub fn get_messages_command() -> Map<String, Value> {
    Map::new()
}

/// `case "get_session_stats"`-equivalent aggregate usage query -- no
/// parameters (the real dispatcher's exact case label for this was not
/// captured verbatim; the request/response shape was, via a direct probe:
/// `{"type":"get_session_stats","id":"1"}` -> `data.tokens.{input,output,
/// ...}`, `data.cost`).
#[must_use]
pub fn get_session_stats_command() -> Map<String, Value> {
    Map::new()
}

/// `case "get_subagents": { ... return u(m, "get_subagents", { subagents:
/// z.getSubagents() }) }` -- no parameters.
#[must_use]
pub fn get_subagents_command() -> Map<String, Value> {
    Map::new()
}

/// `switch_session": { const w = !await A.switchSession(j.sessionPath) }`.
#[must_use]
pub fn switch_session_command(session_path: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert(
        "sessionPath".to_string(),
        Value::String(session_path.to_string()),
    );
    params
}

/// `case "set_model": { ... H.find((T) => T.provider === E.provider &&
/// T.id === E.modelId) ... }`.
#[must_use]
pub fn set_model_command(provider: &str, model_id: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("provider".to_string(), Value::String(provider.to_string()));
    params.insert("modelId".to_string(), Value::String(model_id.to_string()));
    params
}

/// `case "set_subagent_subscription": { ... if (!uNw(E.level)) ...
/// z.setSubscriptionLevel(E.level) }`. The exact enum values `uNw`
/// validates against were not recoverable from the installed binary's
/// stripped symbol names; `"full"` is this adapter's own choice for
/// "subscribe to everything", consistent with the field name and the
/// dispatcher's boolean accept/reject shape.
#[must_use]
pub fn set_subagent_subscription_command(level: &str) -> Map<String, Value> {
    let mut params = Map::new();
    params.insert("level".to_string(), Value::String(level.to_string()));
    params
}

/// The ordered list of `(command, params)` pairs [`super::OmpRpcAdapter`]
/// sends to start one run: `set_subagent_subscription` first, but only
/// when `subscribe_subagents` is true (nested visibility was requested),
/// then `prompt` -- proving subagent subscription is established before
/// work begins without depending on a live process.
#[must_use]
pub fn build_startup_commands(
    subscribe_subagents: bool,
    prompt: &str,
) -> Vec<(String, Map<String, Value>)> {
    let mut commands = Vec::new();
    if subscribe_subagents {
        commands.push((
            "set_subagent_subscription".to_string(),
            set_subagent_subscription_command("full"),
        ));
    }
    commands.push(("prompt".to_string(), prompt_command(prompt)));
    commands
}
