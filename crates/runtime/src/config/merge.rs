//! Layer merging with lock enforcement.
//!
//! Configuration from multiple YAML layers (org → repo → user) is merged
//! with the following rules:
//! 1. Higher layers override lower layers for the same field.
//! 2. Org-level field locks prevent lower layers from overriding specific values.
//! 3. Unknown keys fail closed (handled in `mod.rs`).
//! 4. The result is an immutable [`RuntimePolicy`] with a SHA-256 fingerprint.

use std::collections::HashSet;
use std::path::Path;

use super::{ConfigError, ConfigLayer};

/// Errors from the configuration merge process.
#[derive(Debug, thiserror::Error)]
pub enum ConfigMergeError {
    /// A configuration file failed to parse.
    #[error(transparent)]
    Parse(#[from] ConfigError),

    /// A locked field was overridden by a lower layer.
    #[error("field '{field}' is locked by org policy; lower layer '{layer}' attempted override")]
    LockedFieldOverride { field: String, layer: String },

    /// The merge produced an invalid policy.
    #[error("merge error: {0}")]
    InvalidPolicy(String),
}

/// A single parsed configuration layer, ready for merging.
#[derive(Debug, Clone)]
pub struct ConfigLayerData {
    /// The layer this config came from.
    pub layer: ConfigLayer,
    /// The parsed YAML document.
    pub document: serde_json::Value,
    /// The source file path, if any.
    pub source: Option<String>,
}

/// All configuration layers, loaded from disk.
#[derive(Debug, Clone)]
pub struct LayeredConfig {
    /// Org-level config (lowest precedence).
    pub org: Option<ConfigLayerData>,
    /// Repo-level config.
    pub repo: Option<ConfigLayerData>,
    /// User-level config (highest static precedence).
    pub user: Option<ConfigLayerData>,
}

/// Loads a single layer from an optional path: `None` if the path is
/// `None` or does not exist, `Some(Err(_))` if it exists but fails to
/// parse, `Some(Ok(_))` on success.
fn load_layer(
    path: Option<&Path>,
    layer: ConfigLayer,
) -> Result<Option<ConfigLayerData>, ConfigMergeError> {
    let Some(path) = path else {
        return Ok(None);
    };
    if !path.exists() {
        return Ok(None);
    }
    let parsed = super::parse_config_file(path).map_err(ConfigMergeError::Parse)?;
    Ok(Some(ConfigLayerData {
        layer,
        document: parsed.document,
        source: parsed.source,
    }))
}

impl LayeredConfig {
    /// Loads configuration from the given file paths, returning all
    /// layers that exist. Missing files are silently omitted (not an error).
    ///
    /// # Errors
    /// Returns [`ConfigMergeError`] if any existing file fails to parse.
    pub fn load(
        org_path: Option<&Path>,
        repo_path: Option<&Path>,
        user_path: Option<&Path>,
    ) -> Result<Self, ConfigMergeError> {
        let org = load_layer(org_path, ConfigLayer::Org)?;
        let repo = load_layer(repo_path, ConfigLayer::Repo)?;
        let user = load_layer(user_path, ConfigLayer::User)?;

        Ok(Self { org, repo, user })
    }

    /// Merges all layers with lock enforcement, applying per-run overrides
    /// at the highest precedence. Returns a [`RuntimePolicy`] with a
    /// SHA-256 fingerprint.
    ///
    /// # Errors
    /// Returns [`ConfigMergeError`] if a locked field is overridden, or
    /// if the merged policy is invalid.
    pub fn merge(
        &self,
        per_run_params: Option<&serde_json::Value>,
    ) -> Result<RuntimePolicy, ConfigMergeError> {
        // Collect all layers in precedence order (lowest first).
        let mut layers: Vec<&ConfigLayerData> = Vec::new();
        if let Some(org) = &self.org {
            layers.push(org);
        }
        if let Some(repo) = &self.repo {
            layers.push(repo);
        }
        if let Some(user) = &self.user {
            layers.push(user);
        }

        // Extract org-level locks.
        let org_locks: HashSet<String> = self
            .org
            .as_ref()
            .and_then(|o| o.document.get("locks"))
            .and_then(|v| v.as_object())
            .map(|locks| locks.keys().cloned().collect())
            .unwrap_or_default();

        // Merge from lowest to highest precedence, checking locks.
        let mut merged = serde_json::Map::new();

        for layer in &layers {
            let Some(obj) = layer.document.as_object() else {
                continue;
            };
            for (key, value) in obj {
                // Skip the "locks" key itself — it's metadata, not a policy field.
                if key == "locks" {
                    continue;
                }

                // Check if this field is locked by org policy: only the org
                // layer itself may set a locked field.
                if org_locks.contains(key.as_str()) && layer.layer != ConfigLayer::Org {
                    return Err(ConfigMergeError::LockedFieldOverride {
                        field: key.clone(),
                        layer: layer.layer.to_string(),
                    });
                }

                // Higher layers override lower layers.
                merged.insert(key.clone(), value.clone());
            }
        }

        // Apply per-run params at the highest precedence. Per-run params
        // are an explicit operator override and may set locked fields.
        if let Some(params) = per_run_params.and_then(|p| p.as_object()) {
            for (key, value) in params {
                merged.insert(key.clone(), value.clone());
            }
        }

        let merged = serde_json::Value::Object(merged);

        // Compute SHA-256 fingerprint of the merged policy.
        let fingerprint = RuntimePolicy::compute_fingerprint(&merged);

        // Extract display preference (or default to auto).
        let display_backend = merged
            .get("display")
            .and_then(|d| d.get("backend"))
            .and_then(|b| b.as_str())
            .unwrap_or("auto")
            .to_string();

        // Extract retention policy.
        let retention = merged
            .get("retention")
            .and_then(|r| r.as_str())
            .unwrap_or("30d")
            .to_string();

        // Extract max_workers, clamped to [1, 32].
        let max_workers: u32 = merged
            .get("max_workers")
            .and_then(serde_json::Value::as_u64)
            .map_or(4, |v| v as u32)
            .clamp(1, 32);

        // Extract concurrency ceiling, clamped to [1, 16].
        let concurrency_ceiling: u32 = merged
            .get("concurrency")
            .and_then(|c| c.get("ceiling"))
            .and_then(serde_json::Value::as_u64)
            .map_or(2, |v| v as u32)
            .clamp(1, 16);

        // Extract model allowlist.
        let allowed_models: Vec<String> = merged
            .get("models")
            .and_then(|m| m.get("allowlist"))
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Extract org-level security patterns.
        let org_security_patterns: Vec<String> = merged
            .get("security")
            .and_then(|s| s.get("patterns"))
            .and_then(|p| p.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Extract rollout gates.
        let rollout_gates = RolloutGates::from_value(merged.get("rollout_gates"));

        Ok(RuntimePolicy {
            merged,
            fingerprint,
            display_backend,
            retention,
            max_workers,
            concurrency_ceiling,
            allowed_models,
            org_security_patterns,
            rollout_gates,
        })
    }
}

/// Rollout gates that must be resolved before production use.
#[derive(Debug, Clone)]
pub struct RolloutGates {
    /// Whether vendor terms have been accepted.
    pub vendor_terms_accepted: bool,
    /// Whether retention is configured (non-default).
    pub retention_configured: bool,
    /// Whether model allowlist is set (non-empty).
    pub model_allowlist_set: bool,
    /// Whether concurrency ceiling is explicitly set.
    pub concurrency_explicit: bool,
    /// Whether native discovery has been reviewed.
    pub native_discovery_reviewed: bool,
    /// Whether Ornith identity is configured.
    pub ornith_identity_set: bool,
}

impl RolloutGates {
    /// Returns `true` if any gate is unresolved (production-blocking).
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        !self.vendor_terms_accepted
            || !self.retention_configured
            || !self.model_allowlist_set
            || !self.concurrency_explicit
            || !self.native_discovery_reviewed
            || !self.ornith_identity_set
    }

    /// Returns the set of gate names that are unresolved.
    #[must_use]
    pub fn unresolved_gates(&self) -> Vec<&'static str> {
        let mut gates = Vec::new();
        if !self.vendor_terms_accepted {
            gates.push("vendor_terms_accepted");
        }
        if !self.retention_configured {
            gates.push("retention_configured");
        }
        if !self.model_allowlist_set {
            gates.push("model_allowlist_set");
        }
        if !self.concurrency_explicit {
            gates.push("concurrency_explicit");
        }
        if !self.native_discovery_reviewed {
            gates.push("native_discovery_reviewed");
        }
        if !self.ornith_identity_set {
            gates.push("ornith_identity_set");
        }
        gates
    }

    /// Parses rollout gates from an optional JSON value. `None` or a
    /// non-object value yields every gate unresolved.
    fn from_value(value: Option<&serde_json::Value>) -> Self {
        let Some(obj) = value.and_then(serde_json::Value::as_object) else {
            return RolloutGates {
                vendor_terms_accepted: false,
                retention_configured: false,
                model_allowlist_set: false,
                concurrency_explicit: false,
                native_discovery_reviewed: false,
                ornith_identity_set: false,
            };
        };

        let flag = |key: &str| obj.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);

        RolloutGates {
            vendor_terms_accepted: flag("vendor_terms_accepted"),
            retention_configured: flag("retention_configured"),
            model_allowlist_set: flag("model_allowlist_set"),
            concurrency_explicit: flag("concurrency_explicit"),
            native_discovery_reviewed: flag("native_discovery_reviewed"),
            ornith_identity_set: flag("ornith_identity_set"),
        }
    }
}

/// An immutable, SHA-256-fingerprinted snapshot of the merged runtime
/// policy (org → repo → user → per-run layers, resolved).
///
/// Distinct from [`crate::adapter::EffectivePolicy`], which is the
/// narrower environment-variable allowlist consumed by
/// [`crate::adapter::WorkerProfile::validate`] -- the two types describe
/// unrelated concerns despite the similar name.
#[derive(Debug, Clone)]
pub struct RuntimePolicy {
    /// The fully merged policy document (all layers resolved).
    pub merged: serde_json::Value,
    /// SHA-256 fingerprint of the merged policy (hex-encoded).
    pub fingerprint: String,
    /// The resolved display backend ("auto" if not specified).
    pub display_backend: String,
    /// Audit retention period (e.g. "30d", "90d").
    pub retention: String,
    /// Maximum number of concurrent workers.
    pub max_workers: u32,
    /// Maximum number of concurrent runs (concurrency ceiling).
    pub concurrency_ceiling: u32,
    /// Allowed model identifiers (empty = use adapter defaults).
    pub allowed_models: Vec<String>,
    /// Organization-defined security redaction patterns.
    pub org_security_patterns: Vec<String>,
    /// Rollout gates that must be resolved before production use.
    pub rollout_gates: RolloutGates,
}

impl RuntimePolicy {
    /// Returns `true` if any rollout gate is unresolved.
    #[must_use]
    pub fn is_rollout_blocked(&self) -> bool {
        self.rollout_gates.is_blocked()
    }

    /// Returns the set of unresolved rollout gate names.
    #[must_use]
    pub fn unresolved_gates(&self) -> Vec<&'static str> {
        self.rollout_gates.unresolved_gates()
    }

    /// Computes a SHA-256 fingerprint of the merged policy document as a
    /// canonical (compact, deterministic-key-order) JSON string.
    #[must_use]
    pub fn compute_fingerprint(document: &serde_json::Value) -> String {
        use sha2::{Digest, Sha256};

        let canonical = document.to_string();
        let mut hasher = Sha256::new();
        hasher.update(canonical.as_bytes());
        let result = hasher.finalize();
        format!("{result:x}")
    }
}

/// Returns the SHA-256 fingerprint of a merged policy document.
#[must_use]
#[allow(dead_code)]
pub fn fingerprint_policy(document: &serde_json::Value) -> String {
    RuntimePolicy::compute_fingerprint(document)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layered_config_empty_all_missing() {
        let layers = LayeredConfig::load(None, None, None).unwrap();
        let result = layers.merge(None).unwrap();
        assert_eq!(result.display_backend, "auto");
        assert_eq!(result.max_workers, 4);
        assert_eq!(result.concurrency_ceiling, 2);
        assert!(result.allowed_models.is_empty());
        assert!(result.org_security_patterns.is_empty());
    }

    #[test]
    fn test_precedence_user_wins_over_org() {
        let org = serde_json::json!({ "max_workers": 8, "retention": "30d" });
        let user = serde_json::json!({ "max_workers": 6 });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: Some(ConfigLayerData {
                layer: ConfigLayer::User,
                document: user,
                source: None,
            }),
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.max_workers, 6);
        assert_eq!(result.retention, "30d");
    }

    #[test]
    fn test_org_locks_reject_lower_layer_override() {
        let org = serde_json::json!({
            "max_workers": 8,
            "retention": "30d",
            "locks": { "retention": true }
        });
        let repo = serde_json::json!({ "retention": "90d" });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: Some(ConfigLayerData {
                layer: ConfigLayer::Repo,
                document: repo,
                source: None,
            }),
            user: None,
        };

        let result = layers.merge(None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("locked"));
        assert!(msg.contains("retention"));
    }

    #[test]
    fn test_per_run_overrides_locked_fields() {
        let org = serde_json::json!({
            "max_workers": 8,
            "retention": "30d",
            "locks": { "retention": true }
        });
        let per_run = serde_json::json!({ "retention": "7d" });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(Some(&per_run)).unwrap();
        assert_eq!(result.retention, "7d");
    }

    #[test]
    fn test_display_preference_resolves_to_auto() {
        let org = serde_json::json!({ "display": { "backend": "herdr" } });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.display_backend, "herdr");
    }

    #[test]
    fn test_display_user_layer_wins() {
        let org = serde_json::json!({ "display": { "backend": "herdr" } });
        let user = serde_json::json!({ "display": { "backend": "tmux" } });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: Some(ConfigLayerData {
                layer: ConfigLayer::User,
                document: user,
                source: None,
            }),
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.display_backend, "tmux");
    }

    #[test]
    fn test_fingerprint_is_deterministic() {
        let doc = serde_json::json!({ "max_workers": 4, "retention": "30d" });

        let fp1 = RuntimePolicy::compute_fingerprint(&doc);
        let fp2 = RuntimePolicy::compute_fingerprint(&doc);
        assert_eq!(fp1, fp2);
        assert_eq!(fp1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn test_fingerprint_changes_with_content() {
        let doc1 = serde_json::json!({ "max_workers": 4 });
        let doc2 = serde_json::json!({ "max_workers": 8 });

        let fp1 = RuntimePolicy::compute_fingerprint(&doc1);
        let fp2 = RuntimePolicy::compute_fingerprint(&doc2);
        assert_ne!(fp1, fp2);
    }

    #[test]
    fn test_concurrency_ceiling_clamped() {
        let org = serde_json::json!({ "concurrency": { "ceiling": 100 } });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.concurrency_ceiling, 16); // max cap
    }

    #[test]
    fn test_max_workers_clamped() {
        let org = serde_json::json!({ "max_workers": 1000 });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.max_workers, 32); // max cap
    }

    #[test]
    fn test_rollout_gates_default_blocked() {
        let doc = serde_json::json!({});
        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: doc,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert!(result.is_rollout_blocked());
        assert_eq!(result.unresolved_gates().len(), 6);
    }

    #[test]
    fn test_rollout_gates_all_clear() {
        let doc = serde_json::json!({
            "rollout_gates": {
                "vendor_terms_accepted": true,
                "retention_configured": true,
                "model_allowlist_set": true,
                "concurrency_explicit": true,
                "native_discovery_reviewed": true,
                "ornith_identity_set": true
            }
        });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: doc,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert!(!result.is_rollout_blocked());
        assert!(result.unresolved_gates().is_empty());
    }

    #[test]
    fn test_allowed_models_parsed() {
        let doc = serde_json::json!({
            "models": { "allowlist": ["gpt-4", "claude-3"] }
        });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: doc,
                source: None,
            }),
            repo: None,
            user: None,
        };

        let result = layers.merge(None).unwrap();
        assert_eq!(result.allowed_models, vec!["gpt-4", "claude-3"]);
    }

    #[test]
    fn test_full_precedence_chain_rejects_locked_override() {
        // org: max_workers=8, retention=30d (locked: retention)
        // repo: retention=90d — must be rejected (locked).
        let org = serde_json::json!({
            "max_workers": 8,
            "retention": "30d",
            "locks": { "retention": true }
        });
        let repo = serde_json::json!({ "retention": "90d" });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: Some(ConfigLayerData {
                layer: ConfigLayer::Repo,
                document: repo,
                source: None,
            }),
            user: None,
        };

        let result = layers.merge(None);
        assert!(result.is_err());
    }

    #[test]
    fn test_full_precedence_with_user_override() {
        // org: max_workers=8, retention=30d (locked: retention)
        // repo: max_workers=4 (allowed, not locked)
        // user: max_workers=6 (allowed, not locked)
        // per-run: retention=7d (allowed, per-run overrides locks)
        let org = serde_json::json!({
            "max_workers": 8,
            "retention": "30d",
            "locks": { "retention": true }
        });
        let repo = serde_json::json!({ "max_workers": 4 });
        let user = serde_json::json!({ "max_workers": 6 });
        let per_run = serde_json::json!({ "retention": "7d" });

        let layers = LayeredConfig {
            org: Some(ConfigLayerData {
                layer: ConfigLayer::Org,
                document: org,
                source: None,
            }),
            repo: Some(ConfigLayerData {
                layer: ConfigLayer::Repo,
                document: repo,
                source: None,
            }),
            user: Some(ConfigLayerData {
                layer: ConfigLayer::User,
                document: user,
                source: None,
            }),
        };

        let result = layers.merge(Some(&per_run)).unwrap();
        assert_eq!(result.retention, "7d");
        assert_eq!(result.max_workers, 6);
    }
}
