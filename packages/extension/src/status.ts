// The single status path shared by both the `batman_status` tool and the
// `/batman-status` command: connect to (or spawn) the repository's `batcave`
// runtime, call `runtime/status`, and shape the result. Kept independent of
// OMP's extension types so it is trivial to unit test.

import { BinarySelectionError, ensureRuntime, type EnsureRuntimeOptions } from "./runtime";
import type { BatmanClient } from "./client";
import type { RuntimeStatus } from "@satori/batman-protocol";

/** A text content block, structurally compatible with OMP's `TextContent`. */
export interface StatusTextContent {
  readonly type: "text";
  readonly text: string;
}

/** Sanitized, machine-readable detail returned when the runtime is unreachable. */
export interface RuntimeStatusFailure {
  /** Machine-readable reason, e.g. a {@link BinarySelectionError} code. */
  readonly code: string;
  /** A message safe to display: no stack frames, no environment values. */
  readonly message: string;
  /** A command the operator can run locally to diagnose further. */
  readonly doctorCommand: string;
}

/** Successful result: the runtime answered `runtime/status`. */
export interface RuntimeStatusSuccess {
  content: [StatusTextContent];
  readonly details: RuntimeStatus;
  readonly isError?: false;
}

/** Failure result: the runtime could not be reached or started. */
export interface RuntimeStatusError {
  content: [StatusTextContent];
  readonly details: RuntimeStatusFailure;
  readonly isError: true;
}

export type RuntimeStatusResult = RuntimeStatusSuccess | RuntimeStatusError;

/** Reads and writes the single cached client for the calling extension instance. */
export interface BatmanClientCache {
  get(): BatmanClient | undefined;
  set(client: BatmanClient | undefined): void;
}

/** Context {@link getRuntimeStatus} needs: where to connect, and the client cache. */
export interface GetRuntimeStatusContext {
  readonly ensureRuntimeOptions: EnsureRuntimeOptions;
  readonly cache: BatmanClientCache;
}

const GENERIC_FAILURE_MESSAGE =
  "The BATMAN runtime is not reachable for this repository. Run the doctor command below for details.";

/**
 * Returns the current `runtime/status` for the repository named in
 * `ctx.ensureRuntimeOptions`, connecting to (or spawning) the runtime via the
 * cached client when available. Never throws: connection failures are
 * reported as a sanitized {@link RuntimeStatusError} instead.
 */
export async function getRuntimeStatus(ctx: GetRuntimeStatusContext): Promise<RuntimeStatusResult> {
  let client = ctx.cache.get();

  if (client === undefined) {
    try {
      const connected = await ensureRuntime(ctx.ensureRuntimeOptions);
      client = connected.client;
      ctx.cache.set(client);
    } catch (err) {
      return failureResult(ctx.ensureRuntimeOptions, err);
    }
  }

  try {
    const status = (await client.request("runtime/status")) as RuntimeStatus;
    return {
      content: [{ type: "text", text: formatStatus(status) }],
      details: status,
    };
  } catch (err) {
    // The cached client's connection is no longer good; close it before
    // dropping the reference so its socket, listeners, and pending-request
    // map don't leak, then let the next call attempt a fresh `ensureRuntime`.
    try {
      client.close();
    } catch {
      // Best-effort: the client is already being discarded.
    }
    ctx.cache.set(undefined);
    return failureResult(ctx.ensureRuntimeOptions, err);
  }
}

function failureResult(options: EnsureRuntimeOptions, err: unknown): RuntimeStatusError {
  const code = err instanceof BinarySelectionError ? err.code : "connection-failed";
  const doctorCommand = `batcave status --repo ${options.repository}`;
  return {
    isError: true,
    content: [{ type: "text", text: GENERIC_FAILURE_MESSAGE }],
    details: { code, message: GENERIC_FAILURE_MESSAGE, doctorCommand },
  };
}

function formatStatus(status: RuntimeStatus): string {
  return [
    `BATMAN runtime: ${status.running ? "running" : "not running"}`,
    `Protocol: ${status.protocol.major}.${status.protocol.minor} (healthy: ${status.protocolHealthy})`,
    `Project: ${status.projectId}`,
    `Active runs: ${status.activeRuns}`,
    `Schema version: ${status.schemaVersion}`,
    `Uptime: ${status.uptimeSeconds}s`,
    `Binary source: ${status.binarySource}`,
  ].join("\n");
}
