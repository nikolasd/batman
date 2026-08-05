// Packaging/install invariants for the platform leaf packages.
//
// WHY THERE IS NO REAL REGISTRY HERE: publishing the six `@nikolasd`
// packages needs a running registry plus publish credentials, neither of
// which exists in this environment or in CI (the release workflow publishes
// from a tag, and a test that published would either need real credentials
// or a registry container this repo does not provision). So instead of a
// stub that asserts nothing, this exercises the invariants an install
// actually depends on, with no registry required:
//
//   1. the platform/arch/libc -> leaf package mapping, including the
//      unsupported combinations that must throw rather than guess;
//   2. `manifest.json`'s `sha256` is verified against the real binary bytes,
//      so a corrupted or substituted binary is refused;
//   3. `manifest.json`'s `version` must equal the extension's version, so a
//      leaf from a different release is refused;
//   4. `OMP_BATMAN_BINARY` overrides the resolver outright.
//
// Every case drives the real `resolveBatcave` through its own
// `resolveLeafDir` seam, against real files on disk.

import { describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { mkdtempSync, mkdirSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { BinaryIntegrityError, UnsupportedPlatformError, resolveBatcave } from "../../packages/extension/src/platform";

/** The version the extension package declares; leaves must match it. */
const EXTENSION_VERSION: string = require("../../packages/extension/package.json").version;

/**
 * Materializes a leaf package directory containing `bin/batcave` and a
 * `manifest.json`. `sha256`/`version` default to the honest values so a
 * test only has to state the field it is corrupting.
 */
function fakeLeaf(options: { contents?: string; sha256?: string; version?: string } = {}): string {
  const contents = options.contents ?? "#!/bin/sh\necho batcave\n";
  const dir = mkdtempSync(join(tmpdir(), "batman-leaf-"));
  mkdirSync(join(dir, "bin"), { recursive: true });
  const binPath = join(dir, "bin", "batcave");
  writeFileSync(binPath, contents);
  chmodSync(binPath, 0o755);

  const honestSha = createHash("sha256").update(contents).digest("hex");
  writeFileSync(
    join(dir, "manifest.json"),
    JSON.stringify({
      name: "@nikolasd/batman-darwin-arm64",
      version: options.version ?? EXTENSION_VERSION,
      target: "darwin-arm64",
      sha256: options.sha256 ?? honestSha,
      sizeBytes: Buffer.byteLength(contents),
    }),
  );
  return dir;
}

describe("leaf package selection", () => {
  test("each supported platform/arch/libc resolves to its own leaf package", () => {
    const requested: string[] = [];
    const deps = {
      resolveLeafDir: (packageName: string) => {
        requested.push(packageName);
        return fakeLeaf();
      },
    };

    for (const [platform, arch, libc, expected] of [
      ["darwin", "arm64", undefined, "@nikolasd/batman-darwin-arm64"],
      ["darwin", "x64", undefined, "@nikolasd/batman-darwin-x64"],
      ["linux", "arm64", "glibc", "@nikolasd/batman-linux-arm64-gnu"],
      ["linux", "x64", "glibc", "@nikolasd/batman-linux-x64-gnu"],
    ] as const) {
      requested.length = 0;
      const selected = resolveBatcave(platform, arch, libc, {}, deps);
      expect(requested).toEqual([expected]);
      expect(selected.source).toBe("package");
      expect(selected.path.endsWith(join("bin", "batcave"))).toBe(true);
    }
  });

  test("an unsupported platform, arch, or libc throws instead of guessing a leaf", () => {
    const deps = { resolveLeafDir: () => fakeLeaf() };
    // Windows is unsupported outright; musl is unsupported on Linux; an
    // unknown arch has no leaf. None may silently fall back to another.
    for (const [platform, arch, libc] of [
      ["win32", "x64", undefined],
      ["linux", "x64", "musl"],
      ["linux", "arm64", undefined],
      ["darwin", "riscv64", undefined],
    ] as const) {
      expect(() => resolveBatcave(platform, arch, libc, {}, deps)).toThrow(UnsupportedPlatformError);
    }
  });
});

describe("leaf package integrity", () => {
  test("a leaf whose binary matches its manifest checksum is accepted", () => {
    const dir = fakeLeaf();
    const selected = resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => dir });
    expect(selected).toEqual({ path: join(dir, "bin", "batcave"), source: "package" });
  });

  test("a binary that does not match its manifest checksum is refused", () => {
    // The manifest declares a checksum for different bytes than the ones on
    // disk -- a substituted or truncated binary.
    const dir = fakeLeaf({ sha256: "0".repeat(64) });
    let error: unknown;
    try {
      resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => dir });
    } catch (err) {
      error = err;
    }
    expect(error).toBeInstanceOf(BinaryIntegrityError);
    expect((error as BinaryIntegrityError).message).toContain("checksum mismatch");
  });

  test("a leaf from a different release is refused even when its checksum is honest", () => {
    const dir = fakeLeaf({ version: "0.0.1-not-this-release" });
    let error: unknown;
    try {
      resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => dir });
    } catch (err) {
      error = err;
    }
    expect(error).toBeInstanceOf(BinaryIntegrityError);
    expect((error as BinaryIntegrityError).message).toContain("version");
  });
});

describe("binary override", () => {
  test("a valid OMP_BATMAN_BINARY wins outright and never consults a leaf", () => {
    const dir = mkdtempSync(join(tmpdir(), "batman-override-"));
    const override = join(dir, "batcave");
    writeFileSync(override, "#!/bin/sh\nexit 0\n");
    chmodSync(override, 0o755);

    let leafConsulted = false;
    const selected = resolveBatcave(
      "darwin",
      "arm64",
      undefined,
      { OMP_BATMAN_BINARY: override },
      {
        resolveLeafDir: () => {
          leafConsulted = true;
          return fakeLeaf();
        },
      },
    );

    expect(selected).toEqual({ path: override, source: "override" });
    expect(leafConsulted).toBe(false);
  });
});
