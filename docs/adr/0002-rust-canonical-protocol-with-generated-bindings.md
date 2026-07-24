# Rust as the canonical protocol, with generated bindings

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

The Rust daemon and the TypeScript extension must agree on every wire type — every request,
result, event, and error. Hand-maintaining the same type twice, in two languages, guarantees drift
the moment either side changes without perfect discipline on both ends. Which side is the source
of truth, and how does the other side stay in sync without a human remembering to update it?

## Decision Drivers

* Zero tolerance for silent drift between the two languages' understanding of the wire.
* The Rust side needs strong static types for domain/state-machine logic regardless of the
  protocol question, so it already has to define these types somewhere.
* Need both a JSON Schema (for documentation/validation) and TypeScript types (for the extension)
  from the same definition.
* Generation must be deterministic and checkable in CI — "did the committed bindings drift from
  the Rust types" must be a yes/no question, not a diff someone eyeballs.

## Considered Options

* Hand-write both Rust and TypeScript types, and rely on shared test fixtures to catch drift after
  the fact.
* Define every wire type once in Rust; generate JSON Schema (via `schemars`) and TypeScript
  bindings (via `ts-rs`) from it; commit the generated output and check it in CI.
* Define types in TypeScript/Zod first; generate Rust types from that.

## Decision Outcome

Chosen option: Rust is canonical, with `schemars`/`ts-rs`-generated JSON Schema and TypeScript
bindings. `crates/xtask generate --check` regenerates into a temp directory and byte-compares
against the committed output; `bun run check` runs it, so drift fails CI immediately rather than
surfacing as a runtime mismatch weeks later.

### Positive Consequences

* Zero-drift by construction: a Rust type change that forgets to regenerate fails the build, not a
  code review.
* Rust remains authoritative for both the domain logic *and* the wire shape it produces — one
  definition, not two that must be kept in sync by hand.
* `deny_unknown_fields` on every type (enforced in `tests/wire_contract.rs`) means both sides
  reject unrecognized fields rather than silently ignoring them.

### Negative Consequences

* The TypeScript build now has a build-time dependency on the Rust toolchain being available to
  run codegen (not to *consume* generated files, only to regenerate them).
* `schemars`/`ts-rs` representation choices (e.g. adjacently-tagged enums as
  `{ "type": ..., "payload": ... }`) become permanent wire-format decisions the moment they're
  generated once and a fixture pins them.

## Pros and Cons of the Options

### Hand-write both, catch drift with fixtures

* Good, because it requires no codegen tooling.
* Bad, because it only catches drift that a fixture happens to exercise — anything untested drifts
  silently until it breaks in production.

### Rust canonical, generated bindings (chosen)

* Good, because drift becomes a build failure, not a runtime bug.
* Bad, because it adds a codegen step (`crates/xtask`) that must itself be correct and
  deterministic.

### TypeScript/Zod canonical, generate Rust

* Good, because it's closer to how many OMP extensions already think about schemas.
* Bad, because Rust's domain/state-machine logic (see ADR-0012) needs real enums and exhaustive
  `match` far more than the wire format needs Zod's runtime validation to be the source of truth —
  generating Rust *from* TypeScript would fight the language's own strengths.

## Links

* Narrated in `../journal.md`, commits `480d428` and `700380f`
* Depended on by every ADR that defines a new wire type (0011–0020)
