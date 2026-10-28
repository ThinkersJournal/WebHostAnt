# Consumer contract

**Design only. Nothing here is built.** Written 2026-10-28 by the WebHostAnt lane against WebHostAnt `origin/main` `de284d2`. Derived from Humboldt's
installer design (`docs/INSTALLER-DESIGN.md` on branch `docs/installer-design` @ `1472ce0`, read in full; not yet on Humboldt `main`) and from the
Humboldt lane's own must-have list (peer message, 2026-10-28, status draft). Humboldt is the **first** consumer, not the owner: every name below is
generic, and section 5 lists what stays Humboldt's. **UNVERIFIED** marks anything not read.

## Hello-world first

```rust
// Intended, not built.
let app = AppManifest::new("my-app").health("/health").setting("APP_BASE_URL").secret("DATABASE_URL");
let host = webhostant::Host::from_config(&settings)?;   // target chosen by settings, not by code
host.plan(&app)?.apply()?.read_back()?;                 // plan is inert; read-back is the proof
```

## 1. What a consumer must provide

| # | Provide | Why | Humboldt today (read/reported) |
|---|---|---|---|
| 1 | **App id** (a fixed string) | Tells "ours" from "something else" when probing | `humboldt-pickup` |
| 2 | **A health path** that answers when the app is up | Read-back after every apply | `/health` returns only `{"status":"ok"}` |
| 3 | **A declared list of settings** (plain values) and **secrets** (names only), kept apart | Settings are projected to the host; secrets are delivered by reference | `install::SECRETS` vs `SETTINGS` in `backend/src/install.rs`; settings such as `APP_BASE_URL` are plain wrangler `[vars]` |
| 4 | **A deployable artifact** (container image by digest, or a binary) | The thing to put on the host | Container built from the repo `Dockerfile` |
| 5 | **A statement of data to protect** | Rollback and remove must never destroy it | A Postgres database; backups/restore not yet designed |
| 6 | **Migration policy** (forward-only or reversible; whether older code refuses a newer schema) | Decides what rollback may promise | Forward-only; older code refuses a newer schema |
| 7 | **A configuration-retrieval endpoint** (one path; see section 1a) | Answers "how is our app configured?" to an authenticated admin only, and feeds reconfigure | Humboldt's proposed signature endpoint is the closest thing; **held pending board 318 — do not build on it yet** |

### 1a. Presence is host-side, then an authenticated configuration request (ruled 2026-10-28)

CireSnave, first-hand, verbatim: *"why would an 'Is our app here?' respond at all to unauthenticated users?"* An unauthenticated answer only helps an
attacker fingerprint the app, so **there is no unauthenticated tier**. (An earlier draft of this section had a coarse unauthenticated answer; that came
from the PM's wording, not from the ruling, and is withdrawn.) Presence is two steps:

1. **Host-side presence.** Ask the *host's* API, with the host credential the installer already holds, whether the app is deployed (Cloudflare: list the
   Worker). The app takes no part.
2. **Authenticated configuration retrieval.** One endpoint on the app, answered only to an authenticated administrator or an operator token. It returns the
   app id, version, schema version and the full **non-secret** configuration (current values as the host serves them). Secrets appear by name and set/not-set
   only, never a value. An unauthenticated request gets the app's normal response (401 or its public page) and nothing about version or schema.

Hosting-level changes still use the hosting credential against the host's API (DESIGN.md 4.2). A consumer with no such endpoint still works: detection then
relies on step 1 plus `/health`, and WebHostAnt reports the weaker confidence rather than guessing. The authenticated answer is also the read-only source of
current values for ConfigurAnt's "write-back to non-file places" need (DESIGN.md 6.3 item 3). Exact fields and the version-disclosure choice (board 318) are
now narrower: they concern only what an authenticated caller sees.

## 2. What WebHostAnt returns

Every operation is `plan` (inert data) then `apply`, then a **read-back**:

- **Plan:** an ordered list of steps with their target, non-secret values, and secret **names** only; plus a hash so a tampered plan is refused.
- **Apply result:** per step, what was actually found afterward (never "exit code 0"). Names set, never values.
- **Idempotent:** the same plan applied twice changes nothing the second time.
- **Detect:** what the host's API says is deployed; then, if the caller is authenticated, the app's configuration answer. No unauthenticated probe of the app is made to decide presence.
- **Domain/TLS status:** names bound, certificate not-after date, issuer, last renewal result.
- **Rollback:** switch to the retained previous artifact and configuration, then read health. It states **before apply** whether data can follow
  (the schema accepts the old code, or only a backup restore will do).
- **Errors** say what the person should do next, and an operation an adapter does not support returns a named error, never a partial success.

## 2a. Secrets are pushed (ruled 2026-10-28)

CireSnave: *"Secrets should be pushed."* At deploy, WebHostAnt takes secret names and values from SecretAnt and writes them into the host's own store; where
SecretAnt is not available, the **fallback is manual paste** into the host, with WebHostAnt listing only the names needed. Read-back confirms a name exists, never the value.

## 3. Rules WebHostAnt enforces on every consumer

1. No secret value on a command line, in a file WebHostAnt writes, in a log, in an audit event or in a plan. Tests use a canary and assert booleans.
2. Nothing is believed "not installed" unless a control URL known to answer is reachable from the same machine in the same run.
3. A site that is not the consumer's app is never overwritten.
4. A server-side probe refuses private, loopback and link-local addresses unless the operator opts in.
5. No telemetry, no accounts, no stored credentials; a credential lives in memory for one run.

## 4. The seam, concretely

The consumer depends on the `webhostant` crate through a small public surface: `AppManifest`, `Host`, `Plan`, `Applied`, `ReadBack`, and a
`Target`/`Edge` trait pair for adapters. Adapters live **in this crate** (per the 2026-10-28 ruling), behind cargo features, so a consumer pays only for
the targets it enables. No consumer code is linked into WebHostAnt; no WebHostAnt type names a consumer.

## 5. What stays the consumer's

Its schema, migrations and data tools (Humboldt: `db-install`, `db-wipe`, `db-transfer`, `db-remove-accounts`, `dbtools`); first-user creation; in-app
settings and policy (district settings, passkey rules such as `WEBAUTHN_RP_ID` immutability — WebHostAnt only *carries* the value); creating
databases at a provider; its licence and any decision about whether it may be distributed (Humboldt's `LICENSE` forbids distribution, so nothing here
assumes Humboldt can be bundled); outbound email policy (Humboldt must run with email off); the installer wizard's UI and its `doctor` report.

## 6. Open points

- First target is **Cloudflare only**, so WebHostAnt runs *against* the host remotely; the on-the-box question returns with a Docker server, which waits for local serving.
- Backup/restore is unspecified for Humboldt; WebHostAnt will call a consumer-provided backup hook only once one is defined (no schema invented here).
- The endpoint's exact fields and the version-disclosure choice wait on board 318.
- Open for CireSnave (not decided): D6 real-target test account, D7 ConfigurAnt additions, D8 ReleaseAnt scope.
