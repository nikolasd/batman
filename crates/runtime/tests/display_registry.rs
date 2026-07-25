//! Display registry placeholder test.
use batman_protocol::ProjectId;

#[test]
fn display_registry_exists() {
    let _ = ProjectId::parse("01900000-0000-0000-0000-000000000001").unwrap();
}

