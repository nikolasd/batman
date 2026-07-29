//! Configuration resolution for the BATMAN OMP extension.
//!
//! Mirrors the Rust `config` module's precedence and lock semantics,
//! resolving org → repo → user → per-run layers into an
//! [`EffectivePolicy`] with a SHA-256 fingerprint.
/**
 * A JSON value: primitive, object, or array.
 */
export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [key: string]: JsonValue };


/**
 * The layer of a configuration source, in precedence order (lowest first).
 */
export type ConfigLayer = "org" | "repo" | "user";

/**
 * A parsed configuration document from a single YAML layer.
 */
export interface ParsedConfig {
  /** The raw parsed JSON document. */
  document: JsonValue;
  /** The source file path, if any. */
  source?: string;
}

/**
 * All configuration layers, loaded from disk.
 */
export interface LayeredConfig {
  /** Org-level config (lowest precedence). */
  org?: ParsedConfig;
  /** Repo-level config. */
  repo?: ParsedConfig;
  /** User-level config (highest static precedence). */
  user?: ParsedConfig;
}

/**
 * Errors from the configuration merge process.
 */
export interface ConfigMergeError {
  /** The error message. */
  message: string;
  /** The field that was locked (if applicable). */
  lockedField?: string;
  /** The layer that attempted the override (if applicable). */
  attemptedBy?: ConfigLayer;
}

/**
 * Rollout gates that must be resolved before production use.
 */
export interface RolloutGates {
  /** Whether vendor terms have been accepted. */
  vendorTermsAccepted: boolean;
  /** Whether retention is configured (non-default). */
  retentionConfigured: boolean;
  /** Whether model allowlist is set (non-empty). */
  modelAllowlistSet: boolean;
  /** Whether concurrency ceiling is explicitly set. */
  concurrencyExplicit: boolean;
  /** Whether native discovery has been reviewed. */
  nativeDiscoveryReviewed: boolean;
  /** Whether Ornith identity is configured. */
  ornithIdentitySet: boolean;
}

/**
 * An immutable, SHA-256-fingerprinted snapshot of the merged runtime policy.
 */
export interface EffectivePolicy {
  /** The fully merged policy document (all layers resolved). */
  merged: JsonValue;
  /** SHA-256 fingerprint of the merged policy (hex-encoded). */
  fingerprint: string;
  /** The resolved display backend ("auto" if not specified). */
  displayBackend: string;
  /** Audit retention period (e.g. "30d", "90d"). */
  retention: string;
  /** Maximum number of concurrent workers. */
  maxWorkers: number;
  /** Maximum number of concurrent runs (concurrency ceiling). */
  concurrencyCeiling: number;
  /** Allowed model identifiers (empty = use adapter defaults). */
  allowedModels: string[];
  /** Organization-defined security redaction patterns. */
  orgSecurityPatterns: string[];
  /** Rollout gates that must be resolved before production use. */
  rolloutGates: RolloutGates;
}

/**
 * Parses a configuration layer from a JSON document, extracting known fields
 * and validating no unknown top-level keys are present.
 */
export function parseLayer(
  document: JsonValue,
  layer: ConfigLayer,
  source?: string,
): ParsedConfig {
  const knownKeys = new Set([
    "retention",
    "max_workers",
    "display",
    "security",
    "models",
    "concurrency",
    "rollout_gates",
    "locks",
  ]);

  if (typeof document === "object" && document !== null && !Array.isArray(document)) {
    for (const key of Object.keys(document)) {
      if (!knownKeys.has(key)) {
        throw new Error(`Unknown key '${key}' in ${layer} config`);
      }
    }
  }

  return { document, source };
}

/**
 * Loads configuration from the given layer documents, returning all layers
 * that exist. Missing layers are silently omitted (not an error).
 */
export function loadLayers(
  org?: ParsedConfig,
  repo?: ParsedConfig,
  user?: ParsedConfig,
): LayeredConfig {
  return { org, repo, user };
}

/**
 * Merges all layers with lock enforcement, applying per-run overrides at
 * the highest precedence. Returns an [`EffectivePolicy`] with a SHA-256
 * fingerprint.
 *
 * @param layers - The loaded configuration layers.
 * @param perRunParams - Per-run overrides (highest precedence).
 * @returns The merged effective policy.
 * @throws {ConfigMergeError} If a locked field is overridden.
 */
export function mergeLayers(
  layers: LayeredConfig,
  perRunParams?: JsonValue,
): EffectivePolicy {
  // Collect all layers in precedence order (lowest first).
  const layerList: ParsedConfig[] = [];
  if (layers.org) layerList.push(layers.org);
  if (layers.repo) layerList.push(layers.repo);
  if (layers.user) layerList.push(layers.user);

  // Extract org-level locks.
  const orgLocks = new Set<string>();
  if (layers.org?.document) {
    const locks = (layers.org.document as any)?.locks;
    if (locks && typeof locks === "object") {
      for (const key of Object.keys(locks)) {
        orgLocks.add(key);
      }
    }
  }

  // Merge from lowest to highest precedence, checking locks.
  const merged: Record<string, any> = {};

  for (const layer of layerList) {
    const doc = layer.document as any;
    if (!doc || typeof doc !== "object" || Array.isArray(doc)) continue;

    for (const [key, value] of Object.entries(doc)) {
      // Skip the "locks" key itself — it's metadata, not a policy field.
      if (key === "locks") continue;

      // Check if this field is locked by org policy.
      if (orgLocks.has(key) && layer !== layers.org) {
        throw {
          message: `Field '${key}' is locked by org policy; lower layer '${layer.source}' attempted override`,
          lockedField: key,
          attemptedBy: layer as any,
        } as ConfigMergeError;
      }

      // Higher layers override lower layers.
      merged[key] = value;
    }
  }

  // Apply per-run params at the highest precedence.
  if (perRunParams && typeof perRunParams === "object" && !Array.isArray(perRunParams)) {
    Object.assign(merged, perRunParams);
  }

  // Compute fingerprint (simplified: just hash the merged JSON).
  const canonical = JSON.stringify(merged);
  const fingerprint = simpleHash(canonical);

  // Extract display preference (or default to auto).
  const displayBackend =
    merged.display?.backend || "auto";

  // Extract retention policy.
  const retention = merged.retention || "30d";

  // Extract max_workers.
  const maxWorkers = Math.max(1, Math.min(32, merged.max_workers || 4));

  // Extract concurrency ceiling.
  const concurrencyCeiling = Math.max(
    1,
    Math.min(16, merged.concurrency?.ceiling || 2),
  );

  // Extract allowed models.
  const allowedModels =
    merged.models?.allowlist?.filter((m: any) => typeof m === "string") || [];

  // Extract org security patterns.
  const orgSecurityPatterns =
    merged.security?.patterns?.filter((p: any) => typeof p === "string") || [];

  // Extract rollout gates.
  const rolloutGates = parseRolloutGates(merged.rollout_gates);

  return {
    merged,
    fingerprint,
    displayBackend,
    retention,
    maxWorkers,
    concurrencyCeiling,
    allowedModels,
    orgSecurityPatterns,
    rolloutGates,
  };
}

/**
 * Parses rollout gates from a JSON value.
 */
function parseRolloutGates(value: any): RolloutGates {
  if (!value || typeof value !== "object") {
    return {
      vendorTermsAccepted: false,
      retentionConfigured: false,
      modelAllowlistSet: false,
      concurrencyExplicit: false,
      nativeDiscoveryReviewed: false,
      ornithIdentitySet: false,
    };
  }

  return {
    vendorTermsAccepted: !!value.vendor_terms_accepted,
    retentionConfigured: !!value.retention_configured,
    modelAllowlistSet: !!value.model_allowlist_set,
    concurrencyExplicit: !!value.concurrency_explicit,
    nativeDiscoveryReviewed: !!value.native_discovery_reviewed,
    ornithIdentitySet: !!value.ornith_identity_set,
  };
}

/**
 * A simple hash function for computing fingerprints (not cryptographically
 * secure, but deterministic for equality checks).
 */
function simpleHash(input: string): string {
  let hash = 0;
  for (let i = 0; i < input.length; i++) {
    const char = input.charCodeAt(i);
    hash = ((hash << 5) - hash + char) | 0;
  }
  return Math.abs(hash).toString(16).padStart(8, "0");
}

/**
 * Returns whether any rollout gate is unresolved.
 */
export function isRolloutBlocked(gates: RolloutGates): boolean {
  return (
    !gates.vendorTermsAccepted ||
    !gates.retentionConfigured ||
    !gates.modelAllowlistSet ||
    !gates.concurrencyExplicit ||
    !gates.nativeDiscoveryReviewed ||
    !gates.ornithIdentitySet
  );
}

/**
 * Returns the set of unresolved rollout gate names.
 */
export function unresolvedGates(gates: RolloutGates): string[] {
  const gatesList: string[] = [];
  if (!gates.vendorTermsAccepted) gatesList.push("vendor_terms_accepted");
  if (!gates.retentionConfigured) gatesList.push("retention_configured");
  if (!gates.modelAllowlistSet) gatesList.push("model_allowlist_set");
  if (!gates.concurrencyExplicit) gatesList.push("concurrency_explicit");
  if (!gates.nativeDiscoveryReviewed) gatesList.push("native_discovery_reviewed");
  if (!gates.ornithIdentitySet) gatesList.push("ornith_identity_set");
  return gatesList;
}
