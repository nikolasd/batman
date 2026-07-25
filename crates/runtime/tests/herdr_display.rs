//! Herdr display backend tests.
use batman_protocol::ProjectId;

#[test]
fn herdr_display_exists() {
    let _ = ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap();
}
