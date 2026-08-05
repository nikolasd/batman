import { expect, test } from "bun:test";
import { StateRootError, resolveStateRoot } from "./state";

interface StateRootCase {
  name: string;
  env: Record<string, string>;
  home: string;
  expected?: string;
  error?: string;
}

const cases = (await Bun.file("fixtures/state/state-root-cases.json").json()) as StateRootCase[];

test("shared fixture has at least one case", () => {
  expect(cases.length).toBeGreaterThan(0);
});

for (const testCase of cases) {
  test(`state root precedence: ${testCase.name}`, () => {
    if (testCase.expected !== undefined) {
      expect(resolveStateRoot(testCase.env, testCase.home)).toBe(testCase.expected);
    } else if (testCase.error !== undefined) {
      expect(() => resolveStateRoot(testCase.env, testCase.home)).toThrow(StateRootError);
    } else {
      throw new Error(`case ${testCase.name} must set exactly one of expected/error`);
    }
  });
}

test("rejects a relative BATMAN_STATE_DIR override", () => {
  expect(() => resolveStateRoot({ BATMAN_STATE_DIR: "relative/state" }, "/home/alice")).toThrow(StateRootError);
});

test("rejects a relative XDG_STATE_HOME override", () => {
  expect(() => resolveStateRoot({ XDG_STATE_HOME: "relative/state" }, "/home/alice")).toThrow(StateRootError);
});

test("BATMAN_STATE_DIR wins over XDG_STATE_HOME and the default", () => {
  const root = resolveStateRoot(
    {
      BATMAN_STATE_DIR: "/var/lib/batman",
      XDG_STATE_HOME: "/home/alice/.local/state",
    },
    "/home/alice",
  );
  expect(root).toBe("/var/lib/batman");
});

test("falls back to $HOME/.omp/orchestrator when nothing is set", () => {
  expect(resolveStateRoot({}, "/home/alice")).toBe("/home/alice/.omp/orchestrator");
});

test("PI_CONFIG_DIR overrides the default .omp directory name", () => {
  expect(resolveStateRoot({ PI_CONFIG_DIR: ".config-omp" }, "/home/alice")).toBe("/home/alice/.config-omp/orchestrator");
});

test("does not read process-global env or home", () => {
  const originalStateDir = process.env.BATMAN_STATE_DIR;
  process.env.BATMAN_STATE_DIR = "/should/not/be/read";
  try {
    expect(resolveStateRoot({}, "/home/alice")).toBe("/home/alice/.omp/orchestrator");
  } finally {
    if (originalStateDir === undefined) {
      delete process.env.BATMAN_STATE_DIR;
    } else {
      process.env.BATMAN_STATE_DIR = originalStateDir;
    }
  }
});
