//! Event export in JSONL format.
//!
//! Exports events from the durable journal to a JSONL file, one JSON
//! object per line, ordered by sequence.

use crate::DatabaseHandle;
use serde_json::{Value, json};

/// Event export configuration.
#[derive(Debug, Clone)]
pub struct Export {
    /// The repository path.
    pub repo: String,
    /// The state directory.
    pub state_dir: String,
    /// The start time (optional).
    pub from: Option<String>,
    /// The end time (optional).
    pub to: Option<String>,
    /// The output file path.
    pub output: String,
}

impl Export {
    /// Creates a new export configuration.
    #[must_use]
    pub fn new(
        repo: impl Into<String>,
        state_dir: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        Self {
            repo: repo.into(),
            state_dir: state_dir.into(),
            from: None,
            to: None,
            output: output.into(),
        }
    }

    /// Exports the journal's events, in sequence order, to `self.output`
    /// as JSONL. Returns the number of lines written.
    ///
    /// `from`/`to` bound the export by `timestamp`, inclusive. Timestamps
    /// are stored as RFC3339 text, so a lexicographic comparison is
    /// chronological -- the same approach `super::retention` uses.
    ///
    /// An empty result writes an empty file rather than no file: an audit
    /// consumer must be able to tell "nothing in range" from "the export
    /// never ran".
    ///
    /// # Redaction
    /// Deliberately none. Every event crossed `Redactor::sanitize` before
    /// it was persisted, so re-redacting here would imply the journal is
    /// untrusted and would silently alter audit evidence. The invariant is
    /// asserted in `crates/runtime/tests/audit.rs` instead.
    ///
    /// # Errors
    /// Returns an error if the query fails or the output file cannot be
    /// written.
    pub async fn export(&self, db: &DatabaseHandle) -> Result<u64, String> {
        let from = self.from.clone();
        let to = self.to.clone();

        let rows = db
            .run_domain_op(Box::new(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT sequence, timestamp, project_id, run_id, event_json,
                            task_id, worker_id, parent_worker_id, vendor_event_ref
                     FROM events
                     WHERE (?1 IS NULL OR timestamp >= ?1)
                       AND (?2 IS NULL OR timestamp <= ?2)
                     ORDER BY sequence",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![from, to], |row| {
                        let event_json: String = row.get(4)?;
                        Ok(json!({
                            "sequence": row.get::<_, i64>(0)?,
                            "timestamp": row.get::<_, String>(1)?,
                            "projectId": row.get::<_, String>(2)?,
                            "runId": row.get::<_, Option<String>>(3)?,
                            "taskId": row.get::<_, Option<String>>(5)?,
                            "workerId": row.get::<_, Option<String>>(6)?,
                            "parentWorkerId": row.get::<_, Option<String>>(7)?,
                            "vendorEventRef": row.get::<_, Option<String>>(8)?,
                            // Parsed, not nested as a JSON string: a
                            // consumer must not have to double-decode.
                            "event": serde_json::from_str::<Value>(&event_json)
                                .unwrap_or(Value::Null),
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(Value::Array(rows))
            }))
            .await
            .map_err(|e| format!("failed to read events: {e}"))?;

        let rows = rows.as_array().map(Vec::as_slice).unwrap_or_default();
        let mut body = String::new();
        for row in rows {
            body.push_str(&serde_json::to_string(row).map_err(|e| e.to_string())?);
            body.push('\n');
        }
        std::fs::write(&self.output, body)
            .map_err(|e| format!("failed to write {}: {e}", self.output))?;

        Ok(rows.len() as u64)
    }
}
