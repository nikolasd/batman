//! Normalizes a raw ACP v1 `session/update` payload (the `update` field of
//! a `SessionNotification`) into zero or more [`AdapterEventPayload`]s.
//!
//! **`agent_thought_chunk` is dropped here, before anything ever reaches
//! an `AdapterEvent`** -- this is the adapter's *primary* Thinking-content
//! filter (the sink's own drop-to-`None` on a non-`Visible` `Classified`
//! value is only a defensive backstop, per `event_sink.rs`'s module doc).
//! An unrecognized `sessionUpdate` discriminator (e.g. `plan`,
//! `available_commands_update`, `current_mode_update` -- real ACP v1
//! variants this adapter does not yet map to a canonical `AdapterEvent`)
//! normalizes to no events rather than a guessed shape.

use batman_protocol::{Classified, ContentClass};
use serde_json::Value;

use batman_runtime::adapter::AdapterEventPayload;

/// Normalizes one ACP `session/update` `update` object into the
/// canonical `AdapterEvent` payloads it represents.
#[must_use]
pub fn copilot_normalize_session_update(update: &Value) -> Vec<AdapterEventPayload> {
    match update.get("sessionUpdate").and_then(Value::as_str) {
        Some("agent_thought_chunk") => Vec::new(),
        Some(kind @ ("user_message_chunk" | "agent_message_chunk")) => {
            let role = if kind == "user_message_chunk" {
                "user"
            } else {
                "assistant"
            };
            let Some(text) = update.get("content").and_then(content_block_text) else {
                return Vec::new();
            };
            vec![AdapterEventPayload::MessageChunk {
                role: role.to_string(),
                text: Classified {
                    class: ContentClass::Visible,
                    value: text,
                },
            }]
        }
        Some("tool_call") => {
            let Some(tool_call_id) = update.get("toolCallId").and_then(Value::as_str) else {
                return Vec::new();
            };
            let name = update
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(tool_call_id)
                .to_string();
            vec![AdapterEventPayload::ToolStarted {
                tool_call_id: tool_call_id.to_string(),
                name,
            }]
        }
        Some("tool_call_update") => {
            let Some(tool_call_id) = update.get("toolCallId").and_then(Value::as_str) else {
                return Vec::new();
            };
            let name = update
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or(tool_call_id)
                .to_string();
            let detail = Classified {
                class: ContentClass::Visible,
                value: tool_call_content_text(update.get("content").and_then(Value::as_array)),
            };
            match update.get("status").and_then(Value::as_str) {
                Some("completed") => vec![AdapterEventPayload::ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    name,
                    ok: true,
                    detail,
                }],
                Some("failed") => vec![AdapterEventPayload::ToolResult {
                    tool_call_id: tool_call_id.to_string(),
                    name,
                    ok: false,
                    detail,
                }],
                _ => vec![AdapterEventPayload::ToolProgress {
                    tool_call_id: tool_call_id.to_string(),
                    name,
                    detail,
                }],
            }
        }
        _ => Vec::new(),
    }
}

/// Extracts display text from an ACP `ContentBlock`. Non-text blocks
/// (image/audio/resource/resource_link) never leak their raw payload --
/// only a short, static placeholder naming the block's `type`.
fn content_block_text(block: &Value) -> Option<String> {
    match block.get("type").and_then(Value::as_str) {
        Some("text") => block.get("text").and_then(Value::as_str).map(str::to_owned),
        Some(other) => Some(format!("[{other} content]")),
        None => None,
    }
}

/// Joins an ACP `ToolCallContent[]` array into one display string: plain
/// content blocks render their text, diffs render only the affected path
/// (never the old/new file text, which may be arbitrarily large or
/// sensitive), and embedded terminals render a static placeholder.
fn tool_call_content_text(items: Option<&Vec<Value>>) -> String {
    let Some(items) = items else {
        return String::new();
    };
    items
        .iter()
        .filter_map(|item| match item.get("type").and_then(Value::as_str) {
            Some("content") => item.get("content").and_then(content_block_text),
            Some("diff") => {
                let path = item
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("<unknown path>");
                Some(format!("diff: {path}"))
            }
            Some("terminal") => Some("[terminal output]".to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
