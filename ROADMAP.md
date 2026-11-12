# Roadmap

Built at 0.0.31 (2026-11-08): the contract types and conformance harness, a Cloudflare Worker target (wrangler), Cloudflare domain binding and `ReadTls` over REST, a Render target, the HTTPAnt health probe, ConfigurAnt projection. Every real-account shape is still UNVERIFIED (D6). Original order, set by the PM on CireSnave's proposal (2026-10-28):

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

## Cloudflare path (PM task 2026-11-08; CireSnave 2026-11-08 verbatim: *"Yes, go ahead and get WebHostAnt started on the Cloudflare path."*)

Requirements come from Humboldt's by-hand demo deploy of 2026-11-08 (`.lane-state/humboldt-demo-deploy*.ps1/.out`): install the database, `wrangler secret put` each secret, delete
secrets the app must not have, `wrangler deploy` (a Worker plus a container), poll `/health` until the container is up (about 10 minutes allowed), smoke checks. What went wrong by
hand, and the row that fixes it:

- A deploy KEEPS secrets that were not deleted, and the adapter cannot delete one: row C1.
- Secret values reached the script through the process environment (`with-secret`), with no `Secrets` source for that: row C2.
- A PowerShell script read an npm notice on stderr as an error; the adapter must trust the exit code only, and find `wrangler` behind its `.cmd` shim: row C3.
- The container needs minutes after `deploy` before `/health` answers: row C3.
- A token without the Containers scope fails late; scopes beyond Pages, Workers and Containers are unchecked: row C4.
- Worker secrets are write-only, so existence is checked by name (already so in `secret_names`).

One PR per row, red first, mutations listed in each PR. Rows C1 to C6 are buildable now against fakes; none needs a decision from CireSnave.

| Row | Slice | Needs |
|---|---|---|
| C1 | `Op::DeleteSecret` and `Step::DeleteSecret`: the manifest names secrets that must be absent; planning compares with the observed names; Cloudflare runs `wrangler secret delete`; plan text and digest show it | nothing |
| C2 | `EnvSecrets`: a `Secrets` source that reads named environment variables (what `with-secret` exports); a value never reaches `Debug`, errors or the log | nothing |
| C3 | Deploy hardening: success from the exit code only, the real `wrangler` found on Windows, and a readiness wait (`HealthProbe` retried until a deadline, each answer reported) after a container deploy | nothing |
| C4 | Read-only preflight: `wrangler` and Docker present and recent enough, login present, token scopes that the deploy needs named as present or missing (names only) | nothing |
| C5 | A small runnable example (`examples/`): plan, show, approve the digest, apply, read back, against any Worker project directory; run by hand under `docs/RELEASE-CHECKLIST.md` on a throwaway Worker to close the UNVERIFIED shapes | CireSnave's wrangler login or the D6 account, only for the by-hand run |
| C6 | `Restore` for Cloudflare from the previous version id; states plainly what a container rollback does and does not undo | by-hand run of C5 for the shape |
| C7a | `secretant` feature: `SecretAntSecrets`, a `Secrets` source over a getter returning SecretAnt's `SecretValue` (non-UTF-8 and empty give no value; never printed). DONE in 0.0.49 | nothing |
| C7b | A source over SecretAnt's store (get by name, listing, audit) | SecretAnt row 1 slice 2 (the store) |
| C9 | Self-hosted-runner row: the real-target tests (what `examples/deploy.rs` did by hand: create, setting, secret, delete-secret, restore, then delete the Worker and check the account's Workers are unchanged) run in CI against the D6 test account. The account id comes from the environment (`CLOUDFLARE_ACCOUNT_ID`), never a file, so CI can substitute; the token comes from the secrets store (PM's `with-secret`, env `CLOUDFLARE_API_TOKEN`); throwaway Worker names only, cleanup always, a leftover Worker fails the run. Cargo feature or test gate that is NOT `#[ignore]` (portfolio rule 5.1g/5.1h) | the PM: runner label and secret delivery; [ASK] before any run that needs the vault |
| C8 | Local serving and TLS, split into C8a to C8e below (WebServAnt 0.4.4 published 2026-11-10) | WebServAnt published |
| C8a | `local` feature (optional `webservant` 0.4.4 dependency) and the read-back: map `RunningSite::served_certificate(name)` and `certificates().status()` to `TlsStatus` (names, `not_after`, issuer, last renewal), so the `ReadTls` answer comes from a real handshake, not from the store. Tested with a self-signed pair from `rcgen` on a loopback port, no network. Note: webservant 0.4.4 pins httpant `~0.5.22`, our `httpant-client` is 0.7.1, so two httpant lines compile side by side until webservant moves | nothing |
| C8b | `LocalTarget` (a `Target`): `Create` and `SetSetting` build a `webservant::Settings`/`Site` (files folder, name, optional proxy), `observe` reads the settings back from the running site, `locate` finds it by app id; loopback only by default. Conformance suite runs against it with a fixed `Settings` and a loopback site, no certificate authority  DONE in 0.0.51 (stacked on 0.0.50); refuses any other setting name; secrets/domains/ReadTls/Restore are No | C8a |
| C8c | `Restore` for `LocalTarget`: snapshot is the `Settings` in force; restore stops the site and starts it with the snapshot's, or stops it when the app did not exist (only when its marker is ours) DONE in 0.0.52 (stacked on 0.0.51) | C8b |
| C8d | (fake-CA slice DONE in 0.0.53; the staging run is still to do) Automatic TLS: `Acme` settings, a reachability check wrapping `Site::check_reachable()` (one issuance against Let's Encrypt STAGING with a throwaway store), a failed renewal reported by `ReadTls` as `Renewal::Failed`; fake CA first (webservant's own test CA), then a staging run by hand | C8b; the staging run needs a public name and open ports 80/443 and a go from the PM ([ASK] first) |
| C8e | The on-boot path: a service definition (systemd unit, Windows service) generated and shown, never installed without an approved digest; SupervisorAnt does not exist, so this is ours and says so | C8c |

Open on board 320: D6 (test account) does not block C1 to C4 or the build of C5; it blocks only C5's real run and the shapes. D7 (ConfigurAnt additions) is not needed by C1 to C6
(projection is built). D8 (ReleaseAnt and installer files) is not needed.
