# Roadmap

Nothing is built. Order, set by the PM on CireSnave's proposal (2026-10-28):

1. Wait for the Humboldt installer's first slices to show what the interface needs (extract after two consumers).
2. (Drafted: `docs/DESIGN.md`, awaiting CireSnave decisions.) A short design note and a build-or-adopt comparison (Caddy and other existing deployment/TLS tools, ACME libraries).
3. Host adapters, two or three first; local serving with automatic TLS; ConfigurAnt integration as its own row.

## Spin-out ruling (CireSnave 2026-10-28, board 320)

Adapters are built here from the start; the Humboldt installer depends on this crate; Humboldt is the first consumer and goal-setter, nothing Humboldt-specific lives here. First: consumer contract, edge/agnosticism answer, contract types + conformance harness. See HANDOFF.md.

## Build order ruling (CireSnave 2026-10-28, verbatim)

*"I'm thinking WebHostAnt's local serving should wait until WebServAnt is available. Similarly, WebServAnt should wait until HTTPAnt is available. If we work it that way then we build the tools before we build what sits on top of them. WebHostAnt can still build the remote hosting portions before that."*

Order: HTTPAnt -> WebServAnt (`webservant`) -> WebHostAnt local serving/TLS. Now: remote adapters only (Cloudflare, remote Docker/server), contract types and conformance harness for the remote contract. The Caddy/edge interface stays; Caddy is an optional external adapter.
