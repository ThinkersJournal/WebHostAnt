# Edge, TLS and web-server agnosticism

**Design only. Nothing here is built.** Written 2026-10-28 by the WebHostAnt lane against WebHostAnt `origin/main` `de284d2`.
"Read" means read this session at the named ref; **UNVERIFIED** means recalled, not looked up (every claim about Caddy, nginx, Cloudflare,
ACME crates). Question from CireSnave, 2026-10-28 (board 320, first-hand): *"Caddy is a web server. Does it support our goals of web server
library agnosticism? If not, Caddy is the wrong choice there. ... Do we need a WebServAnt project that is a web server library agnostic web server?"*

## Hello-world first (what the consumer writes)

```rust
// Intended, not built. The app does not name Caddy, nginx or Cloudflare.
let host = webhostant::Host::from_config(&settings)?;          // target + edge chosen by ConfigurAnt settings
let plan = host.plan(&my_app_manifest)?;                       // inert: shows what would change
plan.apply()?.read_back()?;                                    // acts, then proves it by reading state
```

The person running it sets `edge = "caddy"` or `edge = "cloudflare"` in a settings file. The app code does not change.

## The short answer

**No: Caddy does not support web server *library* agnosticism, and it never was meant to.** Two different things are being called "web server":

1. **The library the app is written against** (axum, actix, warp ...). That is HTTPAnt's job: write the app once, pick the library by id.
2. **The edge in front of the app**: the thing on ports 80/443 that holds the certificate, speaks TLS and forwards plain HTTP to the app.

Caddy is an edge. It speaks HTTP to whatever app is behind it, so it works with an app written on *any* library, and in that narrow sense it does not
get in the way of agnosticism. But it is a separate program, not a library, so it cannot be "chosen by id" inside the app, and making it the *required*
edge would make every consumer depend on one outside program. So: **Caddy may be one edge adapter; it must never be the required one.**

## What I checked (and what I did not)

- **HTTPAnt is not the 0.0.0 placeholder the HANDOFF assumed, in the local checkout.** `C:\Projects\HTTPAnt` at `origin/main` `6e9c126` still
  *is* `web-server-abstraction` 1.0.2 (Cargo.toml read): an axum-default adapter layer with actix, rocket, warp, salvo and poem features marked "being
  updated", a `with_*_adapter()` builder and a mock adapter. The reshape into core/server/client (ROADMAP row, read) has not landed in that tree. The
  `httpant` crate name may be a separate placeholder on crates.io; I did not check the registry. So HTTPAnt is *the* answer to question 1, but it cannot be
  relied on yet, and WebHostAnt must not depend on it in the first slice.
- **lightbulb has a working in-process certificate layer.** `src/tls.rs` at `origin/main` (read, first 80 lines only) offers four sources:
  existing files, self-signed (`rcgen`), ACME (directory URL, domains, contact email, cert and cache dirs) and HTTP-only. I did not read the ACME
  half, its renewal, or whether it has been exercised against a real CA. Treat it as a design to learn from, not a proven component.
- **Not checked at all:** Caddy's licence, release history or Windows behaviour; `instant-acme` and `rustls-acme` maintenance; any nginx fact. These are
  the same open facts DESIGN.md section 13 lists, still open.

## The shape that answers the question

TLS/edge is a **capability interface** in WebHostAnt with several providers, like every other family tool (CapAnt's Yes/No/Unverified rows):

| Provider | What it is | Needs | Good for |
|---|---|---|---|
| **In-process** | The app's own listener gets its certificate from an ACME library (lightbulb's `tls.rs` is the pattern) | Port 443 or 80 reachable; an ACME crate | One binary, no outside program |
| **Caddy adapter** | We render a Caddy config and supervise the Caddy process | A pinned Caddy binary (UNVERIFIED licence) | A school server wanting the most-used path |
| **nginx adapter** | We render nginx config; certificates from a separate ACME client | UNVERIFIED | Sites that already run nginx |
| **Cloudflare edge** | The host terminates TLS; we only bind the name | A Cloudflare account | The no-technical-staff case (DESIGN.md 5.4) |
| **Existing certificate** | The person supplies files | Nothing from us | Corporate IT that issues certificates itself |

Interface (from DESIGN.md 5.3, unchanged): `ensure_certificate(names, challenge)`, `status()`, `renew_now()`, an event stream. A provider that fails its
conformance run is `Unverified` and refused. The consumer names a *need* ("automatic certificate for these names"); WebHostAnt picks a provider that
satisfies it.

## Is "WebServAnt" needed?

A standalone web server program that is library-agnostic *and* terminates TLS is exactly what falls out of HTTPAnt (agnostic server half) plus the
in-process provider above. So it is not a separate idea; it is an *artifact* of those two parts, and building it first would mean building both
before anyone needs it.

## Options for CireSnave

**A. (Recommended) Capability interface now; no WebServAnt.** WebHostAnt defines the edge interface; first providers are Cloudflare (Humboldt
already runs there) and Caddy (for the Docker server target); the in-process provider follows when a consumer wants one binary. Caddy is optional by
construction. WebServAnt stays an idea. *Costs:* two providers to keep honest; nothing for someone who wants one file and no Caddy until later.

**B. In-process first, Caddy later.** Build the ACME path (learning from lightbulb) as the first provider so the first release has no outside program.
*Costs:* we own renewal, storage, key handling and failure alerts ourselves, where certificate mistakes are most expensive; and it needs HTTPAnt or a
fixed server choice to be useful, which today are not ready.

**C. Create WebServAnt now** as its own project: an HTTPAnt-based server with in-process ACME that WebHostAnt then deploys like any app.
*Costs:* a new project depending on two unfinished ones (HTTPAnt reshape, an ACME choice); delays Humboldt, which needs only a working edge. Revisit
when HTTPAnt's server half and one in-process provider both exist; at that point it is a packaging step.

**Recommendation: A.** It answers "Caddy must not be required" with a mechanism rather than a promise, and defers WebServAnt until its two parts exist.
Needs from CireSnave: pick A, B or C. Needs from the PM: whether `httpant` crate and `web-server-abstraction` repo state above matches the roadmap's plan.
