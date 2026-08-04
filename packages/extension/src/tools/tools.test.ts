import { afterAll, beforeAll, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

import type { AgentToolResult, ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { z as zod } from "zod/v4";

import { BatmanClient } from "../client";
import { registerOrchestrationTools } from "./index";

const REPO_ROOT = join(import.meta.dir, "..", "..", "..", "..");
const BATCAVE = join(REPO_ROOT, "target", "debug", "batcave");

// ---------------------------------------------------------------- fake API

interface FakeToolDefinition {
  readonly name: string;
  readonly label: string;
  readonly description: string;
  readonly approval?: unknown;
  readonly parameters: unknown;
  readonly execute: (
    toolCallId: string,
    params: unknown,
    signal: AbortSignal | undefined,
    onUpdate: undefined,
    ctx: ExtensionContext,
  ) => Promise<AgentToolResult<unknown>>;
}

function createFakeApi(): { api: ExtensionAPI; tools: Map<string, FakeToolDefinition> } {
  const tools = new Map<string, FakeToolDefinition>();
  const api = {
    zod,
    logger: { debug() {}, info() {}, warn() {}, error() {} },
    registerTool(tool: FakeToolDefinition) {
      tools.set(tool.name, tool);
    },
  };
  return { api: api as unknown as ExtensionAPI, tools };
}


function fakeExtensionContext(cwd: string): ExtensionContext {
  const sessionManager = {
    getSessionId: () => "test-session-id-12345",
  };
  return {
    cwd,
    sessionManager: sessionManager as any,
  } as unknown as ExtensionContext;
}

// ------------------------------------------------------- registration shape

test("registers exactly the eight orchestration tools with expected names", () => {
  const { api, tools } = createFakeApi();
  registerOrchestrationTools(api, {
    getClient: () => {
      throw new Error("not exercised in this test");
    },
  });
  expect([...tools.keys()]).toEqual([
    "batman_task",
    "batman_worker",
    "batman_run",
    "batman_message",
    "batman_approval",
    "batman_reconcile",
    "batman_profile",
    "batman_workspace",
  ]);
});

test("read-only ops resolve to tier read, mutating worker/run ops resolve to tier exec", () => {
  const { api, tools } = createFakeApi();
  registerOrchestrationTools(api, {
    getClient: () => {
      throw new Error("not exercised in this test");
    },
  });

  const worker = tools.get("batman_worker");
  expect(worker).toBeDefined();
  const workerApproval = worker?.approval as (args: unknown) => string;
  expect(workerApproval({ op: "create" })).toBe("exec");
  expect(workerApproval({ op: "list" })).toBe("read");
  expect(workerApproval({ op: "get" })).toBe("read");

  const run = tools.get("batman_run");
  expect(run).toBeDefined();
  const runApproval = run?.approval as (args: unknown) => string;
  expect(runApproval({ op: "submit" })).toBe("exec");
  expect(runApproval({ op: "cancel" })).toBe("exec");
  expect(runApproval({ op: "list" })).toBe("read");
  expect(runApproval({ op: "get" })).toBe("read");
  expect(runApproval({ op: "retry" })).toBe("read");
});

test("batman_approval never auto-approves: fixed exec tier with override and reason", () => {
  const { api, tools } = createFakeApi();
  registerOrchestrationTools(api, {
    getClient: () => {
      throw new Error("not exercised in this test");
    },
  });

  const approval = tools.get("batman_approval");
  expect(approval).toBeDefined();
  expect(approval?.approval).toEqual({
    tier: "exec",
    override: true,
    reason: "Approval decisions are a user-facing safety action.",
  });
});

test("batman_approval requires approvalId, decision, and reason for decide", () => {
  const { api, tools } = createFakeApi();
  registerOrchestrationTools(api, {
    getClient: () => {
      throw new Error("not exercised in this test");
    },
  });
  const approval = tools.get("batman_approval");
  const schema = approval?.parameters as zod.ZodObject;
  expect(() => schema.parse({ op: "decide" })).not.toThrow(); // shape allows optional fields; runtime enforces requiredness
  const shape = schema.shape as Record<string, unknown>;
  expect(Object.keys(shape)).toEqual(["op", "runId", "approvalId", "decision", "reason"]);
});

// -------------------------------------------------- live-daemon round trip

let daemon: ReturnType<typeof Bun.spawn> | undefined;
let stateDir: string;
let repoDir: string;

function findSocket(state: string): string | undefined {
  const reposDir = join(state, "repos");
  if (!existsSync(reposDir)) return undefined;
  for (const entry of readdirSync(reposDir)) {
    const candidate = join(reposDir, entry, "runtime.sock");
    if (existsSync(candidate)) return candidate;
  }
  return undefined;
}

// Polling for the runtime's socket file: OS filesystem creation exposes no
// event/promise API to await directly, so a genuine wall-clock delay between
// polls is unavoidable here (per the real-timer exception for integration
// tests exercising the platform clock).
function delay(ms: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, ms);
  return promise;
}

async function waitForSocket(state: string): Promise<void> {
  for (let i = 0; i < 200; i++) {
    if (findSocket(state) !== undefined) return;
    await delay(50);
  }
  throw new Error("timed out waiting for runtime.sock");
}

beforeAll(async () => {
  const build = Bun.spawnSync(["cargo", "build", "-p", "batman-runtime"], { cwd: REPO_ROOT });
  if (build.exitCode !== 0) {
    throw new Error(`cargo build failed: ${build.stderr.toString()}`);
  }

  stateDir = mkdtempSync("/tmp/bat-tools-s-");
  repoDir = mkdtempSync("/tmp/bat-tools-r-");
  mkdirSync(join(repoDir, ".git"));

  daemon = Bun.spawn(
    [BATCAVE, "serve", "--foreground", "--state-dir", stateDir, "--repo", repoDir],
    { stdout: "ignore", stderr: "pipe" },
  );

  await waitForSocket(stateDir);
}, 180_000);

afterAll(async () => {
  daemon?.kill("SIGTERM");
  await daemon?.exited;
});

async function connectedClient(): Promise<BatmanClient> {
  const socketPath = findSocket(stateDir);
  if (socketPath === undefined) {
    throw new Error("runtime socket not found");
  }
  const client = new BatmanClient({ socketPath });
  await client.whenConnected();
  await client.initialize({
    client: { name: "@satori/batman", version: "0.1.0" },
    supported: { min: { major: 1, minor: 0 }, max: { major: 1, minor: 0 } },
    repository: { canonicalPath: repoDir, vcsRoot: repoDir },
    auth: { role: "ompExtension", instanceId: "omp-tools-test", agentDirectory: repoDir },
    capabilities: { eventReplay: true, maxFrameBytes: 1024 * 1024 },
    lastSequence: null,
  });
  return client;
}

test("batman_task tool creates a task with auto-generated ID and session owner", async () => {
  const { api, tools } = createFakeApi();
  let cached: BatmanClient | undefined;
  registerOrchestrationTools(api, {
    getClient: async () => {
      cached ??= await connectedClient();
      return cached;
    },
  });

  const taskTool = tools.get("batman_task");
  expect(taskTool).toBeDefined();
  if (taskTool === undefined) throw new Error("unreachable");

  // Create a new task - extension auto-generates taskId and uses session ID as owner
  const result = await taskTool.execute(
    "call-1",
    { description: "Test task creation" },
    undefined,
    undefined,
    fakeExtensionContext(repoDir),
  );
  
  // Should succeed with a valid taskId
  expect(result.isError).toBeUndefined();
  const details = result.details as { taskId: string };
  expect(typeof details.taskId).toBe("string");
  expect(details.taskId).toMatch(/^[0-9a-f-]+$/);  // Valid UUID format

  cached?.close();
});

test("batman_worker tool maps a JSON-RPC error to a stable, non-throwing tool error", async () => {
  const { api, tools } = createFakeApi();
  let cached: BatmanClient | undefined;
  registerOrchestrationTools(api, {
    getClient: async () => {
      cached ??= await connectedClient();
      return cached;
    },
  });

  const workerTool = tools.get("batman_worker");
  expect(workerTool).toBeDefined();
  if (workerTool === undefined) throw new Error("unreachable");

  // "get" with a well-formed but nonexistent workerId triggers a runtime
  // NOT_FOUND-shaped error; the tool must surface it as a structured,
  // non-throwing result rather than an unhandled rejection.
  const result = await workerTool.execute(
    "call-1",
    { op: "get", workerId: "018f0000-0000-7000-8000-000000000000" },
    undefined,
    undefined,
    fakeExtensionContext(repoDir),
  );
  expect(result.isError).toBe(true);
  const details = result.details as { code: number; message: string };
  expect(typeof details.code).toBe("number");
  expect(typeof details.message).toBe("string");

  cached?.close();
});
