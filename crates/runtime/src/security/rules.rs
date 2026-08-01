//! Organization-configurable redaction rules: a bounded set of compiled
//! [`regex::Regex`] values, parsed from the `security.patterns` array in an
//! org-level configuration document, plus an optional human-readable `id`
//! extracted from an inline `# comment` after each pattern string.
//!
//! These rules are applied alongside the built-in redaction rules in
//! [`crate::security::redaction::Redactor`]. They are compiled once at
//! startup and reused for every subsequent redaction call.

use regex::Regex;

/// A single org-configured redaction rule: a compiled regex pattern with an
/// optional human-readable identifier.
#[derive(Debug, Clone)]
pub struct OrgRedactionRule {
    /// The rule's human-readable identifier, extracted from an inline `#
    /// comment` after the pattern string (if present), or generated from
    /// the pattern index.
    pub id: String,
    /// The compiled regex pattern.
    pattern: Regex,
}

impl OrgRedactionRule {
    /// Compiles a new [`OrgRedactionRule`] from a pattern string and an
    /// optional identifier.
    ///
    /// # Errors
    ///
    /// Returns an error if the pattern string is not a valid regex.
    pub fn new(id: String, pattern: &str) -> Result<Self, String> {
        let compiled =
            Regex::new(pattern).map_err(|e| format!("invalid regex '{pattern}': {e}"))?;
        Ok(Self {
            id,
            pattern: compiled,
        })
    }

    /// Returns the rule's human-readable identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns the compiled regex pattern.
    #[must_use]
    pub fn pattern(&self) -> &Regex {
        &self.pattern
    }

    /// Applies this rule to the given text, returning the redacted text.
    pub fn apply(&self, text: &str) -> String {
        self.pattern
            .replace_all(text, format!("[REDACTED:{}]", self.id).as_str())
            .to_string()
    }
}

/// Loads org-configured redaction rules from a configuration document.
///
/// # Errors
///
/// Returns an error if:
/// - The document does not contain a `security.patterns` array.
/// - Any pattern string is not a valid regex.
pub fn load_org_rules(document: &serde_json::Value) -> Result<Vec<OrgRedactionRule>, String> {
    let patterns = document
        .get("security")
        .and_then(|s| s.get("patterns"))
        .and_then(|p| p.as_array())
        .ok_or("missing security.patterns array")?;

    let mut rules = Vec::new();
    for (i, pattern_value) in patterns.iter().enumerate() {
        let pattern_str = pattern_value
            .as_str()
            .ok_or_else(|| format!("pattern at index {i} is not a string"))?;

        // Extract the rule ID from the comment (if present) or use the index.
        let id = pattern_value
            .as_str()
            .and_then(|s| s.split('#').nth(1))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("org_pattern_{i}"));

        let rule = OrgRedactionRule::new(id, pattern_str)
            .map_err(|e| format!("invalid regex at index {i}: {e}"))?;
        rules.push(rule);
    }

    Ok(rules)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_org_rules_from_yaml() {
        let doc = serde_json::json!({
            "security": {
                "patterns": [
                    "AKIA[0-9A-Z]{16}",
                    "ghp_[0-9a-zA-Z]{36}"
                ]
            }
        });

        let rules = load_org_rules(&doc).expect("valid patterns");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "org_pattern_0");
        assert_eq!(rules[1].id, "org_pattern_1");
    }

    #[test]
    fn test_load_org_rules_with_comments() {
        let doc = serde_json::json!({
            "security": {
                "patterns": [
                    "AKIA[0-9A-Z]{16}  # AWS access key ID",
                    "ghp_[0-9a-zA-Z]{36}  # GitHub PAT"
                ]
            }
        });

        let rules = load_org_rules(&doc).expect("valid patterns");
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0].id, "AWS access key ID");
        assert_eq!(rules[1].id, "GitHub PAT");
    }

    #[test]
    fn test_load_org_rules_invalid_regex() {
        let doc = serde_json::json!({
            "security": {
                "patterns": ["[invalid"]
            }
        });

        let result = load_org_rules(&doc);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid regex"));
    }

    #[test]
    fn test_load_org_rules_missing_field() {
        let doc = serde_json::json!({});

        let result = load_org_rules(&doc);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("missing security.patterns array")
        );
    }

    #[test]
    fn test_org_rule_applies_correctly() {
        let rule = OrgRedactionRule::new("test".to_string(), r"AKIA[0-9A-Z]{16}").unwrap();
        let text = "my key is AKIA1234567890ABCDEF end";
        let redacted = rule.apply(text);
        assert_eq!(redacted, "my key is [REDACTED:test] end");
    }

    #[test]
    fn test_org_rule_does_not_match_unrelated_text() {
        let rule = OrgRedactionRule::new("test".to_string(), r"AKIA[0-9A-Z]{16}").unwrap();
        let text = "this is unrelated text";
        let redacted = rule.apply(text);
        assert_eq!(redacted, "this is unrelated text");
    }
}
