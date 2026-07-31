// The `batcave doctor` CLI command wrapper: spawn the binary with `--json`
// and parse the structured output. Used by both the `batman_doctor` tool and
// the `/batman-doctor` command.
//
// Unlike `status.ts`, this does not connect to a running runtime — it invokes
// the CLI directly, so it works even when no runtime is serving the repo.

import { spawn } from "node:child_process";
import { detectLibc, resolveBatcave } from "./platform";

/** A single failed check from the doctor output. */
export interface FailedCheck {
  /** The name of the check. */
  readonly check_name: string;
  /** The error message. */
  readonly error: string;
}

/** Successful doctor output. */
export interface DoctorResult {
  /** Whether the runtime is healthy. */
  readonly healthy: boolean;
  /** The set of checks that passed. */
  readonly passed_checks: string[];
  /** The set of checks that failed, with error messages. */
  readonly failed_checks: FailedCheck[];
  /** The set of unresolved rollout gates. */
  readonly unresolved_gates: string[];
}

/** Sanitized, machine-readable detail when the doctor command fails. */
export interface DoctorFailure {
  /** Machine-readable error code. */
  readonly code: string;
  /** Human-readable error message. */
  readonly message: string;
  /** The command the operator can run to diagnose further. */
  readonly doctorCommand: string;
}

/** Successful result from the doctor command. */
export interface DoctorSuccess {
  /** Content blocks for display. */
  readonly content: [DoctorTextContent];
  /** Parsed doctor result. */
  readonly details: DoctorResult;
}

/** Failure result from the doctor command. */
export interface DoctorErrorResult {
  /** Always true for errors. */
  readonly isError: true;
  /** Content blocks for display. */
  readonly content: [DoctorTextContent];
  /** Machine-readable failure details. */
  readonly details: DoctorFailure;
}

export type DoctorTextContent = {
  type: "text";
  text: string;
};

export type DoctorCommandResult = DoctorSuccess | DoctorErrorResult;

/** Context needed to run the doctor command. */
export interface DoctorContext {
  /** Absolute BATMAN state root. */
  readonly stateDir: string;
  /** Absolute repository path. */
  readonly repository: string;
  /** Path to the `batcave` binary. */
  readonly batcavePath: string;
}

/**
 * Builds the doctor context for the given working directory. Resolves the
 * `batcave` binary path using the same logic as `status.ts`.
 */
export function buildDoctorContext(cwd: string, env: NodeJS.ProcessEnv = process.env): DoctorContext {
  const binary = resolveBatcave(process.platform, process.arch, detectLibc(), env);
  return {
    stateDir: resolveStateDir(cwd),
    repository: cwd,
    batcavePath: binary.path,
  };
}

/**
 * Runs `batcave doctor --json` and parses the structured output.
 *
 * This is a synchronous spawn (no network, no runtime connection), so it
 * works even when no runtime is serving the repository.
 */
export async function runDoctorCommand(ctx: DoctorContext): Promise<DoctorCommandResult> {
  return new Promise<DoctorCommandResult>((resolve) => {
    const proc = spawn(ctx.batcavePath, ["doctor", "--json", "--state-dir", ctx.stateDir, "--repo", ctx.repository], {
      stdio: ["ignore", "pipe", "pipe"],
    });

    let stdout = "";
    let stderr = "";

    proc.stdout.on("data", (chunk: Buffer) => {
      stdout += chunk.toString();
    });

    proc.stderr.on("data", (chunk: Buffer) => {
      stderr += chunk.toString();
    });

    proc.on("close", (code) => {
      const exitCode = code ?? 1;
      const doctorCommand = `${ctx.batcavePath} doctor --state-dir ${ctx.stateDir} --repo ${ctx.repository}`;

      if (exitCode !== 0) {
        // Parse JSON from stdout if available, otherwise use stderr
        try {
          const parsed = JSON.parse(stdout);
          if (parsed && typeof parsed === "object" && "healthy" in parsed) {
            resolve({
              isError: true,
              content: [{ type: "text", text: formatDoctorOutput(parsed as DoctorResult) }],
              details: {
                code: "doctor-failed",
                message: stderr || `Doctor command exited with code ${exitCode}`,
                doctorCommand,
              },
            });
          } else {
            resolve(failureResult(ctx, "doctor-failed", stderr || `Doctor command exited with code ${exitCode}`, doctorCommand));
          }
        } catch {
          resolve(failureResult(ctx, "doctor-failed", stderr || `Doctor command exited with code ${exitCode}`, doctorCommand));
        }
      } else {
        // Parse JSON from stdout
        try {
          const parsed: DoctorResult = JSON.parse(stdout);
          resolve({
            content: [{ type: "text", text: formatDoctorOutput(parsed) }],
            details: parsed,
          });
        } catch (err) {
          const message = err instanceof Error ? err.message : "Failed to parse doctor output";
          resolve(failureResult(ctx, "parse-error", message, doctorCommand));
        }
      }
    });

    proc.on("error", (err) => {
      const doctorCommand = `${ctx.batcavePath} doctor --state-dir ${ctx.stateDir} --repo ${ctx.repository}`;
      resolve(failureResult(ctx, "spawn-error", err.message, doctorCommand));
    });
  });
}

function failureResult(
  ctx: DoctorContext,
  code: string,
  message: string,
  doctorCommand?: string,
): DoctorErrorResult {
  return {
    isError: true,
    content: [{ type: "text", text: `Doctor command failed: ${message}` }],
    details: {
      code,
      message,
      doctorCommand: doctorCommand ?? `${ctx.batcavePath} doctor --state-dir ${ctx.stateDir} --repo ${ctx.repository}`,
    },
  };
}

function resolveStateDir(cwd: string): string {
  const path = require("node:path");
  return path.join(cwd, ".batman");
}

function formatDoctorOutput(result: DoctorResult): string {
  const lines: string[] = [];
  lines.push(`Doctor check: ${result.healthy ? "healthy" : "failed"}`);

  if (result.passed_checks.length > 0) {
    lines.push(`Passed checks: ${result.passed_checks.join(", ")}`);
  }

  if (result.failed_checks.length > 0) {
    lines.push("Failed checks:");
    for (const check of result.failed_checks) {
      lines.push(`  - ${check.check_name}: ${check.error}`);
    }
  }

  if (result.unresolved_gates.length > 0) {
    lines.push(`Unresolved gates: ${result.unresolved_gates.join(", ")}`);
  }

  return lines.join("\n");
}
