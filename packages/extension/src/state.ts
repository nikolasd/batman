import { isAbsolute, join } from "node:path";

/**
 * Machine-readable reason a {@link resolveStateRoot} call was rejected.
 */
export type StateRootErrorCode = "relative-override";

/**
 * Thrown by {@link resolveStateRoot} when an override environment variable
 * is set but is not an absolute path.
 */
export class StateRootError extends Error {
  readonly code: StateRootErrorCode;

  constructor(code: StateRootErrorCode, message: string) {
    super(message);
    this.name = "StateRootError";
    this.code = code;
  }
}

/**
 * Resolves the BATMAN state root directory.
 *
 * Precedence, identical to Rust's `StateRoot::resolve`:
 * 1. `BATMAN_STATE_DIR`, if set (must be absolute).
 * 2. `$XDG_STATE_HOME/omp/batman`, if `XDG_STATE_HOME` is set (must be absolute).
 * 3. `$HOME/${PI_CONFIG_DIR:-.omp}/orchestrator`.
 *
 * Pure and side-effect free: `env` and `home` are taken explicitly (never
 * `process.env`/`os.homedir()` internally) so tests can drive fixtures, and
 * so this never touches the filesystem. Unlike the Rust side, this never
 * creates the directory or checks its permissions -- the Rust runtime is
 * solely responsible for creating and securing state directories; this
 * function only computes the path to pass to `batcave`.
 *
 * @throws {StateRootError} if `BATMAN_STATE_DIR` or `XDG_STATE_HOME` is set
 * to a relative path.
 */
export function resolveStateRoot(
  env: Readonly<Record<string, string | undefined>>,
  home: string,
): string {
  const batmanStateDir = env.BATMAN_STATE_DIR;
  if (batmanStateDir !== undefined) {
    if (!isAbsolute(batmanStateDir)) {
      throw new StateRootError(
        "relative-override",
        `BATMAN_STATE_DIR must be an absolute path, got ${JSON.stringify(batmanStateDir)}`,
      );
    }
    return batmanStateDir;
  }

  const xdgStateHome = env.XDG_STATE_HOME;
  if (xdgStateHome !== undefined) {
    if (!isAbsolute(xdgStateHome)) {
      throw new StateRootError(
        "relative-override",
        `XDG_STATE_HOME must be an absolute path, got ${JSON.stringify(xdgStateHome)}`,
      );
    }
    return join(xdgStateHome, "omp", "batman");
  }

  const piConfigDir = env.PI_CONFIG_DIR ?? ".omp";
  return join(home, piConfigDir, "orchestrator");
}
