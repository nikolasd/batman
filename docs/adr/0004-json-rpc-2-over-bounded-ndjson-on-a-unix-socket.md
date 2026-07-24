# JSON-RPC 2.0 over bounded NDJSON on a Unix socket

* Status: Accepted
* Date: 2026-07-23

## Context and Problem Statement

The extension and the daemon (ADR-0001) need a concrete wire protocol: request/response for calls,
plus server-initiated push for live events. The security boundary is "same machine, same user" —
what transport and framing satisfies that without pulling in machinery the boundary doesn't need?

## Decision Drivers

* Same-machine, same-user trust boundary — no network exposure should be possible even by mistake.
* Both sides already have mature JSON serialization (serde / native `JSON.parse`); no need for a
  binary format.
* Needs bidirectional traffic on one connection: request/response *and* server-push notifications
  (for live event delivery), without two separate channels.
* Must be debuggable by hand (a raw socket and a text editor) during development.

## Considered Options

* JSON-RPC 2.0 messages, newline-delimited (NDJSON), over a Unix domain socket.
* gRPC over a Unix domain socket.
* HTTP/1.1 with Server-Sent Events, over a local TCP port.
* A custom binary, length-prefixed protocol.

## Decision Outcome

Chosen option: JSON-RPC 2.0 over bounded NDJSON on a Unix domain socket. The socket is bound
owner-only (mode `0600`); peer UID is checked at `accept()` time before a single byte is parsed as
JSON. Frames are newline-delimited JSON with a 4 MiB bootstrap cap, negotiated down after
`initialize` to `min(client offer, runtime max)` with a 64 KiB protocol floor. JSON-RPC
notifications (no `id`) carry live event pushes on the same connection as request/response traffic.

### Positive Consequences

* No TCP port, ever — the entire attack surface is "can you connect to this Unix socket as this
  user," which the filesystem already answers.
* Requests, responses, and push notifications share one connection and one framing scheme; no
  second channel to keep in sync.
* Trivially debuggable: `printf '{"jsonrpc":"2.0",...}\n' | nc -U <socket>` works without any
  client library.

### Negative Consequences

* NDJSON assumes no embedded raw newlines in a frame's JSON text (guaranteed by using a real JSON
  serializer on both ends, but worth stating as an assumption).
* No built-in multiplexing or backpressure beyond what's hand-rolled: the writer side is fed by a
  bounded channel specifically so a burst of events can't interleave mid-frame with an RPC
  response, and both peers enforce the negotiated frame cap on every inbound and outbound frame.

## Pros and Cons of the Options

### JSON-RPC 2.0 over bounded NDJSON on a Unix socket (chosen)

* Good, because it needs no network stack, no TLS, and is trivial to hand-craft for debugging.
* Bad, because framing discipline (size caps, one frame per line) is the protocol's own
  responsibility rather than delegated to a mature framework.

### gRPC over a Unix socket

* Good, because it gives typed streaming, code-generated clients, and built-in multiplexing.
* Bad, because it pulls in Protobuf as a second schema language alongside the Rust/JSON Schema/TS
  triangle this project already has (ADR-0002), and its tooling is heavier than a same-machine
  IPC boundary needs.

### HTTP/1.1 + SSE over a local TCP port

* Good, because HTTP tooling (curl, browsers) is universally available for debugging.
* Bad, because a TCP port — even bound to loopback — is a materially larger attack surface than a
  Unix socket with peer-credential admission, and this project's constraint is explicit: no TCP
  listener.

### Custom binary protocol

* Good, because it could be more compact and faster to parse.
* Bad, because both languages already have fast, correct JSON support, and a bespoke binary format
  would need its own schema, its own generator, and its own debugging tools — solving a problem
  this project doesn't have.

## Links

* Narrated in `../journal.md`, commit `4ed1b14`
* Companion to [ADR-0009](0009-role-based-authorization-from-the-connection-not-per-call.md)
  (authorization over this same connection)
