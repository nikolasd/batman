# BATMAN TODO

The single source of truth for this project's implementation gaps, verified against the
current codebase rather than inferred from planning documents.

**Pruned 2026-08-06.** Every item previously marked fully `✅ Closed`/`Complete` was removed —
their resolutions are preserved in git history and `docs/journal.md`. Items closed by being
**moved** rather than resolved (Copilot usage reporting, Copilot nested-worker observation,
Org Config URL support, and the four config-backlog ideas) now live in
`docs/future-features.md` with a decision trigger each. Only genuinely open or
partially-closed items remain below.

---

## Low / Environment / Permanent

### 55. Codex/Copilot: several capabilities are unprovable in fixture mode — not a bug, requires a gated live run to confirm the positive case

**Status:** ⚠️ Partially closed 2026-08-05 — split by cause, with per-scenario evidence.
- **Codex (4 scenarios + `result_usage_artifacts`): blocked on account credits, not code.** `follow_up`, `cancellation_scope`, `session_resume`, `runtime_restart`, and `result_usage_artifacts` all fail with one vendor cause: `usageLimitExceeded: Your workspace is out of credits.` `codex login status` reports `Logged in using ChatGPT`; `initialize`/`thread/start` succeed; the turn is refused server-side after ~3s. Refill and they become provable with no code change. Report: `release/live-codex.json`.
- **Adapter fix this run DID produce:** the `error` notification was previously dropped by `codex/normalize.rs`, so this appeared as an unexplained `never produced a MessageFinal within 60s`. It now normalizes to `ProtocolHealthChanged{healthy:false}` and the live probe fails fast with the vendor's own text (62s → 5s). Defended by `a_vendor_error_notification_normalizes_to_an_unhealthy_protocol_event`.
- **Copilot `session_resume`/`runtime_restart`: a genuine ACP v1 protocol wall.** A session that completed a real turn cannot be reloaded from a new process — `session/load` answers `Resource not found: Session <id> not found`. Recorded as a CLI limitation, distinct from the Codex account condition.
**Priority:** Low
**Labels:** adapter, conformance, environment

**Description:**
Both are real properties of the installed vendor binary, not bugs in this codebase. Fixture-mode conformance must never spend a real, billed model call by design, so these can only be proven positively via `BATMAN_LIVE_<ADAPTER>=1` (see `docs/manual-testing.md` §4c):
- **Codex: `follow_up`, `session_resume`, `runtime_restart`, `cancellation_scope` (`CancelScope::Turn`)** — the installed `codex-cli` does not write a thread's resumable rollout file to disk until a turn actually runs; a bare `thread/start` with no turn leaves no rollout at all, so `Adapter::resume()` against such a thread fails with a real vendor error. `turn/start` is exactly what invokes the model. See `crates/runtime/src/adapter/codex/conformance.rs`'s `unprovable_without_a_live_turn` helper and `live_report()`.
- **Copilot: `session_resume`, `runtime_restart`** — the installed CLI does not persist a freshly-created, never-prompted session in a form a brand-new process can reach via `session/load` alone; empirically confirmed via a real cross-process probe (`crates/runtime/src/adapter/copilot/conformance.rs::session_resume_probe`). A future CLI version might persist it without a turn; the check is written to pass automatically if that ever changes.

**Implementation:**
- No code change needed. Run `BATMAN_LIVE_CODEX=1`/`BATMAN_LIVE_COPILOT=1` conformance to prove these for real when a licensed, billed run is acceptable

**References:** `crates/runtime/src/adapter/codex/conformance.rs`, `crates/runtime/src/adapter/copilot/conformance.rs`, `docs/manual-testing.md` §4c
