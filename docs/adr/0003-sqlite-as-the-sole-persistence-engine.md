# SQLite as the sole persistence engine

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

BATMAN needs durable, crash-safe storage per repository: an append-only event journal plus
queryable projections. The project's constraints explicitly rule out any networked or containerized
dependency. What should actually hold the data on disk?

## Decision Drivers

* Global constraint: no TCP listener, cloud service, Redis, PostgreSQL, or container dependency.
* One daemon per repository implies many small, independent stores, not one shared server.
* Durability must survive a process crash without a separate WAL/replication layer to operate.
* Operators (and this project's own tests) need to inspect state with a plain CLI tool, no server
  to stand up first.

## Considered Options

* SQLite, one file per repository, WAL mode.
* An embedded key-value store (e.g. `sled`, RocksDB) with a hand-rolled event-log format on top.
* A shared local PostgreSQL instance serving all repositories.

## Decision Outcome

Chosen option: SQLite, one file per repository, opened with
`journal_mode=WAL`, `foreign_keys=ON`, `synchronous=FULL`, `busy_timeout=5000`, migrated with
`rusqlite_migration`. This is the only option that needs zero operational setup, is trivially
inspectable (`sqlite3 runtime.db 'SELECT ...'`), and gives crash safety (WAL) without any
replication machinery this project doesn't need at its current scale.

### Positive Consequences

* Zero ops burden — no server process to install, configure, or keep alive besides `batcave`
  itself.
* Trivial debugging: any engineer can `sqlite3` into a repository's `runtime.db` and read exactly
  what's there.
* WAL plus `synchronous=FULL` gives durable-on-commit semantics without extra code.

### Negative Consequences

* SQLite wants one writer; this forces the single-thread actor pattern (see ADR-0005) rather than
  a connection pool with concurrent writers.
* No built-in replication or multi-node story if a hosted/shared mode is ever wanted — deferred
  explicitly, not designed around.

## Pros and Cons of the Options

### SQLite per repository (chosen)

* Good, because it needs no server, no network port, and is inspectable with a stock CLI tool.
* Bad, because every write serializes through one connection, one thread.

### Embedded KV store + hand-rolled log

* Good, because it could offer higher write throughput for pure log-append workloads.
* Bad, because "queryable projections" (tasks/workers/runs/messages/approvals tables with foreign
  keys) is exactly what a relational engine is for — reimplementing that on a KV store means
  reimplementing SQL, badly, by hand.

### Shared local PostgreSQL

* Good, because it removes the single-writer-thread constraint and gives real concurrent access.
* Bad, because it violates the no-external-service constraint outright and turns "one daemon per
  repository" into "one daemon per repository plus one shared database server everything depends
  on."

## Links

* Narrated in `../journal.md`, commit `8cd8ad8`
* Requires [ADR-0005](0005-single-thread-actor-owns-the-sqlite-connection.md)
