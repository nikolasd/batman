// This package ships a prebuilt platform-specific `batcave` binary; it has
// no runtime API of its own. `platform.ts`'s `resolveBatcave` resolves
// `bin/batcave` and `manifest.json` relative to this package's directory via
// `import.meta.resolve("@nikolasd/batman-darwin-arm64/package.json")`.
export {};
