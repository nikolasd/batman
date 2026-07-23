//! The durable runtime event stream: envelopes, sanitized event payloads,
//! and the content-classification types used to keep unsanitized content out
//! of the durable log.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;
use ts_rs::TS;

use crate::ids::{ProjectId, RunId, TaskId, WorkerId};

/// Canonical UTC RFC 3339 timestamp text, as carried on the wire.
///
/// Rather than expose [`time::OffsetDateTime`] across generated bindings,
/// BATMAN normalizes every timestamp to a UTC RFC 3339 string at
/// construction time; downstream consumers (including schemars/ts-rs) only
/// ever see a plain string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, JsonSchema, TS)]
#[ts(export)]
pub struct Timestamp(String);

/// Error returned when a string cannot be parsed as an RFC 3339 timestamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimestampParseError(String);

impl fmt::Display for TimestampParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid RFC 3339 timestamp: {}", self.0)
    }
}

impl std::error::Error for TimestampParseError {}

impl Timestamp {
    /// Parses an RFC 3339 timestamp and normalizes it to UTC.
    ///
    /// # Errors
    /// Returns [`TimestampParseError`] if `input` is not a valid RFC 3339
    /// timestamp.
    pub fn parse(input: &str) -> Result<Self, TimestampParseError> {
        let parsed = OffsetDateTime::parse(input, &Rfc3339)
            .map_err(|err| TimestampParseError(err.to_string()))?;
        Self::from_offset_date_time(parsed)
    }

    /// Returns the current time as a normalized UTC timestamp.
    #[must_use]
    pub fn now() -> Self {
        Self::from_offset_date_time(OffsetDateTime::now_utc())
            .expect("formatting the current UTC time as RFC 3339 cannot fail")
    }

    fn from_offset_date_time(value: OffsetDateTime) -> Result<Self, TimestampParseError> {
        let utc = value.to_offset(time::UtcOffset::UTC);
        let formatted = utc
            .format(&Rfc3339)
            .map_err(|err| TimestampParseError(err.to_string()))?;
        Ok(Self(formatted))
    }

    /// Returns the canonical RFC 3339 string representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(serde::de::Error::custom)
    }
}

/// The sensitivity classification of a piece of raw content produced by a
/// worker, before it is sanitized for the durable event log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ContentClass {
    Visible,
    Thinking,
    Secret,
}

/// A value tagged with its [`ContentClass`]. Used for raw, in-memory event
/// fields before sanitization; the durable [`RuntimeEvent`] must never
/// contain a `Classified<T>` field, only plain sanitized values.
///
/// `Debug` is implemented manually (not derived): printing a
/// `Thinking`/`Secret`-classified value must never leak its raw content,
/// even via `{:?}`, so only `Visible` values are actually printed -- see
/// the `impl fmt::Debug` below.
#[derive(Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Classified<T> {
    pub class: ContentClass,
    pub value: T,
}

/// A placeholder printed in place of a redacted `Classified` value; has its
/// own `Debug` impl so it renders without the surrounding quotes a `&str`
/// placeholder would otherwise get.
struct RedactedPlaceholder;

impl fmt::Debug for RedactedPlaceholder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted>")
    }
}

impl<T: fmt::Debug> fmt::Debug for Classified<T> {
    /// Prints `value` only when `class` is [`ContentClass::Visible`];
    /// `Thinking`/`Secret` values print [`RedactedPlaceholder`] instead, so
    /// `{:?}` on a raw classified value can never leak secret or thinking
    /// content.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug_struct = f.debug_struct("Classified");
        debug_struct.field("class", &self.class);
        match self.class {
            ContentClass::Visible => {
                debug_struct.field("value", &self.value);
            }
            ContentClass::Thinking | ContentClass::Secret => {
                debug_struct.field("value", &RedactedPlaceholder);
            }
        }
        debug_struct.finish()
    }
}

/// Identifies which subsystem produced an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EventSource {
    Runtime,
}

/// The severity of a [`RuntimeEvent::Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum DiagnosticLevel {
    Info,
    Warning,
    Error,
}

/// A sanitized, durable runtime event. Fields are plain, already-sanitized
/// types (never [`Classified`]) so that raw thinking/secret content can
/// never reach the durable log through this type.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(
    tag = "type",
    content = "payload",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export)]
pub enum RuntimeEvent {
    RuntimeStarted,
    RuntimeStopping,
    Diagnostic {
        level: DiagnosticLevel,
        code: String,
        message: String,
    },
}

/// The envelope wrapping every durable runtime event, carrying its sequence
/// number and routing metadata.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EventEnvelope {
    pub sequence: u64,
    pub timestamp: Timestamp,
    pub project_id: ProjectId,
    pub task_id: Option<TaskId>,
    pub worker_id: Option<WorkerId>,
    pub run_id: Option<RunId>,
    pub parent_worker_id: Option<WorkerId>,
    pub source: EventSource,
    pub event: RuntimeEvent,
    pub vendor_event_ref: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_normalizes_offset_to_utc_z() {
        let ts = Timestamp::parse("2024-03-05T10:15:00+02:00").unwrap();
        assert_eq!(ts.as_str(), "2024-03-05T08:15:00Z");
    }

    #[test]
    fn timestamp_rejects_invalid_input() {
        assert!(Timestamp::parse("not a timestamp").is_err());
    }

    #[test]
    fn timestamp_serializes_as_plain_string() {
        let ts = Timestamp::parse("2024-01-01T00:00:00Z").unwrap();
        assert_eq!(
            serde_json::to_value(&ts).unwrap(),
            serde_json::json!("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn runtime_event_unit_variant_is_adjacently_tagged() {
        let value = serde_json::to_value(RuntimeEvent::RuntimeStarted).unwrap();
        assert_eq!(value["type"], "runtimeStarted");
    }

    #[test]
    fn diagnostic_event_matches_exact_json_shape() {
        let event = RuntimeEvent::Diagnostic {
            level: DiagnosticLevel::Warning,
            code: "fixture".into(),
            message: "example".into(),
        };
        let value = serde_json::to_value(&event).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "type": "diagnostic",
                "payload": {
                    "level": "warning",
                    "code": "fixture",
                    "message": "example"
                }
            })
        );
    }

    #[test]
    fn classified_debug_redacts_secret_and_thinking_but_not_visible() {
        let visible = Classified {
            class: ContentClass::Visible,
            value: "plain narration".to_string(),
        };
        let secret = Classified {
            class: ContentClass::Secret,
            value: "sk-super-secret-value".to_string(),
        };
        let thinking = Classified {
            class: ContentClass::Thinking,
            value: "internal chain of thought".to_string(),
        };

        assert!(format!("{visible:?}").contains("plain narration"));
        assert!(!format!("{secret:?}").contains("sk-super-secret-value"));
        assert!(format!("{secret:?}").contains("<redacted>"));
        assert!(!format!("{thinking:?}").contains("internal chain of thought"));
        assert!(format!("{thinking:?}").contains("<redacted>"));
    }

    #[test]
    fn classified_is_not_reachable_from_runtime_event() {
        // Compile-time proof: RuntimeEvent's Diagnostic::message is a plain
        // String, not Classified<String>, so this construction is only
        // possible with sanitized content.
        let event = RuntimeEvent::Diagnostic {
            level: DiagnosticLevel::Info,
            code: "x".into(),
            message: "sanitized".into(),
        };
        match event {
            RuntimeEvent::Diagnostic { message, .. } => {
                let _: String = message;
            }
            _ => panic!("expected diagnostic"),
        }
    }
}
