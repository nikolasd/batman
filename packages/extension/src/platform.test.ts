import { describe, expect, test } from "bun:test";
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { sha256File } from "./integrity";
import { BinaryIntegrityError, resolveBatcave, UnsupportedPlatformError } from "./platform";

import pkg from "../package.json" with { type: "json" };

const EXTENSION_VERSION: string = pkg.version;

/** A `resolveLeafDir` stand-in that fails the test if ever invoked. */
const NEVER_RESOLVE_LEAF_DIR = (packageName: string): string => {
  throw new Error(`resolveLeafDir must not be called, but was called with ${packageName}`);
};

interface LeafFixtureOptions {
  readonly binaryBytes?: Buffer;
  readonly sha256?: string;
  readonly version?: string;
  readonly target?: string;
}

/**
 * Builds a fixture leaf package directory (bin/batcave + manifest.json) in a
 * fresh temp directory, so integrity tests never depend on a real committed
 * binary.
 */
function makeLeaf(options: LeafFixtureOptions = {}): string {
  const dir = mkdtempSync(join(tmpdir(), "bat-leaf-"));
  mkdirSync(join(dir, "bin"));
  const binaryBytes = options.binaryBytes ?? Buffer.from("fake-batcave-binary-fixture-bytes");
  const binPath = join(dir, "bin", "batcave");
  writeFileSync(binPath, binaryBytes);
  chmodSync(binPath, 0o755);

  const manifest = {
    name: "@satori/batman-fixture",
    version: options.version ?? EXTENSION_VERSION,
    target: options.target ?? "fixture-target",
    sha256: options.sha256 ?? sha256File(binPath),
    sizeBytes: binaryBytes.length,
  };
  writeFileSync(join(dir, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`);
  return dir;
}

describe("resolveBatcave: tuple mapping", () => {
  test("darwin/arm64 maps to @satori/batman-darwin-arm64", () => {
    const leaf = makeLeaf();
    let requested: string | undefined;
    const result = resolveBatcave("darwin", "arm64", undefined, {}, {
      resolveLeafDir: (packageName) => {
        requested = packageName;
        return leaf;
      },
    });
    expect(requested).toBe("@satori/batman-darwin-arm64");
    expect(result).toEqual({ path: join(leaf, "bin", "batcave"), source: "package" });
  });

  test("darwin/x64 maps to @satori/batman-darwin-x64", () => {
    const leaf = makeLeaf();
    let requested: string | undefined;
    const result = resolveBatcave("darwin", "x64", undefined, {}, {
      resolveLeafDir: (packageName) => {
        requested = packageName;
        return leaf;
      },
    });
    expect(requested).toBe("@satori/batman-darwin-x64");
    expect(result).toEqual({ path: join(leaf, "bin", "batcave"), source: "package" });
  });

  test("linux/arm64/glibc maps to @satori/batman-linux-arm64-gnu", () => {
    const leaf = makeLeaf();
    let requested: string | undefined;
    const result = resolveBatcave("linux", "arm64", "glibc", {}, {
      resolveLeafDir: (packageName) => {
        requested = packageName;
        return leaf;
      },
    });
    expect(requested).toBe("@satori/batman-linux-arm64-gnu");
    expect(result).toEqual({ path: join(leaf, "bin", "batcave"), source: "package" });
  });

  test("linux/x64/glibc maps to @satori/batman-linux-x64-gnu", () => {
    const leaf = makeLeaf();
    let requested: string | undefined;
    const result = resolveBatcave("linux", "x64", "glibc", {}, {
      resolveLeafDir: (packageName) => {
        requested = packageName;
        return leaf;
      },
    });
    expect(requested).toBe("@satori/batman-linux-x64-gnu");
    expect(result).toEqual({ path: join(leaf, "bin", "batcave"), source: "package" });
  });
});

describe("resolveBatcave: unsupported platforms", () => {
  test("win32/x64 throws UnsupportedPlatformError with the exact platform/arch/libc", () => {
    expect(() =>
      resolveBatcave("win32", "x64", undefined, {}, { resolveLeafDir: NEVER_RESOLVE_LEAF_DIR }),
    ).toThrow(UnsupportedPlatformError);

    try {
      resolveBatcave("win32", "x64", undefined, {}, { resolveLeafDir: NEVER_RESOLVE_LEAF_DIR });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(UnsupportedPlatformError);
      const unsupported = err as UnsupportedPlatformError;
      expect(unsupported.platform).toBe("win32");
      expect(unsupported.arch).toBe("x64");
      expect(unsupported.libc).toBeUndefined();
    }
  });

  test("win32/arm64 throws UnsupportedPlatformError with the exact platform/arch/libc", () => {
    try {
      resolveBatcave("win32", "arm64", undefined, {}, { resolveLeafDir: NEVER_RESOLVE_LEAF_DIR });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(UnsupportedPlatformError);
      const unsupported = err as UnsupportedPlatformError;
      expect(unsupported.platform).toBe("win32");
      expect(unsupported.arch).toBe("arm64");
      expect(unsupported.libc).toBeUndefined();
    }
  });

  test("linux/x64/musl throws UnsupportedPlatformError with the exact platform/arch/libc", () => {
    try {
      resolveBatcave("linux", "x64", "musl", {}, { resolveLeafDir: NEVER_RESOLVE_LEAF_DIR });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(UnsupportedPlatformError);
      const unsupported = err as UnsupportedPlatformError;
      expect(unsupported.platform).toBe("linux");
      expect(unsupported.arch).toBe("x64");
      expect(unsupported.libc).toBe("musl");
    }
  });

  test("linux/arm64/musl throws UnsupportedPlatformError with the exact platform/arch/libc", () => {
    try {
      resolveBatcave("linux", "arm64", "musl", {}, { resolveLeafDir: NEVER_RESOLVE_LEAF_DIR });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(UnsupportedPlatformError);
      const unsupported = err as UnsupportedPlatformError;
      expect(unsupported.platform).toBe("linux");
      expect(unsupported.arch).toBe("arm64");
      expect(unsupported.libc).toBe("musl");
    }
  });
});

describe("resolveBatcave: integrity", () => {
  test("resolves to source package when the checksum and version match", () => {
    const leaf = makeLeaf();
    const result = resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => leaf });
    expect(result).toEqual({ path: join(leaf, "bin", "batcave"), source: "package" });
  });

  test("flipping one byte of the binary causes BinaryIntegrityError before spawn", () => {
    const leaf = makeLeaf();
    const binPath = join(leaf, "bin", "batcave");
    const bytes = readFileSync(binPath);
    bytes[0] = (bytes[0]! ^ 0xff) & 0xff;
    writeFileSync(binPath, bytes);

    expect(() =>
      resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => leaf }),
    ).toThrow(BinaryIntegrityError);

    try {
      resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => leaf });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(BinaryIntegrityError);
      expect((err as BinaryIntegrityError).code).toBe("checksum-mismatch");
    }
  });

  test("a manifest version that does not match the extension version fails", () => {
    const leaf = makeLeaf({ version: "0.0.1-does-not-match" });

    try {
      resolveBatcave("darwin", "arm64", undefined, {}, { resolveLeafDir: () => leaf });
      throw new Error("expected resolveBatcave to throw");
    } catch (err) {
      expect(err).toBeInstanceOf(BinaryIntegrityError);
      expect((err as BinaryIntegrityError).code).toBe("version-mismatch");
    }
  });
});

describe("resolveBatcave: override precedence", () => {
  test("a valid absolute executable override wins before tuple mapping, source override, no checksum performed", () => {
    // A deliberately corrupt manifest (wrong sha256) that would fail
    // integrity validation if the package path were ever consulted.
    const leaf = makeLeaf({ sha256: "0".repeat(64) });
    let leafDirRequested = false;

    const overrideDir = mkdtempSync(join(tmpdir(), "bat-override-"));
    const overridePath = join(overrideDir, "batcave");
    writeFileSync(overridePath, "#!/bin/sh\nexit 0\n");
    chmodSync(overridePath, 0o755);

    const result = resolveBatcave(
      "darwin",
      "arm64",
      undefined,
      { OMP_BATMAN_BINARY: overridePath },
      {
        resolveLeafDir: () => {
          leafDirRequested = true;
          return leaf;
        },
      },
    );

    expect(result).toEqual({ path: overridePath, source: "override" });
    // Proves the (corrupt) package manifest was never consulted.
    expect(leafDirRequested).toBe(false);
  });
});
