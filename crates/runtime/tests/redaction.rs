//! Integration tests for redaction boundary.

use batman_runtime::security::redaction::Redactor;

#[tokio::test]
async fn redactor_removes_api_keys() {
    let redactor = Redactor::new();
    let input = "my_api_key=sk-1234567890abcdef";
    let output = redactor.redact_text(input);
    assert!(!output.contains("sk-1234567890abcdef"));
    assert!(output.contains("my_api_key="));
}

#[tokio::test]
async fn redactor_removes_bearer_tokens() {
    let redactor = Redactor::new();
    let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiJ9.test";
    let output = redactor.redact_text(input);
    assert!(!output.contains("Bearer eyJhbGciOiJIUzI1NiJ9.test"));
    assert!(output.contains("Authorization:"));
}

#[tokio::test]
async fn redactor_preserves_non_secret_content() {
    let redactor = Redactor::new();
    let input = "This is a normal message with no secrets.";
    let output = redactor.redact_text(input);
    assert_eq!(output, input);
}
