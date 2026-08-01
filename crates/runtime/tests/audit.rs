//! Integration tests for audit module.

#[tokio::test]
async fn retention_prunes_old_events() {
    // TODO: Implement retention pruning test
    // This test should:
    // 1. Create a temporary state directory
    // 2. Insert some old events (with timestamps in the past)
    // 3. Call retention.prune() with a retention period
    // 4. Verify old events are removed and recent events are kept
}

#[tokio::test]
async fn export_creates_jsonl_file() {
    // TODO: Implement export test
    // This test should:
    // 1. Create a temporary state directory with some events
    // 2. Call export.export() with from/to timestamps
    // 3. Verify the output file exists and contains valid JSONL
    // 4. Verify each line is a valid JSON object
    // 5. Verify events are redacted (no secrets in output)
}

#[tokio::test]
async fn export_handles_empty_range() {
    // TODO: Implement empty range test
    // This test should:
    // 1. Create a temporary state directory with no events in range
    // 2. Call export.export() with from/to timestamps that don't overlap
    // 3. Verify the output file is empty or contains no events
}

#[tokio::test]
async fn export_filters_by_timestamp() {
    // TODO: Implement timestamp filtering test
    // This test should:
    // 1. Create a temporary state directory with events at different timestamps
    // 2. Call export.export() with specific from/to timestamps
    // 3. Verify only events within the range are exported
}
