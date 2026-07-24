# Flat `op` discriminator over Zod discriminated unions

* Status: Accepted
* Date: 2026-07-24

## Context and Problem Statement

Each orchestration tool (`batman_task`, `batman_worker`, ...) needs parameters that vary in shape
by operation (`upsert` vs `get`, `create` vs `list` vs `get`, and so on). Zod's
`z.discriminatedUnion` is the idiomatic way to model "one of several shapes, tagged by a field" —
but combining it with this codebase's existing generic `ToolDefinition<TParams>` type triggered a
real TypeScript compiler limit ("excessively deep instantiation"), not a bug in the code itself.
How should tool parameters vary by operation without breaking the type checker?

## Decision Drivers

* The type checker must actually pass — a schema that's more idiomatic but doesn't compile isn't
  usable.
* Should not require inventing a second, bespoke tool-registration pattern just for this one
  problem; the fix should fit the existing `ToolDefinition<TParams>` convention.
* Must not weaken the actual validation a model-supplied `op` value receives.

## Considered Options

* A flat Zod object with an `op: z.enum([...])` field and every operation's optional fields
  declared alongside it; the `execute` body dispatches on `op` with a runtime `if`/`switch` and
  reads only the fields relevant to that branch.
* Keep `z.discriminatedUnion`, and either work around or simply accept the compiler limit (slow or
  failing typecheck) as a cost of the more idiomatic schema.
* Split each operation into its own tool (`batman_task_upsert`, `batman_task_get`, ...), avoiding
  any per-tool union entirely.

## Decision Outcome

Chosen option: the flat `op` field with runtime dispatch. It typechecks cleanly against the
existing generic tool-registration type, and from the model's perspective the result is identical
to a discriminated union — one tool, one schema, one `op` field selecting behavior — just
represented as a flat object with optional fields instead of a tagged union type.

### Positive Consequences

* No behavior change visible to a model calling the tool; the fix is entirely internal to how the
  TypeScript types are expressed.
* No proliferation of near-identical tools, and no new tool-registration pattern to maintain
  alongside the existing one.
* Six tools shipped against this pattern in the same commit with zero further compiler friction.

### Negative Consequences

* Zod doesn't enforce "only this op's relevant fields may be present" as strictly as a real
  discriminated union would — an irrelevant field is simply optional and silently ignored rather
  than rejected. Each tool's `execute` body compensates by checking only the fields its own branch
  needs and validating them explicitly (e.g., `batman_task`'s `op === "get"` branch never reads
  `revision`, regardless of whether the model supplied one).

## Pros and Cons of the Options

### Flat `op` + runtime dispatch (chosen)

* Good, because it compiles cleanly and needs no new pattern.
* Bad, because it is a slightly weaker input-shape guarantee than a true discriminated union.

### Keep the discriminated union, accept the compiler cost

* Good, because it would be the textbook-idiomatic Zod shape.
* Bad, because "excessively deep instantiation" is not a stylistic complaint — it can mean the
  type checker genuinely fails or times out, which is not a cost worth paying for stylistic
  purity.

### One tool per operation

* Good, because each tool's schema would be maximally precise.
* Bad, because it multiplies the number of tools a model has to choose between for what is
  conceptually one capability (task management), and duplicates the shared plumbing
  (`callOrchestration`) across more registration call sites for no real benefit.

## Links

* Narrated in `../journal.md`, commit `16f9a23`
