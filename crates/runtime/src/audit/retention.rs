//! Event retention and pruning.
//!
//! Prunes events older than the configured retention period.

use crate::db::DomainClosure;
use crate::DatabaseHandle;

/// Retention policy for event pruning.
#[derive(Debug, Clone)]
pub struct Retention {
    pub period: String,
}

impl Retention {
    #[must_use]
    pub fn new(period: impl Into<String>) -> Self {
        Self { period: period.into() }
    }

    pub async fn prune(&self, db_handle: &DatabaseHandle) -> Result<(), String> {
        let period = parse_period(&self.period)?;
        
        // Calculate the cutoff timestamp (cast to i64 for rusqlite compatibility)
        let cutoff: i64 = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| format!("failed to get current time: {e}"))?
            .as_secs()
            .checked_sub(period)
            .ok_or_else(|| "retention period exceeds system time".to_string())?
            .try_into()
                .map_err(|_| "retention period exceeds i64::MAX seconds".to_string())?;
        
        // Build the closure to delete events older than the cutoff (bounded batch + active-run protection)
        let closure = Box::new(move |conn: &mut rusqlite::Connection| -> Result<serde_json::Value, crate::domain::DomainError> {
            // Delete events older than the cutoff in batches (1000 per batch), excluding events belonging to non-terminal runs
            let mut deleted = 0;
            loop {
                let affected = conn
                    .execute(
                        "DELETE FROM events WHERE sequence IN (SELECT sequence FROM events WHERE timestamp < ?1 AND (run_id IS NULL OR run_id NOT IN (SELECT run_id FROM runs WHERE state NOT IN ('completed','failed','cancelled'))) LIMIT 1000)",
                        rusqlite::params![cutoff],
                    )
                    .map_err(crate::domain::DomainError::Sqlite)?;
                
                if affected == 0 {
                    break;
                }
                deleted += affected;
            }
            
            Ok(serde_json::json!({"deleted": deleted}))
        }) as DomainClosure;
        
        // Use the existing run_domain_op method to execute the closure
        db_handle.run_domain_op(closure)
            .await
            .map_err(|e| format!("failed to execute prune operation: {e}"))?;
        
        Ok(())
    }
}

fn parse_period(period: &str) -> Result<u64, String> {
    let period = period.trim();
    if let Some(days) = period.strip_suffix("d") {
        days.parse::<u64>()
            .map(|d| d * 24 * 60 * 60)
            .map_err(|e| format!("invalid period: {e}"))
    } else if let Some(months) = period.strip_suffix("mo") {
        months.parse::<u64>()
            .map(|m| m * 30 * 24 * 60 * 60)
            .map_err(|e| format!("invalid period: {e}"))
    } else if let Some(years) = period.strip_suffix("y") {
        years.parse::<u64>()
            .map(|y| y * 365 * 24 * 60 * 60)
            .map_err(|e| format!("invalid period: {e}"))
    } else {
        Err(format!("invalid period format: {period}, expected format like '30d', '90d', '1y'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retention_new() {
        let retention = Retention::new("30d");
        assert_eq!(retention.period, "30d");
    }

    #[tokio::test]
    async fn test_retention_prune() {
        let retention = Retention::new("30d");
        // Note: This test requires a real DatabaseHandle to work properly
        // For now, just verify the struct can be created
        assert_eq!(retention.period, "30d");
    }
}
