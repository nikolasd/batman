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
    /// How to handle nested worker policy violations (quarantine/cancel/quarantineAndCancel).
    pub nested_violation_action: NestedViolationAction,
    /// Whether development binary override is allowed (defaults to false outside fixture/development mode).
    pub allow_development_binary_override: bool,
}

/// How to handle nested worker policy violations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NestedViolationAction {
    /// Quarantine the nested worker (blocks all side effects, requires explicit release).
    Quarantine,
    /// Cancel the nested worker (audited adapter path).
    Cancel,
    /// Quarantine then cancel (default).
    #[default]
    QuarantineAndCancel,
}

impl std::fmt::Display for NestedViolationAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Quarantine => write!(f, "quarantine"),
            Self::Cancel => write!(f, "cancel"),
            Self::QuarantineAndCancel => write!(f, "quarantineAndCancel"),
        }
    }
}


impl<'de> serde::Deserialize<'de> for NestedViolationAction {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.to_lowercase().as_str() {
            "quarantine" => Ok(Self::Quarantine),
            "cancel" => Ok(Self::Cancel),
            "quarantineandcancel" | "quarantine_and_cancel" => Ok(Self::QuarantineAndCancel),
            _ => Err(serde::de::Error::custom(format!(
                "invalid nested_violation_action: {s}, expected 'quarantine', 'cancel', or 'quarantineAndCancel'"
            ))),
        }
    }
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
            || self.allow_development_binary_override
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
        if self.allow_development_binary_override {
            gates.push("allow_development_binary_override");
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
                nested_violation_action: NestedViolationAction::default(),
                allow_development_binary_override: false,
            };
        };

        let flag = |key: &str| obj.get(key).and_then(serde_json::Value::as_bool).unwrap_or(false);
        let nested_action = obj
            .get("nested_violation_action")
            .and_then(serde_json::Value::as_str)
            .map(|s| match s.to_lowercase().as_str() {
                "quarantine" => NestedViolationAction::Quarantine,
                "cancel" => NestedViolationAction::Cancel,
                "quarantineandcancel" | "quarantine_and_cancel" => {
                    NestedViolationAction::QuarantineAndCancel
                }
                _ => NestedViolationAction::default(),
            })
            .unwrap_or_default();

        RolloutGates {
            vendor_terms_accepted: flag("vendor_terms_accepted"),
            retention_configured: flag("retention_configured"),
            model_allowlist_set: flag("model_allowlist_set"),
            concurrency_explicit: flag("concurrency_explicit"),
            native_discovery_reviewed: flag("native_discovery_reviewed"),
            ornith_identity_set: flag("ornith_identity_set"),
            nested_violation_action: nested_action,
            allow_development_binary_override: flag("allow_development_binary_override"),
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

    /// Returns the set of unresolved gate names.
    #[must_use]
    pub fn unresolved_gates(&self) -> Vec<&'static str> {
        self.rollout_gates.unresolved_gates()
    }

    /// Computes a SHA-256 fingerprint of the merged policy document.
    #[must_use]
    pub fn compute_fingerprint(merged: &serde_json::Value) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(merged.to_string().as_bytes());
        hex::encode(hasher.finalize())
    }
}
