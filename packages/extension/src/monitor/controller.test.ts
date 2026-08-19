// Tests for the monitor controller's session lifecycle: the widget must
// render immediately on session start (R56), and a `session_shutdown`
// followed by a new session must resubscribe rather than early-return into
// a dead monitor (R39). Both drive `registerMonitor` through a fake
// ExtensionAPI, mirroring tools.test.ts's fake-API pattern.

import { expect, test } from "bun:test";

import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";

import type { BatmanClient } from "../client";
import { registerMonitor } from "./controller";

type SessionHandler = (event: unknown, extCtx: ExtensionContext) => Promise<void>;

interface FakeHarness {
  readonly api: ExtensionAPI;
  readonly handlers: Map<string, SessionHandler>;
  readonly commands: Map<string, { handler: (args: string, ctx: ExtensionContext) => Promise<void> }>;
}

function createFakeApi(): FakeHarness {
  const handlers = new Map<string, SessionHandler>();
  const commands = new Map<string, { handler: (args: string, ctx: ExtensionContext) => Promise<void> }>();
  const api = {
    logger: { debug() {}, info() {}, warn() {}, error() {} },
    appendEntry() {},
    on(event: string, handler: SessionHandler) {
      handlers.set(event, handler);
    },
    registerCommand(name: string, options: { handler: (args: string, ctx: ExtensionContext) => Promise<void> }) {
      commands.set(name, options);
    },
  };
  return { api: api as unknown as ExtensionAPI, handlers, commands };
}

interface FakeClient {
  subscribeCalls: number;
  closed: boolean;
  client: BatmanClient;
}

function createFakeClient(): FakeClient {
  const fake: FakeClient = {
    subscribeCalls: 0,
    closed: false,
    client: undefined as unknown as BatmanClient,
  };
  fake.client = {
    get isClosed() {
      return fake.closed;
    },
    close() {
      fake.closed = true;
    },
    subscribe(_fromSequence: number, _onEvent: unknown) {
      fake.subscribeCalls += 1;
      return () => {};
    },
  } as unknown as BatmanClient;
  return fake;
}

function fakeTheme(): unknown {
  return {
    boxRound: {
      topLeft: "╭",
      topRight: "╮",
      bottomLeft: "╰",
      bottomRight: "╯",
      horizontal: "─",
      vertical: "│",
      cross: "┼",
      teeDown: "┬",
      teeUp: "┴",
      teeRight: "├",
      teeLeft: "┤",
    },
    fg: (_color: unknown, text: string) => text,
  };
}

function fakeExtensionContext(widgetCalls: unknown[][]): ExtensionContext {
  return {
    sessionManager: { getEntries: () => [] },
    ui: {
      theme: fakeTheme(),
      setWidget(...args: unknown[]) {
        widgetCalls.push(args);
      },
      notify() {},
    },
  } as unknown as ExtensionContext;
}

test("session_start renders the widget immediately, before any event arrives (R56)", async () => {
  const { api, handlers } = createFakeApi();
  const fake = createFakeClient();
  registerMonitor(api, { getClient: async () => fake.client });

  const widgetCalls: unknown[][] = [];
  const extCtx = fakeExtensionContext(widgetCalls);

  await handlers.get("session_start")?.(undefined, extCtx);

  // A healthy runtime with no runs must still render "No BATMAN runs yet."
  // instead of nothing at all.
  expect(fake.subscribeCalls).toBe(1);
  expect(widgetCalls.length).toBe(1);
});

test("a session_shutdown followed by a new session_start resubscribes instead of early-returning into a dead monitor (R39)", async () => {
  const { api, handlers } = createFakeApi();
  const fake = createFakeClient();
  registerMonitor(api, { getClient: async () => fake.client });

  const widgetCalls: unknown[][] = [];
  const extCtx = fakeExtensionContext(widgetCalls);

  await handlers.get("session_start")?.(undefined, extCtx);
  expect(fake.subscribeCalls).toBe(1);

  await handlers.get("session_shutdown")?.(undefined, extCtx);

  // The old client object is still open (isClosed === false); only the
  // subscription was torn down. A new session must resubscribe anyway.
  await handlers.get("session_start")?.(undefined, extCtx);
  expect(fake.subscribeCalls).toBe(2);
});

test("a closed client is repaired on the next connect even without the shutdown clear (production's index.ts close path)", async () => {
  // Production closes the cached client in its own session_shutdown handler
  // (index.ts), so connect()'s pre-existing repair branch (isClosed check)
  // fires regardless of R39's clear. This pins that path: even if the
  // subscribedClient reference survives, a closed client must be dropped
  // and resubscribed.
  const { api, handlers } = createFakeApi();
  const fake = createFakeClient();
  registerMonitor(api, { getClient: async () => fake.client });

  const widgetCalls: unknown[][] = [];
  const extCtx = fakeExtensionContext(widgetCalls);

  await handlers.get("session_start")?.(undefined, extCtx);
  expect(fake.subscribeCalls).toBe(1);

  // No session_shutdown here on purpose: only the client is closed, the
  // subscribedClient reference is still set, so the repair branch is the
  // only thing that can save the monitor.
  fake.client.close();

  await handlers.get("session_start")?.(undefined, extCtx);
  expect(fake.subscribeCalls).toBe(2);
});
