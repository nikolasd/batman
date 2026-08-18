//! Deterministic scrubber for captured vendor frames.
//!
//! Rewrites every nondeterministic value in a captured frame -- session
//! identifiers, absolute paths, timestamps, costs, secrets -- into stable
//! placeholders so re-capturing an unchanged CLI produces byte-identical
//! files, while preserving the correlation ids that conformance suites
//! assert on.

use std::collections::HashMap;

use serde_json::Value;

/// Rewrites captured vendor frames into committed-fixture form:
/// secrets removed, then every nondeterministic value replaced by a
/// stable placeholder, so re-capturing an unchanged CLI yields a
/// byte-identical file.
pub struct Scrubber {
    redactor: crate::security::redaction::Redactor,
    cwd: String,
    /// Maps each unique session-id input to a distinct canonical UUID so
    /// repeated session identities remain correlated within one capture.
    session_ids: HashMap<String, String>,
    /// Maps each unique raw `uuid` input to a canonical UUID so repeated
    /// values remain correlated within one capture.
    uuid_ids: HashMap<String, String>,
}

impl Scrubber {
    /// `cwd` is the absolute working directory the capture ran in; it is
    /// rewritten to `/workspace/batman` wherever it appears.
    pub fn new(cwd: String) -> Self {
        Self {
            redactor: crate::security::redaction::Redactor::default(),
            cwd,
            session_ids: HashMap::new(),
            uuid_ids: HashMap::new(),
        }
    }

    /// Scrubs one captured frame. Empty lines and invalid UTF-8 input are
    /// defensively dropped; valid UTF-8 non-JSON text is retained after path
    /// and secret rewriting so the capture path can apply its reader policy.
    pub fn scrub_line(&mut self, line: &[u8]) -> Option<String> {
        match serde_json::from_slice(line) {
            Ok(value) => {
                let scrubbed = self.walk(None, None, value);
                Some(serde_json::to_string(&scrubbed).expect("scrubbed value is serializable"))
            }
            Err(_) => {
                if line.is_empty() {
                    return None;
                }
                let text = std::str::from_utf8(line).ok()?;
                Some(self.rewrite_unstructured_line(text))
            }
        }
    }

    /// Recursively walks `val`, rewriting nondeterministic values. Both key
    /// contexts distinguish correlation `id` fields from `thread.id` and
    /// `turn.id` session identities.
    fn walk(
        &mut self,
        parent_key: Option<&str>,
        grandparent_key: Option<&str>,
        val: Value,
    ) -> Value {
        match val {
            Value::String(s) => Value::String(self.rewrite_string(parent_key, grandparent_key, &s)),
            Value::Number(n) => self.rewrite_number(parent_key, n),
            Value::Array(arr) => Value::Array(
                arr.into_iter()
                    .map(|value| self.walk(parent_key, grandparent_key, value))
                    .collect(),
            ),
            Value::Object(obj) => Value::Object(
                obj.into_iter()
                    .map(|(key, value)| {
                        let rewritten = self.walk(Some(&key), parent_key, value);
                        (key, rewritten)
                    })
                    .collect(),
            ),
            value => value,
        }
    }

    fn rewrite_string(
        &mut self,
        parent_key: Option<&str>,
        grandparent_key: Option<&str>,
        s: &str,
    ) -> String {
        if let Some(key) = parent_key {
            if Self::is_session_key(key)
                || (key == "id" && Self::is_session_context(grandparent_key))
            {
                return self.stable_session_id(s);
            }
            if key == "sessionFile" {
                if let Some(rewritten) = self.rewrite_session_file(s) {
                    return rewritten;
                }
            }
            if Self::is_timestamp_key(key) {
                return "2026-01-01T00:00:00Z".to_string();
            }
            if Self::is_cost_key(key) {
                return "0.0142".to_string();
            }
            if key == "uuid" {
                return self.stable_uuid(s);
            }
        }

        // Check for RFC 3339 timestamp anywhere in a structured string value.
        if Self::looks_like_rfc3339(s) {
            return "2026-01-01T00:00:00Z".to_string();
        }

        self.rewrite_unstructured_line(s)
    }

    /// Rewrites a raw non-JSON line without applying structured-value rules.
    fn rewrite_unstructured_line(&self, text: &str) -> String {
        let cwd_rewritten = text.replace(&self.cwd, "/workspace/batman");
        self.redactor.redact_text(&cwd_rewritten)
    }

    /// Replaces the session identity encoded in a `.omp/sessions/*.jsonl` path.
    fn rewrite_session_file(&mut self, value: &str) -> Option<String> {
        const PREFIX: &str = ".omp/sessions/";
        const SUFFIX: &str = ".jsonl";

        let prefix_start = value.find(PREFIX)?;
        let id_start = prefix_start + PREFIX.len();
        let id_end = id_start + value[id_start..].find(SUFFIX)?;
        if id_start == id_end {
            return None;
        }

        let stable_id = self.stable_session_id(&value[id_start..id_end]);
        Some(format!(
            "{}{}{}",
            &value[..id_start],
            stable_id,
            &value[id_end..]
        ))
    }

    fn rewrite_number(&self, parent_key: Option<&str>, n: serde_json::Number) -> Value {
        if let Some(k) = parent_key {
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

    /// Key names whose values are session identities to rewrite.
    fn is_session_key(key: &str) -> bool {
        matches!(
            key,
            "session_id" | "sessionId" | "threadId" | "turnId" | "conversationId"
        )
    }

    /// Whether the containing object identifies a session, rather than a
    /// correlation id.
    fn is_session_context(grandparent_key: Option<&str>) -> bool {
        matches!(grandparent_key, Some("thread" | "turn"))
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

    /// Returns the canonical session ID for this capture's encounter order.
    fn stable_session_id(&mut self, input: &str) -> String {
        if let Some(existing) = self.session_ids.get(input) {
            return existing.clone();
        }

        let seq = self.session_ids.len() + 1;
        let value = format!("11111111-1111-4111-8111-{seq:012}");
        self.session_ids.insert(input.to_string(), value.clone());
        value
    }

    /// Returns the canonical UUID for this capture's encounter order.
    fn stable_uuid(&mut self, input: &str) -> String {
        if let Some(existing) = self.uuid_ids.get(input) {
            return existing.clone();
        }

        let seq = self.uuid_ids.len() + 1;
        let value = format!("a0000000-0000-4000-8000-{seq:012}");
        self.uuid_ids.insert(input.to_string(), value.clone());
        value
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

    /// Re-scrubbing the canonical initialization fixture is a fixed point.
    ///
    /// The fixture's current encounter order produces byte-identical output
    /// with a matching `cwd`. This only proves re-scrub fixed-point behavior;
    /// capture fixture migration establishes fresh-capture no-churn behavior.
    #[test]
    fn scrubbing_scrubbed_fixture_is_identity() {
        let fixture_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/adapters/claude/initialize.jsonl");
        let content = std::fs::read_to_string(&fixture_path).expect("fixture must be readable");
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
        let scrubbed = scrubber.scrub_line(input.as_bytes()).expect("must parse");
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
        let scrubbed = scrubber.scrub_line(input.as_bytes()).expect("must parse");
        let obj: Value = serde_json::from_str(&scrubbed).expect("scrubbed must be valid JSON");
        let obj = obj.as_object().expect("must be object");
        assert_eq!(
            obj.get("cwd").unwrap().as_str().unwrap(),
            "/workspace/batman"
        );
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
        let scrubbed = scrubber.scrub_line(input.as_bytes()).expect("must parse");
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

    #[test]
    fn drops_non_json_lines() {
        let mut scrubber = Scrubber::new("/tmp/capture-123".into());

        assert_eq!(scrubber.scrub_line(b"this-is-not-json"), None);
        assert_eq!(scrubber.scrub_line(b""), None);
        assert_eq!(scrubber.scrub_line(&[0xff]), None);
    }

    #[test]
    fn nested_thread_id_and_thread_id_share_one_stable_session_identity() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let scrubbed = scrubber
            .scrub_line(br#"{"thread":{"id":"thread-actual"},"threadId":"thread-actual"}"#)
            .expect("frame must be retained");
        let value: Value = serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");
        let expected = "11111111-1111-4111-8111-000000000001";

        assert_eq!(value["thread"]["id"], expected);
        assert_eq!(value["threadId"], expected);
    }

    #[test]
    fn nested_turn_id_and_turn_id_share_one_stable_session_identity() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let scrubbed = scrubber
            .scrub_line(br#"{"turn":{"id":"turn-actual"},"turnId":"turn-actual"}"#)
            .expect("frame must be retained");
        let value: Value = serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");
        let expected = "11111111-1111-4111-8111-000000000001";

        assert_eq!(value["turn"]["id"], expected);
        assert_eq!(value["turnId"], expected);
    }

    #[test]
    fn session_file_and_session_id_share_one_stable_identity_in_any_key_order() {
        let session_id = "019f9652-7aac-7000-a8e1-db0d90064c58";
        let expected = "11111111-1111-4111-8111-000000000001";

        for input in [
            format!(
                r#"{{"sessionFile":".omp/sessions/{session_id}.jsonl","sessionId":"{session_id}"}}"#
            ),
            format!(
                r#"{{"sessionId":"{session_id}","sessionFile":".omp/sessions/{session_id}.jsonl"}}"#
            ),
        ] {
            let mut scrubber = Scrubber::new("/workspace/batman".into());
            let scrubbed = scrubber
                .scrub_line(input.as_bytes())
                .expect("frame must be retained");
            let value: Value =
                serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");

            assert_eq!(value["sessionId"], expected);
            assert_eq!(
                value["sessionFile"],
                format!(".omp/sessions/{expected}.jsonl")
            );
        }
    }

    #[test]
    fn prefixed_ids_are_renumbered_by_encounter_order() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let session_ids: Vec<String> = [
            r#"{"session_id":"11111111-1111-4111-8111-111111111111"}"#,
            r#"{"session_id":"11111111-1111-4111-8111-111111111111"}"#,
            r#"{"session_id":"11111111-1111-4111-8111-222222222222"}"#,
        ]
        .iter()
        .map(|frame| {
            let scrubbed = scrubber
                .scrub_line(frame.as_bytes())
                .expect("frame must be retained");
            let value: Value =
                serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");
            value["session_id"]
                .as_str()
                .expect("session id must remain a string")
                .to_owned()
        })
        .collect();
        assert_eq!(
            session_ids,
            vec![
                "11111111-1111-4111-8111-000000000001".to_string(),
                "11111111-1111-4111-8111-000000000001".to_string(),
                "11111111-1111-4111-8111-000000000002".to_string(),
            ]
        );

        let uuids: Vec<String> = [
            r#"{"uuid":"a0000000-0000-4000-8000-999999999999"}"#,
            r#"{"uuid":"a0000000-0000-4000-8000-999999999999"}"#,
            r#"{"uuid":"a0000000-0000-4000-8000-888888888888"}"#,
        ]
        .iter()
        .map(|frame| {
            let scrubbed = scrubber
                .scrub_line(frame.as_bytes())
                .expect("frame must be retained");
            let value: Value =
                serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");
            value["uuid"]
                .as_str()
                .expect("UUID must remain a string")
                .to_owned()
        })
        .collect();
        assert_eq!(
            uuids,
            vec![
                "a0000000-0000-4000-8000-000000000001".to_string(),
                "a0000000-0000-4000-8000-000000000001".to_string(),
                "a0000000-0000-4000-8000-000000000002".to_string(),
            ]
        );
    }

    #[test]
    fn correlation_ids_are_renumbered_by_family_and_encounter_order() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let first = scrubber
            .scrub_line(
                br#"{"message":{"id":"msg-first"},"messageId":"msg-first","tool_use":{"id":"toolu-first"},"parent_tool_use_id":"toolu-first","tool_use_id":"toolu-first","callId":"toolu-first","toolCallId":"toolu-first","hook_id":"hook-first","item":{"id":"item-first"},"itemId":"item-first","agentId":"agent-first"}"#,
            )
            .expect("frame must be retained");
        let first: Value = serde_json::from_str(&first).expect("scrubbed frame must be JSON");
        let second = scrubber
            .scrub_line(
                br#"{"message":{"id":"msg-second"},"tool_use":{"id":"toolu-second"},"hook_id":"hook-second","item":{"id":"item-second"},"agentId":"agent-second"}"#,
            )
            .expect("frame must be retained");
        let second: Value = serde_json::from_str(&second).expect("scrubbed frame must be JSON");

        assert_eq!(first["message"]["id"], "msg-000000000001");
        assert_eq!(first["messageId"], "msg-000000000001");
        for key in [
            "id",
            "parent_tool_use_id",
            "tool_use_id",
            "callId",
            "toolCallId",
        ] {
            let value = if key == "id" {
                &first["tool_use"][key]
            } else {
                &first[key]
            };
            assert_eq!(value, "tool-000000000001", "{key} must share the tool family");
        }
        assert_eq!(first["hook_id"], "hook-000000000001");
        assert_eq!(first["item"]["id"], "item-000000000001");
        assert_eq!(first["itemId"], "item-000000000001");
        assert_eq!(first["agentId"], "agent-000000000001");

        assert_eq!(second["message"]["id"], "msg-000000000002");
        assert_eq!(second["tool_use"]["id"], "tool-000000000002");
        assert_eq!(second["hook_id"], "hook-000000000002");
        assert_eq!(second["item"]["id"], "item-000000000002");
        assert_eq!(second["agentId"], "agent-000000000002");
    }

    #[test]
    fn absolute_session_file_rewrites_its_id_path_and_secret() {
        let mut scrubber = Scrubber::new("/tmp/capture-123".into());
        let scrubbed = scrubber
            .scrub_line(
                br#"{"sessionId":"session-actual","sessionFile":"/tmp/capture-123/.omp/sessions/session-actual.jsonl","token":"sk-ABCDEFGHIJKLMNOPQRSTUVWX"}"#,
            )
            .expect("frame must be retained");
        let value: Value = serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");
        let expected = "11111111-1111-4111-8111-000000000001";

        assert_eq!(value["sessionId"], expected);
        assert_eq!(
            value["sessionFile"],
            format!("/workspace/batman/.omp/sessions/{expected}.jsonl")
        );
        assert_eq!(value["token"], "[REDACTED:api_key]");
    }

    #[test]
    fn normalizes_command_paths_without_misclassifying_prose_or_nested_turns() {
        let mut scrubber = Scrubber::new("/workspace/batman".into());
        let scrubbed = scrubber
            .scrub_line(
                br#"{"command":"/opt/homebrew/bin/copilot","prose":"meetingTendsAtZ","thread":{"turns":[{"id":"turn-actual"}]}}"#,
            )
            .expect("frame must be retained");
        let value: Value = serde_json::from_str(&scrubbed).expect("scrubbed frame must be JSON");

        assert_eq!(value["command"], "/usr/local/bin/copilot");
        assert_eq!(value["prose"], "meetingTendsAtZ");
        assert_eq!(
            value["thread"]["turns"][0]["id"],
            "11111111-1111-4111-8111-000000000001"
        );
    }
}
