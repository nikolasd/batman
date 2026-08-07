//! Deterministic scrubber for captured vendor frames.
//!
//! Rewrites every nondeterministic value in a captured frame -- session
//! identifiers, absolute paths, timestamps, costs, secrets -- into stable
//! placeholders so re-capturing an unchanged CLI produces byte-identical
//! files, while preserving the correlation ids that conformance suites
//! assert on.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// Rewrites captured vendor frames into committed-fixture form:
/// secrets removed, then every nondeterministic value replaced by a
/// stable placeholder, so re-capturing an unchanged CLI yields a
/// byte-identical file.
pub struct Scrubber {
    redactor: crate::security::redaction::Redactor,
    cwd: String,
    /// Maps each unique session-id input to a distinct stable UUID so
    /// multiple sessions within one capture each get a consistent
    /// placeholder.
    session_ids: HashMap<String, String>,
    /// Monotonically increasing counter for generating stable `uuid`
    /// values.
    uuid_seq: u32,
}

impl Scrubber {
    /// `cwd` is the absolute working directory the capture ran in; it is
    /// rewritten to `/workspace/batman` wherever it appears.
    pub fn new(cwd: String) -> Self {
        Self {
            redactor: crate::security::redaction::Redactor::default(),
            cwd,
            session_ids: HashMap::new(),
            uuid_seq: 1,
        }
    }

    /// Scrubs one captured frame. Returns `None` when the line is not
    /// JSON at all (vendor banner/log noise), which the caller drops.
    pub fn scrub_line(&mut self, line: &[u8]) -> Option<String> {
        let val = serde_json::from_slice(line).ok()?;
        let scrubbed = self.walk(Value::Object(Map::new()), val);
        Some(serde_json::to_string(&scrubbed).expect("scrubbed value is serializable"))
    }

    /// Recursively walks `val`, rewriting nondeterministic values.
    /// `parent_key` is the key this value is stored under (used for
    /// context-aware rewriting of ambiguous keys like `id`).
    fn walk(&mut self, parent_key: Value, val: Value) -> Value {
        match val {
            Value::String(s) => Value::String(self.rewrite_string(parent_key, &s)),
            Value::Number(n) => self.rewrite_number(parent_key, n),
            Value::Array(arr) => {
                Value::Array(arr.into_iter().map(|v| self.walk(Value::Null, v)).collect())
            }
            Value::Object(obj) => {
                Value::Object(
                    obj.into_iter()
                        .map(|(k, v)| {
                            let pk = Value::String(k.clone());
                            (k, self.walk(pk, v))
                        })
                        .collect(),
                )
            }
            v => v,
        }
    }

    fn rewrite_string(&mut self, parent_key: Value, s: &str) -> String {
        // Check for session-related keys first (parent-aware)
        if let Some(k) = parent_key.as_str() {
            if Self::is_session_key(k) {
                return self.stable_session_id(s);
            }
            if Self::is_timestamp_key(k) {
                return "2026-01-01T00:00:00Z".to_string();
            }
            if Self::is_cost_key(k) {
                return "0.0142".to_string();
            }
            // `uuid` key always gets a stable placeholder
            if k == "uuid" {
                return self.stable_uuid(s);
            }
            // `id` under `thread` or `turn` is a session identity;
            // everywhere else it is a correlation id to preserve.
            if k == "id" && Self::is_session_context(&parent_key) {
                return self.stable_session_id(s);
            }
        }

        // Check for RFC 3339 timestamp anywhere in the string value
        if Self::looks_like_rfc3339(s) {
            return "2026-01-01T00:00:00Z".to_string();
        }

        // Rewrite cwd paths
        let out = s.replace(&self.cwd, "/workspace/batman");
        // Always apply secret redaction as the final pass, even when
        // cwd was rewritten, so a path string that also contains a
        // secret-shaped value gets sanitized.
        self.redactor.redact_text(&out)
    }

    fn rewrite_number(&self, parent_key: Value, n: serde_json::Number) -> Value {
        if let Some(k) = parent_key.as_str() {
            if Self::is_timestamp_key(k) {
                // Numeric timestamps (ms since epoch) → stable ms value
                // for 2026-01-01T00:00:00Z
                // If already at the stable value, return unchanged.
                if n.as_u64() == Some(1_735_689_600_000) {
                    return Value::Number(n);
                }
                return Value::Number(serde_json::Number::from(1_735_689_600_000u64));
            }
            if Self::is_cost_key(k) {
                // Cost is a float; if already at the stable value,
                // return unchanged so re-scrubbing is a no-op.
                if n.as_f64() == Some(0.0142) {
                    return Value::Number(n);
                }
                return Value::Number(
                    serde_json::Number::from_f64(0.0142).expect("0.0142 is a valid JSON number"),
                );
            }
            if k == "duration_ms" || k == "duration_api_ms" {
                let target = if k == "duration_ms" { 4210u64 } else { 3980u64 };
                if n.as_u64() == Some(target) {
                    return Value::Number(n);
                }
                return Value::Number(serde_json::Number::from(target));
            }
        }
        Value::Number(n)
    }

    /// Key names whose values are session/thread identities to rewrite.
    fn is_session_key(k: &str) -> bool {
        matches!(
            k,
            "session_id"
                | "sessionId"
                | "threadId"
                | "conversationId"
                | "thread" // nested: thread.id
                | "turn" // nested: turn.id
        )
    }

    /// Whether the parent key signals a session-identity context where
    /// a child `id` field should be rewritten (rather than preserved as
    /// a correlation id).
    fn is_session_context(pk: &Value) -> bool {
        pk.as_str()
            .map(|k| matches!(k, "thread" | "turn"))
            .unwrap_or(false)
    }

    /// Key names whose values are timestamps.
    fn is_timestamp_key(k: &str) -> bool {
        matches!(
            k,
            "startedAtMs"
                | "completedAtMs"
                | "createdAt"
                | "updatedAt"
                | "timestamp"
                | "time"
                | "createdAtMs"
                | "startedAt"
                | "finishedAt"
        )
    }

    /// Key names whose values are cost figures.
    fn is_cost_key(k: &str) -> bool {
        matches!(k, "total_cost_usd" | "costUSD" | "cost")
    }

    fn stable_session_id(&mut self, input: &str) -> String {
        // If the value is already in our stable format, return unchanged
        // so re-scrubbing a committed fixture is a no-op.
        if input.starts_with("11111111-1111-4111-8111-") {
            return input.to_string();
        }
        if let Some(existing) = self.session_ids.get(input) {
            return existing.clone();
        }
        let seq = self.session_ids.len() + 1;
        let value = format!("11111111-1111-4111-8111-{seq:012}");
        self.session_ids.insert(input.to_string(), value.clone());
        value
    }

    fn stable_uuid(&mut self, input: &str) -> String {
        // If already in stable format, return unchanged.
        if input.starts_with("a0000000-0000-4000-8000-") {
            return input.to_string();
        }
        let seq = self.uuid_seq;
        self.uuid_seq += 1;
        format!("a0000000-0000-4000-8000-{seq:012}")
    }

    fn looks_like_rfc3339(s: &str) -> bool {
        // Heuristic: contains 'T' and ends with 'Z' or has an offset
        // like +00:00. Real timestamps in fixtures are well-formed.
        s.contains('T')
            && (s.ends_with('Z')
                || (s.len() > 20 && (s.ends_with("+00:00") || s.ends_with("-00:00"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scrubbing an already-scrubbed fixture is a no-op.
    ///
    /// The committed `initialize.jsonl` is already in canonical form, so
    /// running it through the scrubber with a matching `cwd` must produce
    /// byte-identical output. This is the single strongest check that
    /// capture will not churn committed fixtures.
    #[test]
    fn scrubbing_scrubbed_fixture_is_identity() {
        let fixture_path =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/adapters/claude/initialize.jsonl");
        let content =
            std::fs::read_to_string(&fixture_path).expect("fixture must be readable");
        let mut scrubber = Scrubber::new("/workspace/batman".into());

        for line in content.lines() {
            if line.is_empty() {
                continue;
            }
            let scrubbed = scrubber
                .scrub_line(line.as_bytes())
                .expect("fixture line must parse as JSON");
            assert_eq!(
                scrubbed, line,
                "scrubbing already-scrubbed line must be identity: {line}"
            );
        }
    }

    /// Correlation ids are preserved, session ids are rewritten.
    #[test]
    fn preserves_correlation_ids() {
        let mut scrubber = Scrubber::new("/tmp/capture-123".into());
        let input = r#"{"session_id":"real-session-uuid","message":{"id":"msg_01ABC","content":[{"type":"tool_use","id":"toolu_01READ","name":"Read"}]}}"#;
        let scrubbed = scrubber
            .scrub_line(input.as_bytes())
            .expect("must parse");
        let obj: Value = serde_json::from_str(&scrubbed).expect("scrubbed must be valid JSON");
        let obj = obj.as_object().expect("must be object");

        // Session id was rewritten (first unique session gets seq=1)
        assert_eq!(
            obj.get("session_id").unwrap().as_str().unwrap(),
            "11111111-1111-4111-8111-000000000001"
        );

        // Correlation ids preserved
        let msg = obj.get("message").unwrap().as_object().unwrap();
        assert_eq!(msg.get("id").unwrap().as_str().unwrap(), "msg_01ABC");
        let content = msg.get("content").unwrap().as_array().unwrap();
        let tool = content[0].as_object().unwrap();
        assert_eq!(tool.get("id").unwrap().as_str().unwrap(), "toolu_01READ");
    }

    /// Cwd paths are rewritten.
    #[test]
    fn rewrites_cwd_paths() {
        let mut scrubber = Scrubber::new("/tmp/capture-123".into());
        let input = r#"{"cwd":"/tmp/capture-123","file_path":"/tmp/capture-123/config.toml"}"#;
        let scrubbed = scrubber
            .scrub_line(input.as_bytes())
            .expect("must parse");
        let obj: Value = serde_json::from_str(&scrubbed).expect("scrubbed must be valid JSON");
        let obj = obj.as_object().expect("must be object");
        assert_eq!(obj.get("cwd").unwrap().as_str().unwrap(), "/workspace/batman");
        assert_eq!(
            obj.get("file_path").unwrap().as_str().unwrap(),
            "/workspace/batman/config.toml"
        );
    }

    /// Numeric timestamps are rewritten.
    #[test]
    fn rewrites_numeric_timestamps() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let input = r#"{"startedAtMs":1732400000000,"completedAtMs":1732400001000}"#;
        let scrubbed = scrubber
            .scrub_line(input.as_bytes())
            .expect("must parse");
        let obj: Value = serde_json::from_str(&scrubbed).expect("scrubbed must be valid JSON");
        let obj = obj.as_object().expect("must be object");
        assert_eq!(
            obj.get("startedAtMs").unwrap().as_u64().unwrap(),
            1_735_689_600_000
        );
        assert_eq!(
            obj.get("completedAtMs").unwrap().as_u64().unwrap(),
            1_735_689_600_000
        );
    }

    /// Non-JSON lines are dropped.
    #[test]
    fn drops_non_json_lines() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        assert!(scrubber.scrub_line(b"this-is-not-json").is_none());
        assert!(scrubber.scrub_line(b"").is_none());
    }
}
