# Roadmap

Nothing is built. Order, set by the PM on CireSnave's proposal (2026-10-28):

1. Wait for the Humboldt installer's first slices to show what the interface needs (extract after two consumers).
2. (Drafted: `docs/DESIGN.md`, awaiting CireSnave decisions.) A short design note and a build-or-adopt comparison (Caddy and other existing deployment/TLS tools, ACME libraries).
3. Host adapters, two or three first; local serving with automatic TLS; ConfigurAnt integration as its own row.

## Spin-out ruling (CireSnave 2026-10-28, board 320)

Adapters are built here from the start; the Humboldt installer depends on this crate; Humboldt is the first consumer and goal-setter, nothing Humboldt-specific lives here. First: consumer contract, edge/agnosticism answer, contract types + conformance harness. See HANDOFF.md.
