// A no-model compatibility check for the embedded monitor's two OMP
// surfaces: `pi.appendEntry` and `ctx.ui.setWidget`. Both are pinned to
// OMP 17.0.7 (`docs/extensions.md`); this fails with a named, specific
// error before the monitor ever runs if the installed peer dependency
// drifts outside the supported `>=17.0.7 <18` range, rather than letting
// a signature mismatch surface as a confusing runtime failure later.

import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";

// A plain filesystem walk (rather than resolving the subpath through
// Bun's module resolver, whether via a static `with { type: "json" }`
// import or `require`) sidesteps a Bun resolver-cache defect: resolving
// this exact package.json subpath alongside several other test files in
// one `bun test` invocation corrupts resolution state (`NameTooLong` on a
// runaway `file:` prefix chain).
function installedPiCodingAgentVersion(): string {
  let dir = import.meta.dir;
  for (;;) {
    const candidate = join(dir, "node_modules", "@oh-my-pi", "pi-coding-agent", "package.json");
    if (existsSync(candidate)) {
      return (JSON.parse(readFileSync(candidate, "utf8")) as { version: string }).version;
    }
    const parent = dirname(dir);
    if (parent === dir) {
      throw new Error("could not locate an installed @oh-my-pi/pi-coding-agent/package.json");
    }
    dir = parent;
  }
}

/** The inclusive lower bound and exclusive upper bound this extension is verified against. */
export const SUPPORTED_PI_CODING_AGENT_RANGE = { min: "17.0.7", maxExclusive: "18.0.0" } as const;

/** Raised when the installed `@oh-my-pi/pi-coding-agent` falls outside {@link SUPPORTED_PI_CODING_AGENT_RANGE}. */
export class PiCodingAgentVersionError extends Error {
  readonly installedVersion: string;

  constructor(installedVersion: string) {
    super(
      `@oh-my-pi/pi-coding-agent@${installedVersion} is outside the supported range ` +
        `[${SUPPORTED_PI_CODING_AGENT_RANGE.min}, ${SUPPORTED_PI_CODING_AGENT_RANGE.maxExclusive}) ` +
        "the embedded monitor's pi.appendEntry/ctx.ui.setWidget usage is pinned to.",
    );
    this.name = "PiCodingAgentVersionError";
    this.installedVersion = installedVersion;
  }
}

/** Parses a `major.minor.patch` version string into its numeric parts. */
function parseVersion(version: string): readonly [number, number, number] {
  const match = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
  if (match === null) {
    throw new PiCodingAgentVersionError(version);
  }
  return [Number(match[1]), Number(match[2]), Number(match[3])];
}

function compareVersions(a: readonly [number, number, number], b: readonly [number, number, number]): number {
  for (let i = 0; i < 3; i++) {
    if (a[i] !== b[i]) {
      return a[i] - b[i];
    }
  }
  return 0;
}

/**
 * Asserts `installedVersion` (defaults to the actually installed
 * `@oh-my-pi/pi-coding-agent`) falls within
 * {@link SUPPORTED_PI_CODING_AGENT_RANGE}.
 *
 * @throws {PiCodingAgentVersionError} if the version is outside the range.
 */
export function assertCompatiblePiCodingAgentVersion(
  installedVersion: string = installedPiCodingAgentVersion(),
): void {
  const installed = parseVersion(installedVersion);
  const min = parseVersion(SUPPORTED_PI_CODING_AGENT_RANGE.min);
  const maxExclusive = parseVersion(SUPPORTED_PI_CODING_AGENT_RANGE.maxExclusive);
  if (compareVersions(installed, min) < 0 || compareVersions(installed, maxExclusive) >= 0) {
    throw new PiCodingAgentVersionError(installedVersion);
  }
}
