//! The redaction boundary: the single place raw, classified vendor content
//! is turned into the only event type the durable journal can accept.
//!
//! Raw vendor frames -- which may carry [`ContentClass::Thinking`] or
//! [`ContentClass::Secret`] fragments -- exist only in bounded process
//! memory. [`Redactor::sanitize`] is the sole path from that raw
//! representation to [`PersistableEvent`]: it drops `Thinking` and `Secret`
//! fragments entirely, and rewrites built-in regex-pattern matches (e.g.
//! API-key-shaped tokens) found in `Visible` text with a `[REDACTED:<rule
//! id>]` marker. `PersistableEvent`'s fields are private and it has no
//! public constructor, so the only way to obtain one -- anywhere in this
//! crate or downstream -- is through [`Redactor::sanitize`].

use batman_protocol::{
    Classified, ContentClass, DiagnosticLevel, ProjectId, RunId, RuntimeEvent, Timestamp,
};
use regex::Regex;

/// A raw, potentially-classified runtime event, as produced by a worker or
/// vendor process before it crosses the redaction boundary.
///
/// This type must never be persisted or logged directly -- only the
/// [`PersistableEvent`] produced by [`Redactor::sanitize`] may reach the
/// database actor.
#[derive(Debug, Clone)]
pub struct RawRuntimeEvent {
    pub timestamp: Timestamp,
    pub project_id: ProjectId,
    pub run_id: Option<RunId>,
    pub kind: RawEventKind,
}

/// The raw, pre-redaction payload of a [`RawRuntimeEvent`].
///
/// Mirrors [`RuntimeEvent`]'s shape, except [`RawEventKind::Diagnostic`]
/// carries a list of classified text fragments rather than a plain
/// `message`: vendor frames often interleave visible narration with
/// thinking or secret content, and every fragment's classification must be
/// honored independently when redacting.
#[derive(Debug, Clone)]
pub enum RawEventKind {
    RuntimeStarted,
    RuntimeStopping,
    Diagnostic {
        level: DiagnosticLevel,
        code: String,
        fragments: Vec<Classified<String>>,
    },
}

/// A sanitized event, the only type the database actor's journal accepts.
///
/// Fields are private; there is no public constructor. The only way to
/// obtain one is [`Redactor::sanitize`].
#[derive(Debug, Clone)]
pub struct PersistableEvent {
    timestamp: Timestamp,
    project_id: ProjectId,
    run_id: Option<RunId>,
    event_json: String,
}

impl PersistableEvent {
    /// The event's timestamp.
    #[must_use]
    pub fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    /// The project the event belongs to.
    #[must_use]
    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    /// The run the event belongs to, if any.
    #[must_use]
    pub fn run_id(&self) -> Option<&RunId> {
        self.run_id.as_ref()
    }

    /// The sanitized event body, serialized as JSON text. Guaranteed to be
    /// the JSON serialization of a plain [`RuntimeEvent`] -- never a raw or
    /// classified value.
    #[must_use]
    pub fn event_json(&self) -> &str {
        &self.event_json
    }
}

/// Sanitized JSON text, the only type
/// [`crate::db::DatabaseHandle::record_operation_intent`] and
/// [`crate::db::DatabaseHandle::acknowledge_operation`] accept for their
/// intent/acknowledgement payloads.
///
/// There is no public constructor: the only way to obtain one, anywhere, is
/// [`Redactor::sanitize_json`], which deep-walks a `serde_json::Value` and
/// applies the same redaction rules used for events to every string (key
/// and value alike) before serializing it. This keeps unsanitized operation
/// payloads from reaching the durable `operations` table the same way
/// [`PersistableEvent`] keeps them out of `events`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SanitizedJson(String);

impl SanitizedJson {
    /// The sanitized JSON text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SanitizedJson {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One built-in, bounded regex rule applied to `Visible` text.
struct RedactionRule {
    id: &'static str,
    pattern: Regex,
}

impl RedactionRule {
    fn apply(&self, text: &str) -> String {
        self.pattern
            .replace_all(text, &format!("[REDACTED:{}]", self.id))
            .to_string()
    }
}

/// Compiles the built-in redaction rules once, then sanitizes raw events
/// into [`PersistableEvent`]s: the only crossing point of the redaction
/// boundary.
pub struct Redactor {
    rules: Vec<RedactionRule>,
    /// Org-configured redaction rules, compiled once at startup.
    org_rules: Vec<crate::security::rules::OrgRedactionRule>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new()
    }
}

impl Redactor {
    /// Compiles the built-in bounded regex rules. Intended to be called
    /// once at process startup and reused for every subsequent
    /// [`Redactor::sanitize`] call.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: vec![
                RedactionRule {
                    id: "api_key",
                    // API-key-shaped tokens, e.g. `sk-...` style secrets.
                    pattern: Regex::new(r"sk-[A-Za-z0-9]{16,}")
                        .expect("built-in api_key pattern is a valid, bounded regex"),
                },
                RedactionRule {
                    id: "bearer_token",
                    // Long bearer-ish tokens surfaced in free text.
                    pattern: Regex::new(r"Bearer\s+[A-Za-z0-9._-]{20,}")
                        .expect("built-in bearer_token pattern is a valid, bounded regex"),
                },
                RedactionRule {
                    id: "github_pat",
                    // GitHub personal access tokens (ghp_ prefix).
                    pattern: Regex::new(r"ghp_[A-Za-z0-9]{16,}")
                        .expect("built-in github_pat pattern is a valid, bounded regex"),
                },
                RedactionRule {
                    id: "aws_access_key",
                    // AWS access key IDs (AKIA prefix).
                    pattern: Regex::new(r"AKIA[0-9A-Z]{16}")
                        .expect("built-in aws_access_key pattern is a valid, bounded regex"),
                },
                RedactionRule {
                    id: "jwt",
                    // JSON Web Tokens (three base64url-encoded segments).
                    pattern: Regex::new(
                        r"[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}\.[A-Za-z0-9_-]{20,}",
                    )
                    .expect("built-in jwt pattern is a valid, bounded regex"),
                },
            ],
            org_rules: Vec::new(),
        }
    }

    /// Creates a [`Redactor`] with both built-in and org-configured rules.
    ///
    /// # Errors
    ///
    /// Returns an error if any org pattern is not a valid regex.
    pub fn with_org_rules(org_patterns: &[String]) -> Result<Self, String> {
        let mut redactor = Self::new();

        // Compile org patterns into OrgRedactionRule instances
        for (i, pattern) in org_patterns.iter().enumerate() {
            let rule =
                crate::security::rules::OrgRedactionRule::new(format!("org_pattern_{i}"), pattern)
                    .map_err(|e| format!("invalid org pattern at index {i}: {e}"))?;
            redactor.org_rules.push(rule);
        }

        Ok(redactor)
    }

    /// Sanitizes a raw event into the only type the durable journal
    /// accepts: `Thinking` and `Secret` fragments are dropped entirely
    /// (never even scanned), and built-in and org-defined pattern matches in
    /// `Visible` text are replaced with `[REDACTED:<rule-id>]`.
    #[must_use]
    pub fn sanitize(&self, raw: RawRuntimeEvent) -> PersistableEvent {
        let event = match raw.kind {
            RawEventKind::RuntimeStarted => RuntimeEvent::RuntimeStarted,
            RawEventKind::RuntimeStopping => RuntimeEvent::RuntimeStopping,
            RawEventKind::Diagnostic {
                level,
                code,
                fragments,
            } => {
                let message = fragments
                    .into_iter()
                    .filter(|fragment| fragment.class == ContentClass::Visible)
                    .map(|fragment| self.redact_visible_text(&fragment.value))
                    .collect::<Vec<_>>()
                    .join("\n");
                RuntimeEvent::Diagnostic {
                    level,
                    code,
                    message,
                }
            }
        };

        let event_json = serde_json::to_string(&event)
            .expect("a plain, already-sanitized RuntimeEvent always serializes");

        PersistableEvent {
            timestamp: raw.timestamp,
            project_id: raw.project_id,
            run_id: raw.run_id,
            event_json,
        }
    }

    /// Sanitizes an arbitrary `serde_json::Value` into [`SanitizedJson`]:
    /// the only path by which pre-serialized JSON (e.g. an operation's
    /// intent or acknowledgement payload) may cross the redaction boundary
    /// on its way to the durable `operations` table.
    ///
    /// Deep-walks the value, applying the same built-in and org-defined
    /// regex rules used for event text to every string found -- object keys
    /// and values alike, at any nesting depth -- replacing matches with
    /// `[REDACTED:<rule-id>]`. Unlike [`Redactor::sanitize`], nothing here
    /// is dropped based on a [`ContentClass`]: arbitrary JSON carries no
    /// classification, so every string is scanned. The result is
    /// serialized deterministically: `serde_json::Value`'s default `Map`
    /// (no `preserve_order` feature enabled in this workspace) orders
    /// object keys lexicographically, so the same logical JSON always
    /// produces the same bytes regardless of input key order.
    #[must_use]
    pub fn sanitize_json(&self, value: &serde_json::Value) -> SanitizedJson {
        let redacted = self.redact_json_value(value);
        let text = serde_json::to_string(&redacted)
            .expect("a serde_json::Value built from redaction always serializes");
        SanitizedJson(text)
    }

    /// Sanitizes a single classified text fragment for a wire-shape field
    /// (as opposed to a whole [`RawRuntimeEvent`]): `Thinking`/`Secret`
    /// fragments are dropped (returned as `None`), and `Visible` text has
    /// the same built-in and org-defined regex rules applied as
    /// [`Redactor::sanitize`]. Used by adapter event normalization
    /// (`crate::adapter::event_sink`), which carries free-text vendor
    /// output (message chunks, tool details, diagnostics) as
    /// `Classified<String>` fields that must cross this exact boundary
    /// before becoming part of a durable `RuntimeEvent`.
    #[must_use]
    pub fn sanitize_fragment(&self, fragment: &Classified<String>) -> Option<String> {
        match fragment.class {
            ContentClass::Visible => Some(self.redact_visible_text(&fragment.value)),
            ContentClass::Thinking | ContentClass::Secret => None,
        }
    }

    /// Recursively rebuilds `value`, applying [`Redactor::redact_visible_text`]
    /// to every string it contains (both object keys and string values).
    fn redact_json_value(&self, value: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::String(text) => {
                serde_json::Value::String(self.redact_visible_text(text))
            }
            serde_json::Value::Array(items) => serde_json::Value::Array(
                items
                    .iter()
                    .map(|item| self.redact_json_value(item))
                    .collect(),
            ),
            serde_json::Value::Object(map) => {
                let mut redacted = serde_json::Map::with_capacity(map.len());
                for (key, val) in map {
                    let redacted_key = self.redact_visible_text(key);
                    redacted.insert(redacted_key, self.redact_json_value(val));
                }
                serde_json::Value::Object(redacted)
            }
            serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {
                value.clone()
            }
        }
    }

    /// Applies the same built-in and org-defined regex rules used for
    /// `Visible` text to an always-visible, non-classified string -- a short
    /// vendor-assigned label (tool name, vendor session/child/parent
    /// identifier, role, artifact kind, ...) that carries no `ContentClass`
    /// because it is never dropped for being `Thinking`/`Secret`, but is
    /// still vendor-sourced and must not be trusted to never accidentally
    /// contain a secret-shaped value.
    #[must_use]
    pub fn redact_text(&self, text: &str) -> String {
        self.redact_visible_text(text)
    }

    /// Applies every built-in and org rule to `text`, replacing each match
    /// with `[REDACTED:<rule-id>]`.
    fn redact_visible_text(&self, text: &str) -> String {
        let mut redacted = text.to_string();

        // Apply built-in rules
        for rule in &self.rules {
            redacted = rule.apply(&redacted);
        }

        // Apply org rules
        for rule in &self.org_rules {
            redacted = rule.apply(&redacted);
        }

        redacted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(fragments: Vec<Classified<String>>) -> RawRuntimeEvent {
        RawRuntimeEvent {
            timestamp: Timestamp::now(),
            project_id: ProjectId::new(),
            run_id: None,
            kind: RawEventKind::Diagnostic {
                level: DiagnosticLevel::Info,
                code: "test".to_string(),
                fragments,
            },
        }
    }

    fn visible(value: &str) -> Classified<String> {
        Classified {
            class: ContentClass::Visible,
            value: value.to_string(),
        }
    }

    fn secret(value: &str) -> Classified<String> {
        Classified {
            class: ContentClass::Secret,
            value: value.to_string(),
        }
    }

    fn thinking(value: &str) -> Classified<String> {
        Classified {
            class: ContentClass::Thinking,
            value: value.to_string(),
        }
    }

    #[test]
    fn visible_text_survives_unchanged() {
        let redactor = Redactor::new();
        let persisted = redactor.sanitize(event(vec![visible("hello world")]));
        
        match serde_json::from_str::<RuntimeEvent>(persisted.event_json()) {
            Ok(RuntimeEvent::Diagnostic { message, .. }) => {
                assert_eq!(message, "hello world");
            }
            other => panic!("expected Diagnostic, got {:?}", other),
        }
    }

    #[test]
    fn secret_fragments_are_dropped_entirely() {
        let redactor = Redactor::new();
        let persisted = redactor.sanitize(event(vec![secret("sk-ABC...UVWX")]));

        match serde_json::from_str::<RuntimeEvent>(persisted.event_json()) {
            Ok(RuntimeEvent::Diagnostic { message, .. }) => {
                assert_eq!(message, "");
            }
            other => panic!("expected Diagnostic, got {:?}", other),
        }
    }

    #[test]
    fn thinking_fragments_are_dropped_entirely() {
        let redactor = Redactor::new();
        let persisted = redactor.sanitize(event(vec![thinking("internal reasoning")]));

        match serde_json::from_str::<RuntimeEvent>(persisted.event_json()) {
            Ok(RuntimeEvent::Diagnostic { message, .. }) => {
                assert_eq!(message, "");
            }
            other => panic!("expected Diagnostic, got {:?}", other),
        }
    }

    #[test]
    fn api_key_shaped_visible_text_is_redacted() {
        let redactor = Redactor::new();
        let persisted = redactor.sanitize(event(vec![visible(
            "key is sk-ABCDEFGHIJKLMNOPQRSTUVWX here",
        )]));

        match serde_json::from_str::<RuntimeEvent>(persisted.event_json()) {
            Ok(RuntimeEvent::Diagnostic { message, .. }) => {
                assert!(message.contains("[REDACTED:api_key]"));
                assert!(!message.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"));
            }
            other => panic!("expected Diagnostic, got {:?}", other),
        }
    }

    #[test]
    fn sanitize_json_redacts_secret_shaped_values_at_any_depth() {
        let redactor = Redactor::new();
        let value = serde_json::json!({
            "action": "spawn_worker",
            "nested": {
                "key": "sk-ABCDEFGHIJKLMNOPQRSTUVWX"
            }
        });

        let sanitized = redactor.sanitize_json(&value);
        let text = sanitized.as_str();
        assert!(text.contains("[REDACTED:api_key]"));
        assert!(!text.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"));
    }

    #[test]
    fn sanitize_json_redacts_secret_shaped_object_keys() {
        let redactor = Redactor::new();
        let value = serde_json::json!({
            "sk-ABCDEFGHIJKLMNOPQRSTUVWX": "value"
        });

        let sanitized = redactor.sanitize_json(&value);
        let text = sanitized.as_str();
        assert!(text.contains("[REDACTED:api_key]"));
        assert!(!text.contains("sk-ABCDEFGHIJKLMNOPQRSTUVWX"));
    }

    #[test]
    fn sanitize_json_is_deterministic_regardless_of_input_key_order() {
        let redactor = Redactor::new();
        let a = serde_json::json!({"a": 1, "b": 2});
        let b = serde_json::json!({"b": 2, "a": 1});

        assert_eq!(
            redactor.sanitize_json(&a).as_str(),
            redactor.sanitize_json(&b).as_str()
        );
    }

    #[test]
    fn org_patterns_are_applied_during_redaction() {
        let redactor = Redactor::with_org_rules(&["CUSTOM_SECRET_[0-9A-Z]{16}".to_string()])
            .expect("valid pattern");
        let persisted = redactor.sanitize(event(vec![visible(
            "key is CUSTOM_SECRET_ABCDEFGHIJKLMNOP here",
        )]));

        match serde_json::from_str::<RuntimeEvent>(persisted.event_json()) {
            Ok(RuntimeEvent::Diagnostic { message, .. }) => {
                assert!(message.contains("[REDACTED:org_pattern_0]"));
                assert!(!message.contains("CUSTOM_SECRET_ABCDEFGHIJKLMNOP"));
            }
            other => panic!("expected Diagnostic, got {:?}", other),
        }
    }
}
