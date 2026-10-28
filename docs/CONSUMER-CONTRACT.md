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
| 7 | *(optional)* **Signature endpoint** `/.well-known/<app-id>` returning `app`, `setup`, and fields the app chooses | Lets a client choose install vs reconfigure | Proposed; **held pending board 318 — do not build on it yet** |

A consumer with no signature endpoint still works: detection then relies on the host API alone (what is deployed under that name) plus `/health`, and
WebHostAnt reports the weaker confidence rather than guessing.

## 2. What WebHostAnt returns

Every operation is `plan` (inert data) then `apply`, then a **read-back**:

- **Plan:** an ordered list of steps with their target, non-secret values, and secret **names** only; plus a hash so a tampered plan is refused.
- **Apply result:** per step, what was actually found afterward (never "exit code 0"). Names set, never values.
- **Idempotent:** the same plan applied twice changes nothing the second time.
- **Detect:** what is deployed and, if the signature exists, its answer — labelled as a hint, never as authority.
- **Domain/TLS status:** names bound, certificate not-after date, issuer, last renewal result.
- **Rollback:** switch to the retained previous artifact and configuration, then read health. It states **before apply** whether data can follow
  (the schema accepts the old code, or only a backup restore will do).
- **Errors** say what the person should do next, and an operation an adapter does not support returns a named error, never a partial success.

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

- Does the consumer need WebHostAnt to run **on** the host (over SSH) or **against** it remotely? Installer design prefers on-the-box for Docker.
- Backup/restore is unspecified for Humboldt; WebHostAnt will call a consumer-provided backup hook only once one is defined (no schema invented here).
- Signature endpoint fields wait on board 318.
