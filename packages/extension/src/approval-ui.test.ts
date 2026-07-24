import { expect, test } from "bun:test";

import type { ExtensionUIContext } from "@oh-my-pi/pi-coding-agent";

import { renderApprovalMessage, showApprovalDialog, type PendingApproval } from "./approval-ui";

function fakeUi(overrides: Partial<ExtensionUIContext> = {}): ExtensionUIContext {
  return {
    select: async () => undefined,
    confirm: async () => false,
    input: async () => undefined,
    notify: () => {},
    onTerminalInput: () => () => {},
    setStatus: () => {},
    setWorkingMessage: () => {},
    setWidget: () => {},
    setFooter: () => {},
    setHeader: () => {},
    setTitle: () => {},
    custom: async () => {
      throw new Error("not exercised");
    },
    setEditorText: () => {},
    pasteToEditor: () => {},
    getEditorText: () => "",
    editor: async () => undefined,
    addAutocompleteProvider: () => {},
    setEditorComponent: () => {},
    theme: {} as ExtensionUIContext["theme"],
    getAllThemes: async () => [],
    getTheme: async () => undefined,
    setTheme: async () => ({ success: true }),
    getToolsExpanded: () => false,
    setToolsExpanded: () => {},
    ...overrides,
  } as ExtensionUIContext;
}

const BASE_APPROVAL: PendingApproval = {
  approvalId: "approval-1",
  workerId: "worker-1",
  action: "write file",
  arguments: { path: "/tmp/x", apiKey: "sk-should-never-render" },
  policyReason: "write requires human approval",
  humanRequired: true,
};

test("does not prompt when humanRequired is false", async () => {
  let selectCalled = false;
  const ui = fakeUi({
    select: async () => {
      selectCalled = true;
      return "Approve";
    },
  });

  const result = await showApprovalDialog(ui, { ...BASE_APPROVAL, humanRequired: false });

  expect(result).toBeUndefined();
  expect(selectCalled).toBe(false);
});

test("returns an approve decision with the collected reason", async () => {
  const ui = fakeUi({
    select: async () => "Approve",
    input: async () => "looks safe",
  });

  const result = await showApprovalDialog(ui, BASE_APPROVAL);

  expect(result).toEqual({ decision: "approve", reason: "looks safe" });
});

test("returns a deny decision with the collected reason", async () => {
  const ui = fakeUi({
    select: async () => "Deny",
    input: async () => "not safe",
  });

  const result = await showApprovalDialog(ui, BASE_APPROVAL);

  expect(result).toEqual({ decision: "deny", reason: "not safe" });
});

test("a selection timeout returns no decision and leaves the request pending", async () => {
  const ui = fakeUi({
    select: async () => undefined, // simulates the dialog timing out
  });

  const result = await showApprovalDialog(ui, BASE_APPROVAL);

  expect(result).toBeUndefined();
});

test("a reason-input timeout after selection also returns no decision", async () => {
  const ui = fakeUi({
    select: async () => "Approve",
    input: async () => undefined, // simulates the reason prompt timing out
  });

  const result = await showApprovalDialog(ui, BASE_APPROVAL);

  expect(result).toBeUndefined();
});

test("the dialog notifies with worker, action, policy reason, and approval id", async () => {
  let notified = "";
  const ui = fakeUi({
    notify: (message) => {
      notified = message;
    },
    select: async () => undefined,
  });

  await showApprovalDialog(ui, BASE_APPROVAL);

  expect(notified).toContain("Approval ID: approval-1");
  expect(notified).toContain("Worker: worker-1");
  expect(notified).toContain("Action: write file");
  expect(notified).toContain("Policy reason: write requires human approval");
});

test("renderApprovalMessage redacts secret-looking argument keys", () => {
  const message = renderApprovalMessage(BASE_APPROVAL);

  expect(message).toContain("\"path\":\"/tmp/x\"");
  expect(message).not.toContain("sk-should-never-render");
  expect(message).toContain("\"apiKey\":\"<redacted>\"");
});

test("renderApprovalMessage omits the worker line when no workerId is known", () => {
  const message = renderApprovalMessage({ ...BASE_APPROVAL, workerId: undefined });

  expect(message).not.toContain("Worker:");
});
