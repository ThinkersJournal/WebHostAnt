# Roadmap

Nothing is built. Order, set by the PM on CireSnave's proposal (2026-10-28):

1. Wait for the Humboldt installer's first slices to show what the interface needs (extract after two consumers).
2. (Drafted: `docs/DESIGN.md`, awaiting CireSnave decisions.) A short design note and a build-or-adopt comparison (Caddy and other existing deployment/TLS tools, ACME libraries).
3. Host adapters, two or three first; local serving with automatic TLS; ConfigurAnt integration as its own row.

## Spin-out ruling (CireSnave 2026-10-28, board 320)

Adapters are built here from the start; the Humboldt installer depends on this crate; Humboldt is the first consumer and goal-setter, nothing Humboldt-specific lives here. First: consumer contract, edge/agnosticism answer, contract types + conformance harness. See HANDOFF.md.

## Build order ruling (CireSnave 2026-10-28, verbatim)

*"I'm thinking WebHostAnt's local serving should wait until WebServAnt is available. Similarly, WebServAnt should wait until HTTPAnt is available. If we work it that way then we build the tools before we build what sits on top of them. WebHostAnt can still build the remote hosting portions before that."*

Order: HTTPAnt -> WebServAnt (`webservant`) -> WebHostAnt local serving/TLS. Now: **Cloudflare is the only first target** (a Docker server needs local serving or a TLS front, so it waits), contract types and conformance harness for the remote contract. The Caddy/edge interface stays; Caddy is an optional external adapter.

## Target, secrets and presence ruling (CireSnave 2026-10-28, verbatim)

CireSnave, 2026-10-28, first-hand, verbatim: *"I'm fine with Cloudflare as the first remote server. Secrets should be pushed. The 'is our app here' would likely be a configuration retrieval request ... it is needed for configuration."* and *"we delay local serving entirely until WebServAnt is available. Doesn't that negate the docker server?"*

So: first remote target Cloudflare only; secrets are pushed (names and values from SecretAnt at deploy; manual paste is the fallback); presence is host-side (the host's API with the host credential), then an authenticated configuration retrieval from the app, with **no unauthenticated tier** (CireSnave: *"why would an 'Is our app here?' respond at all to unauthenticated users?"*); see `docs/CONSUMER-CONTRACT.md` 1a. Open for CireSnave: D6 test account, D7 ConfigurAnt additions, D8 ReleaseAnt.
