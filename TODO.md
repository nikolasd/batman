# BATMAN TODO

## Architecture Document Deferred Items (from docs/architecture.md §19)

### 1. Events table missing task_id/worker_id columns

**Status:** Open  
**Priority:** High  
**Labels:** bug, persistence, schema-migration

**Description:**
The `events` table still only stores `run_id`, not `task_id`/`worker_id`/`parent_worker_id`/`vendor_event_ref` (`source` is still hardcoded `runtime`). A *live* `events/event` notification's envelope carries `task_id`/`worker_id` (§11's `append_and_apply` sets them from its caller's parameters), but a *replayed* one from `events/replay` always has them `None` — `ipc/connection.rs::replay()` can only reconstruct an envelope from what the `events` table's columns hold.

The monitor (§17) is unaffected because it reads the inner `RuntimeEvent` variant's own `task_id`/`worker_id` fields (always present, part of the payload), never the outer envelope's convenience fields — but any future consumer that filters `events/replay` by the envelope's `task_id`/`worker_id` will get silently wrong (empty) results.

**Implementation:**
- Schema migration to add `task_id`, `worker_id`, `parent_worker_id`, `vendor_event_ref` columns to `events` table
- Update `append_and_apply` in `crates/runtime/src/domain/repository.rs` to populate these columns
- Update `replay()` in `crates/runtime/src/ipc/connection.rs` to use the new columns

**References:** `docs/architecture.md` §4, §11

---

### 2. Worker adapters implemented but authorization layer not wired

**Status:** Implemented (gated)  
**Priority:** Medium  
**Labels:** adapter, authorization, hardening

**Description:**
The `AdapterRegistry` exists and implements `RunDriver` against Claude/Codex/Copilot/OMP-RPC adapters. However, production `ServerConfig::default()` uses `DenyByDefaultAuthorization` until the Hardening plan's `PolicyEvaluator` is wired. The credential store for `workerMcp` connections is not yet implemented (`RejectAllWorkerVerifier` by default).

**Implementation:**
- Wire `PolicyEvaluator` into `ServerConfig`
- Implement credential store for `workerMcp` connections
- Replace `RejectAllWorkerVerifier` with real credential verification

**References:** `docs/architecture.md` §10, §15, ADR-0013

---

### 3. Redaction regex denylist expansion

**Status:** Open  
**Priority:** Low  
**Labels:** security, defense-in-depth

**Description:**
The redaction regex denylist is intentionally small (API-key/bearer shapes); classification is the primary boundary. Expanding the denylist (`ghp_`, `AKIA…`, JWT shapes) is planned defense-in-depth.

**Implementation:**
- Add regex patterns for GitHub personal access tokens (`ghp_`)
- Add regex patterns for AWS access key IDs (`AKIA…`)
- Add regex patterns for JWT shapes
- Update `crates/runtime/src/security/redaction.rs`

**References:** `docs/architecture.md` §5

---

### 4. Subscription forwarder reaping

**Status:** Open (low priority)  
**Priority:** Low  
**Labels:** cleanup, subscription

**Description:**
Subscription forwarder tasks for closed connections are reaped lazily on the next event broadcast; harmless in practice since a closed connection's own `events_rx.recv()` loop (`spawn_subscription`) exits on its own `Err` the next time anything is broadcast.

**Implementation:**
- Optional: add explicit reaping logic for closed connections
- Current behavior is acceptable; no fix needed

**References:** `docs/architecture.md` §6

---

### 5. Remote service integration

**Status:** Open  
**Priority:** Future  
**Labels:** future-milestone, remote-services

**Description:**
Remote service integration (cloud storage, external APIs) is explicitly out of scope for this milestone.

**Implementation:**
- Future milestone work
- No current action required

**References:** `docs/architecture.md` §19

---

## Feature Requests

### Org Config: URL or File Path Support

**Status:** Not Started  
**Priority:** Medium  
**Labels:** enhancement, configuration

**Description:**
Currently, org config is loaded only from file paths. This should be enhanced to support either:
- A file path (current behavior)
- A URL (HTTP/HTTPS) for remote configuration

**Implementation Notes:**
- Modify `crates/runtime/src/config/merge.rs` `load_layer` function
- Detect if the path is a URL (starts with `http://` or `https://`)
- If URL, fetch the content and parse as YAML
- If file path, load from disk (current behavior)
- Add appropriate error handling for network failures
- Consider caching fetched URLs to avoid repeated network calls

**Example Usage:**
```bash
# File path (current)
batman serve --org-config /etc/batman/org.yaml

# URL (new)
batman serve --org-config https://config.example.com/org.yaml
```

**Dependencies:**
- Network access for URL fetching
- TLS certificate validation for HTTPS
- Timeout handling for network requests

---

## Other Potential Features

- [ ] Add support for config templates
- [ ] Add config validation against schema before loading
- [ ] Add config versioning and migration support
- [ ] Add config encryption for sensitive values

---

## Adapter Implementation Gaps (from conformance test failures)

### 6. Claude adapter: missing lifecycle/usage/result event extraction

**Status:** Open  
**Priority:** Medium  
**Labels:** adapter, claude, conformance

**Description:**
The Claude adapter's normalizer reads `initialize.jsonl` (a real Claude CLI session's stdout containing session start, streaming text, tool calls, and final result with usage/cost data) but does not extract three critical signals:
- `VendorSessionEstablished` — proves which session is being tracked
- `UsageReported` — proves token consumption and cost data
- `MessageFinal(role="result")` — proves the turn is complete

Without these, the runtime cannot track session lifecycle, report usage to OMP, or know when a turn is truly complete.

**Implementation:**
- Enhance `ClaudeNormalizer` to extract `VendorSessionEstablished` from the initialize frame
- Extract `UsageReported` from the result frame's usage data (input/output tokens, cost)
- Extract `MessageFinal(role="result")` from the result frame's text content
- Ensure all three correlate to the same session ID

**References:** `fixtures/adapters/claude/initialize.jsonl`, `crates/runtime/src/adapter/claude/normalize.rs`

---

### 7. Codex adapter: missing lifecycle/usage/artifact event extraction

**Status:** Open  
**Priority:** Medium  
**Labels:** adapter, codex, conformance

**Description:**
The Codex adapter's normalizer reads `thread-turn.jsonl` (a real Codex thread transcript containing text chunks, tool calls, token usage, and file change artifacts) but does not extract six critical event types:
- `MessageChunk` — streaming text chunks
- `MessageFinal` — final message
- `ToolStarted` — when a command begins
- `ToolResult` — when a command finishes
- `UsageReported` — token counts
- `ArtifactProduced` — file changes

Without these, OMP cannot track costs, know which files were changed by the worker, or correlate the full turn lifecycle.

**Implementation:**
- Enhance `CodexNormalizer` to extract all six event types from the thread transcript
- Ensure all events correlate to the same thread/run/task/worker IDs
- Verify the hidden `reasoning` content is still dropped before reaching visible events

**References:** `fixtures/adapters/codex/thread-turn.jsonl`, `crates/runtime/src/adapter/codex/normalize.rs`

---

### 8. Copilot adapter: ACP v1 protocol limitation on usage reporting

**Status:** Known gap (protocol limitation)  
**Priority:** Low  
**Labels:** adapter, copilot, protocol

**Description:**
ACP v1 does not transmit token usage or cost information in its session update frames. The Copilot adapter honestly declares `usage: none` rather than pretending to report something it cannot see. This is a protocol limitation, not a code bug.

**What can be done:**
- Wait for ACP v2 (or future protocol version) that transmits usage data
- No code changes possible until the protocol evolves

**References:** `fixtures/adapters/copilot/session-updates.jsonl`, `crates/runtime/src/adapter/copilot/client.rs`

---

### 9. Copilot live test: requires authenticated CLI session

**Status:** Environment dependency  
**Priority:** Low  
**Labels:** testing, environment

**Description:**
The `real_binary_initialize_and_session_list_never_invoke_a_model` test requires a real, authenticated Copilot CLI session. Without one, the test cannot run — that's expected and documented. This is not a code gap; it's an environment gap.

**What can be done:**
- Run tests with `BATMAN_LIVE_COPILOT=1` and a valid Copilot session
- No code changes needed

**References:** `docs/manual-testing.md` §4c
