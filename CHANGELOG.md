# Changelog

## [0.0.22] - 2026-10-31

- First Cloudflare dry-run row: a test runs the **real** `wrangler deploy --dry-run` with the exact arguments the adapter sends (`deploy_args`, split out of `deploy`), which builds the Worker and exits before any upload, so it needs no account. It proves wrangler 4.141.0 accepts `--cwd`, `--name`, `--keep-vars` and `--var`, and that variable values are not echoed. Mutation (a misspelled flag) fails it with wrangler's own "Unknown arguments"; a second test shows the dry run can fail. CI installs `wrangler@4.141.0`. Does not verify anything on Cloudflare's side (D6). No public API change. Version provisional; the PM allocates.

## [0.0.21] - 2026-10-31

- Docs: `docs/HTTPS-CLIENT-NEEDS.md`, what WebHostAnt needs from the HTTPAnt client for https (Cloudflare API calls and a real health probe), each need marked must/should. No code change. Version provisional; the PM allocates.

## [0.0.20] - 2026-10-30

- Docs: `docs/TLS-NEED-LIST.md`, the need list for HTTPAnt (who terminates, certificate source, SNI, ALPN, renewal without restart, what HTTPAnt reports), each need marked must/should. HTTPAnt gets a certificate hole, not a TLS stack. No code change. Version provisional; the PM allocates.

## [0.0.19] - 2026-10-29

- Data after a rollback, as proposed in `docs/DATA-AFTER-ROLLBACK-DESIGN.md` and answered by the PM: `Migrations` (`NoData`, `Reversible`, `ForwardOnlyOlderTolerates` = additive-only, `ForwardOnlyOlderRefuses`) on the manifest, `Release` and `Host::plan_release` beside `Host::plan` (which keeps `Unknown`), `DataAfterRollback` on the plan, printed as a first `DATA:` line and included in `Plan::digest`, so an approved plan with the warning edited away is refused. `NeedsBackupRestore` is information, not a block; rollback still never touches data. Mutations (data left out of the digest; a refusing migration reported as following) fail tests. Breaking pre-1.0: `Plan` text gains a line, new public types.
- Secret names stay out of logs (CodeQL cleartext-logging alerts on this PR): the Cloudflare adapter's log, its errors and the timeout text withhold a secret's name (`secret put`, not `secret put <NAME>`); the fake target logs `set secret`. Names still appear in `Plan`'s text (the person approving must see which secrets are pushed) and in `Error::MissingSecret` (the person must know which one to supply); neither is logged by WebHostAnt. One test captures log, errors and call text for a recognisable fake name; putting the name back fails it.
- New `SECURITY.md` with the portfolio's "Supported versions" text (latest line gets fixes; an older line only when the backport is small; no period promised).

## [0.0.18] - 2026-10-29

- Docs: propose stating before apply whether data can follow a rollback (`docs/DATA-AFTER-ROLLBACK-DESIGN.md`): a `Migrations` policy on the manifest, a per-release flag, `DataAfterRollback` on the plan and in its digest. No code change. Version provisional; the PM allocates.

## [0.0.17] - 2026-10-29

- Docs: `docs/CLOUDFLARE-FACTS.md` records that Cloudflare custom domains have no wrangler bind or list command. Ruled by the PM: no Cloudflare `BindDomain`/`ReadTls` adapter until the D6 test account exists and HTTPAnt can carry REST calls. No code change.

## [0.0.16] - 2026-10-29

- Approved plans only (CONSUMER-CONTRACT section 2: "a hash so a tampered plan is refused"): `Plan::digest()` is a SHA-256 fingerprint of the app, undo note and every step (setting values, secret names only); `Host::apply_approved(plan, digest, secrets)` refuses a changed plan with the new `Error::PlanChanged` before the target is touched. `Host::apply` is unchanged. New dependency `sha2`. Mutation (digest ignores steps) fails two tests. Breaking pre-1.0: `Error` gains a variant. Version provisional; the PM allocates at gate.

## [0.0.14] - 2026-10-29

- Setting up domains, as proposed in `docs/DOMAINS-DESIGN.md`: `AppManifest::domain(name)`, `Step::BindDomain`, `Op::BindDomain`, `Observed.domains`, `ReadBack.domains`, `Error::InvalidDomain`. `plan` refuses a name that is not a plain host name before touching the target, and binds each declared name the target does not already show; the read-back fails unless every bound name is shown. Names bound but not declared are left alone. No DNS and no certificate work; rollback does not unbind. The Cloudflare adapter declares `BindDomain` as `No` (not built).
- Conformance: one new check; `fake::Fault::DomainNotBound` proves it can fail. Breaking pre-1.0: `Op`, `Step` and `Error` gain variants, `Observed` and `ReadBack` gain fields.

## [0.0.13] - 2026-10-29

- Docs: propose setting up domains (`docs/DOMAINS-DESIGN.md`): `AppManifest::domain`, `Step::BindDomain`, bound names in `Observed`, read-back and conformance. No code change.

## [0.0.12] - 2026-10-29

- TLS read-back: `Host::tls_status(app_id)` returns `TlsStatus { names, not_after, issuer, last_renewal }` as the target reports it (`Op::ReadTls`, `Target::tls_status`, defaulted to refuse). An app that is absent or not ours, an unreachable host and a target that cannot report are errors, never an empty status. Read-only; setting up domains is a later slice. The Cloudflare adapter does not implement it yet (stays `Unverified`, so `Unsupported`).
- Conformance: one new check; `fake::Fault::TlsEmptyInsteadOfError` proves it can fail. Breaking pre-1.0: `Op` gains a variant.

## [0.0.11] - 2026-10-29

- Rollback and per-step apply results, as ruled by the PM (`docs/ROLLBACK-DESIGN.md`). `apply` snapshots before the first step (when the target can restore) and, on any failure after it starts, returns `Error::ApplyFailed { steps, cause, rollback_point }` with one `StepOutcome` (`Applied`/`Skipped`/`Failed`) per step; a re-plan then resumes without repeating finished steps. `Host::rollback` restores, reads the target back, and answers `Rollback::Restored | Unchanged | Unknown { reason, details }`: never `Restored` unless the read-back matched. New `Op::Restore`, `Snapshot`, `Target::snapshot`/`restore` (defaulted: a target that does not override `restore` refuses), `Host::can_roll_back`, `Plan::undoable`. A target that cannot restore (Cloudflare, for now: `Support::No`) is told so in the plan text and gets no rollback point. Secrets: only the name set is restored.
- Conformance: five new checks and two scenarios (`FailsLastStepOnce`, `DropsAfterApply`; a custom `make` closure must handle them). `fake::Fault::RestoreDoesNothing` proves a dishonest restore is caught; mutations (read-back always passes; failed step not marked) fail the suite.
- Breaking (pre-1.0): `Op` and `Error` gain variants, `Applied` gains a field, `Scenario` gains variants.

## [0.0.10] - 2026-10-29

- Host-call timeouts: `cloudflare::ProcessRunner` now stops a `wrangler` call (and its child processes) that outlives its limit (`DEFAULT_TIMEOUT` 10 min, `ProcessRunner::with_timeout`) and returns the new `Error::TimedOut`, which says the target may be partly changed. A timeout is an error, never "absent". `ProcessRunner` is no longer a unit struct (use `ProcessRunner::default()`); new `Error` variant. Mutation (limit never fires) fails the test.

## [0.0.9] - 2026-10-29

- `docs/ROLLBACK-DESIGN.md`: proposed per-step apply results, `snapshot`/`restore` and a `Rollback` outcome that is never `Restored` unless read back. Design only; nothing built.

## [0.0.8] - 2026-10-29

- `conformance`: new check that an unreachable host is never reported as not installed (`Scenario::Unreachable`; `fake::Fault::UnreachableIsAbsent` proves it can fail). Adds public enum variants, so a custom `make` closure must handle `Scenario::Unreachable`.

## [0.0.7] - 2026-10-29

- `cloudflare::CloudflareTarget`: a `Target` that drives a pinned `wrangler` through a `Runner` trait (`ProcessRunner` runs the real one). Ownership is a `WEBHOSTANT_APP` marker setting; unreachable or unauthenticated is an error, never "absent"; secrets go in on stdin and are redacted from `Debug`, logs and errors. Every operation is `Unverified` until the caller opts in per operation (`trust_operations`): commands and flags were read from `wrangler 4.141.0 --help`, but what wrangler prints is assumed (marked UNVERIFIED in the code) until a test account exists (D6). Conformance suite passes against a fake runner; a mutation (secrets not stored) fails it. Adds `serde_json`. Not for release: patent sweep rows still owed.

## [0.0.6] - 2026-10-29

- `docs/CLOUDFLARE-FACTS.md`: Containers plan requirement not stated (probably paid); deploy documented only through `wrangler deploy` with local Docker. Docs only.

## [0.0.5] - 2026-10-29

- First code: contract types (`AppManifest`, `Plan`, `Step`, `Target`, `Host`), plan/apply/read-back, an in-memory `fake::FakeTarget` with deliberate faults, and `conformance::run` (plan inert; read-back proves apply; idempotent; no secret leak; refusals change nothing; foreign app never overwritten; unsupported is an error). No real adapter; Cloudflare is next. README example compiled as a doctest. Not for release: the patent sweep rows (PATENT-SWEEP.md) are still owed before a release with functionality.

## [0.0.4] - 2026-10-28

- DESIGN.md 4.1-4.3 and every other copy of the unauthenticated-signature claim replaced (retraction, not caveat). Rulings applied (docs only): Cloudflare is the only first target, secrets are pushed with a manual-paste fallback, and presence is a host-side check then an authenticated configuration retrieval, with no unauthenticated tier (`docs/CONSUMER-CONTRACT.md` 1a, 2a).

## [0.0.3] - 2026-10-28

- `docs/CONSUMER-CONTRACT.md` (what a consumer provides and gets back, generalised from Humboldt's installer design) and `docs/EDGE-AND-AGNOSTICISM.md` (edge/TLS as a capability interface; updated for the build-order ruling: local serving waits for WebServAnt, which waits for HTTPAnt). Docs only.

## [0.0.2] - 2026-10-28

- HANDOFF for the new WebHostAnt lane; ROADMAP records the spin-out ruling (board 320). Docs only.

## [0.0.1] - 2026-10-28

- Design note `docs/DESIGN.md`: goals, targets, detection, local TLS (build or adopt), ConfigurAnt and SecretAnt integration, adapter interface, threat model, slices, patent queries, decisions. Docs only; still no functionality, not published.

## [0.0.0] - 2026-10-28

- Name reserved with an empty placeholder crate.
