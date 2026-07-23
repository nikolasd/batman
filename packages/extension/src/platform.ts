// Selects the `batcave` binary this extension runs: a validated
// `OMP_BATMAN_BINARY` development override (see `runtime.ts`), or the
// platform-specific leaf package `batman-xtask package` assembled, verified
// by SHA-256 checksum and extension-version match before it is ever spawned.
//
// `resolveBatcave` takes `platform`/`arch`/`libc`/`env` explicitly (rather
// than reading `process.platform`/`process.arch`/`process.env` itself) so it
// stays pure and hermetically testable; production wiring lives in
// `context.ts`.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

import { resolveOverride, type SelectedBinary } from "./runtime";
import { sha256File } from "./integrity";

import pkg from "../package.json" with { type: "json" };

/** This extension's own version; leaf package versions must match it exactly. */
const EXTENSION_VERSION: string = pkg.version;

/** The four target triples the foundation ships prebuilt `batcave` leaves for. */
type SupportedTarget = "darwin-arm64" | "darwin-x64" | "linux-arm64-gnu" | "linux-x64-gnu";

/** Maps each supported target triple to its leaf package name. */
const TARGET_PACKAGES: Record<SupportedTarget, string> = {
  "darwin-arm64": "@satori/batman-darwin-arm64",
  "darwin-x64": "@satori/batman-darwin-x64",
  "linux-arm64-gnu": "@satori/batman-linux-arm64-gnu",
  "linux-x64-gnu": "@satori/batman-linux-x64-gnu",
};

/**
 * Thrown when `platform`/`arch`/`libc` do not map to a supported target
 * triple (Windows in any form, or Linux with a non-glibc libc such as musl).
 */
export class UnsupportedPlatformError extends Error {
  /** Machine-readable reason, mirrored by `status.ts`'s failure mapping. */
  readonly code = "unsupported-platform";
  readonly platform: string;
  readonly arch: string;
  readonly libc: string | undefined;

  constructor(platform: string, arch: string, libc: string | undefined) {
    super(
      `unsupported platform: platform=${platform} arch=${arch} libc=${libc ?? "unknown"} ` +
        `(supported: ${Object.keys(TARGET_PACKAGES).join(", ")})`,
    );
    this.name = "UnsupportedPlatformError";
    this.platform = platform;
    this.arch = arch;
    this.libc = libc;
  }
}

/** Machine-readable reason a package binary failed integrity validation. */
export type BinaryIntegrityErrorCode = "manifest-invalid" | "checksum-mismatch" | "version-mismatch";

/**
 * Thrown before any spawn when a packaged `batcave` binary's manifest is
 * missing/malformed, its SHA-256 does not match the manifest, or its leaf
 * package version does not match this extension's version. Never thrown for
 * an `OMP_BATMAN_BINARY` override -- override binaries are not checksummed.
 */
export class BinaryIntegrityError extends Error {
  readonly code: BinaryIntegrityErrorCode;

  constructor(code: BinaryIntegrityErrorCode, message: string) {
    super(message);
    this.name = "BinaryIntegrityError";
    this.code = code;
  }
}

/** The deterministic checksum/provenance payload `batman-xtask package` writes. */
interface LeafManifest {
  readonly name: string;
  readonly version: string;
  readonly target: string;
  readonly sha256: string;
  readonly sizeBytes: number;
}

/** Injectable seams for {@link resolveBatcave}, used to keep tests hermetic. */
export interface ResolveBatcaveDeps {
  /**
   * Resolves a leaf package name (e.g. `@satori/batman-darwin-arm64`) to its
   * installed package directory. Defaults to `import.meta.resolve`.
   */
  readonly resolveLeafDir?: (packageName: string) => string;
}

/**
 * Resolves the `batcave` binary to run.
 *
 * Order:
 * 1. A valid absolute executable `OMP_BATMAN_BINARY` in `env` wins outright
 *    -- source `"override"`. No checksum or version validation is performed
 *    for an override; validation applies only to the package path.
 * 2. Otherwise, `platform`/`arch`/`libc` are mapped to one of the four
 *    supported leaf packages (or a typed {@link UnsupportedPlatformError}).
 *    The leaf's `bin/batcave` is SHA-256-verified against its
 *    `manifest.json`, and the manifest's `version` must equal this
 *    extension's version, before returning -- source `"package"`.
 */
export function resolveBatcave(
  platform: string,
  arch: string,
  libc: string | undefined,
  env: Readonly<Record<string, string | undefined>>,
  deps: ResolveBatcaveDeps = {},
): SelectedBinary {
  const override = resolveOverride(env);
  if (override !== undefined) {
    return override;
  }

  const packageName = resolveLeafPackageName(platform, arch, libc);
  const resolveLeafDir = deps.resolveLeafDir ?? defaultResolveLeafDir;
  const leafDir = resolveLeafDir(packageName);

  const manifestPath = join(leafDir, "manifest.json");
  const binPath = join(leafDir, "bin", "batcave");
  const manifest = readManifest(manifestPath);

  const actualSha256 = sha256File(binPath);
  if (actualSha256 !== manifest.sha256) {
    throw new BinaryIntegrityError(
      "checksum-mismatch",
      `checksum mismatch for ${binPath}: manifest ${manifestPath} declares ${manifest.sha256}, ` +
        `computed ${actualSha256}`,
    );
  }

  if (manifest.version !== EXTENSION_VERSION) {
    throw new BinaryIntegrityError(
      "version-mismatch",
      `leaf package ${packageName} is version ${manifest.version}, but this extension is ` +
        `version ${EXTENSION_VERSION}`,
    );
  }

  return { path: binPath, source: "package" };
}

/** Maps a platform/arch/libc tuple to its leaf package name, or throws. */
function resolveLeafPackageName(platform: string, arch: string, libc: string | undefined): string {
  const target = mapTarget(platform, arch, libc);
  if (target === undefined) {
    throw new UnsupportedPlatformError(platform, arch, libc);
  }
  return TARGET_PACKAGES[target];
}

function mapTarget(platform: string, arch: string, libc: string | undefined): SupportedTarget | undefined {
  if (platform === "darwin" && arch === "arm64") {
    return "darwin-arm64";
  }
  if (platform === "darwin" && arch === "x64") {
    return "darwin-x64";
  }
  if (platform === "linux" && arch === "arm64" && libc === "glibc") {
    return "linux-arm64-gnu";
  }
  if (platform === "linux" && arch === "x64" && libc === "glibc") {
    return "linux-x64-gnu";
  }
  return undefined;
}

/** Reads and parses a leaf package's `manifest.json`. */
function readManifest(manifestPath: string): LeafManifest {
  let raw: string;
  try {
    raw = readFileSync(manifestPath, "utf8");
  } catch (err) {
    throw new BinaryIntegrityError(
      "manifest-invalid",
      `unable to read manifest at ${manifestPath}: ${(err as Error).message}`,
    );
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch (err) {
    throw new BinaryIntegrityError(
      "manifest-invalid",
      `manifest at ${manifestPath} is not valid JSON: ${(err as Error).message}`,
    );
  }

  if (
    typeof parsed !== "object" ||
    parsed === null ||
    typeof (parsed as Partial<LeafManifest>).sha256 !== "string" ||
    typeof (parsed as Partial<LeafManifest>).version !== "string"
  ) {
    throw new BinaryIntegrityError(
      "manifest-invalid",
      `manifest at ${manifestPath} is missing required string fields "sha256"/"version"`,
    );
  }

  return parsed as LeafManifest;
}

/**
 * Resolves an installed leaf package's directory via `import.meta.resolve`,
 * which -- per the leaf packages' `exports` map -- resolves `<name>/package.json`
 * to the leaf's own `package.json`, whose directory contains `bin/batcave`
 * and `manifest.json`.
 */
function defaultResolveLeafDir(packageName: string): string {
  const resolved = import.meta.resolve(`${packageName}/package.json`);
  return dirname(fileURLToPath(resolved));
}

/**
 * Best-effort Linux libc detection: `"glibc"`, `"musl"`, or `undefined` when
 * undetermined (which `resolveBatcave` treats as unsupported). Not meaningful
 * off Linux. Foundation-scope heuristic: checks Node's build report for a
 * glibc runtime version, then falls back to checking for musl's well-known
 * dynamic loader paths.
 */
export function detectLibc(platform: string = process.platform): string | undefined {
  if (platform !== "linux") {
    return undefined;
  }

  try {
    const report = (process.report?.getReport() as { header?: { glibcVersionRuntime?: string } })
      ?.header;
    if (report?.glibcVersionRuntime) {
      return "glibc";
    }
  } catch {
    // Fall through to musl detection below.
  }

  const muslLoaders = ["/lib/ld-musl-x86_64.so.1", "/lib/ld-musl-aarch64.so.1"];
  if (muslLoaders.some((loader) => existsSync(loader))) {
    return "musl";
  }

  return undefined;
}
