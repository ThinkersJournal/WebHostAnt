# Changelog

## [0.0.31] - 2026-10-31

- Render adapter, slice 2 (module `render_target`; stacked on 0.0.30): `RenderTarget`, a `Target` over `RenderApi`. A service is ours when its variable `WEBHOSTANT_APP` equals the app id; a secret is a variable whose name is listed in `WEBHOSTANT_SECRETS` and is reported by name only (Render's variable schema has no secret flag; whether the API shows a value to a token holder is UNVERIFIED). Create waits for the first deploy; every variable change triggers a deploy that is waited for (whether Render redeploys by itself on a variable change is UNVERIFIED); a failed deploy is an error, a deploy that never goes live is `TimedOut`. Domains are bound with the custom-domain call. `Restore` and `ReadTls` are `No` for now. Every operation is `Unverified` until trusted. The conformance suite passes against a fake that answers in the schema's shapes. Six mutations (secret left in settings, in-progress counted as live, failed deploy ignored, restore claimed, secret not listed, everything ours) fail tests. `RenderApi::create_web_service` now returns `Created` (service plus first deploy id). Per the PM's ruling the secret limitation is stated loudly: in the module docs, in `RenderTarget::help()` and in `docs/SECOND-HOST.md` (on Render a secret is an ordinary environment variable that the API may show to any holder of the API key; use a dedicated workspace and key); `RenderTarget` and `RenderApi` have a `Debug` that carries no value, and a test checks `Debug`, `help()` and the log for a canary. Version 0.0.31 allocated by the PM.

## [0.0.30] - 2026-10-31

- Render adapter, slice 1 of about five (module `render`; recommended in `docs/SECOND-HOST.md`): `RenderApi` over the existing `Rest` trait for the calls the adapter will make: find a service by exact name and workspace, create a web service that runs a container image (plan checked against the schema's list before any call), read and set variables and secret files, list and add custom domains (with `verified` from `verificationStatus`), trigger and read deploys (status words classified; an unknown word is never live), and roll back. Shapes are from Render's published OpenAPI file, **not** a live answer (no Render account yet); tests use a fake that answers in them. Not yet a `Target`. Seven mutations (substring name match, other workspace, unknown status as live, plan unchecked, domain answer for another name, deploy answer for another deploy, full page trusted) each fail a test. A secret value never appears in an error. `cloudflare_domains::Method` gains `Post` and its `plain_id` is shared. Version provisional; the PM allocates.

## [0.0.29] - 2026-10-31

- Docs: `docs/SECOND-HOST.md`, Render, DigitalOcean App Platform and Fly.io read against DESIGN 3.1's six criteria from their published OpenAPI files and documentation pages, each cell naming what was read and marking the rest UNVERIFIED. Recommends Render, with the build cost (about five PRs) and what could not be read. Also commits the rewritten `HANDOFF.md`. No code change. Version provisional; the PM allocates.

## [0.0.28] - 2026-10-31

- ConfigurAnt integration, first slice (cargo feature `configurant`, module `configurant_source`, on ConfigurAnt 0.5.43): `project_settings` renders an app's declared plain settings from a resolved ConfigurAnt configuration into the `Config` that planning takes (found by the schema value's `env` name, else its `key`; unset values are left out so planning reports them missing), and `secret_sources` returns the schema's `source` reference for each of the app's secrets, naming a secret without ever reading its value. A value the schema marks `secret` can never be projected as a plain setting, and a manifest secret the schema does not mark secret is refused; neither error shows a value. This is DESIGN 6.3 item 1 (projection) and needs nothing new from ConfigurAnt. Mutations (secret check removed in either direction; the source reference replaced by the value) fail tests; an end-to-end test plans and applies with projected settings. CI now builds, lints, tests and documents with `--all-features`. Version provisional; the PM allocates.

## [0.0.27] - 2026-10-31

- `ReadTls` for Cloudflare, from the zone's certificate packs (`GET /zones/{zone_id}/ssl/certificate_packs`; fields read from Cloudflare's published OpenAPI schema, not a live answer, D6): `Domains::certificate_packs` and `tls_findings`. The status reports the soonest expiry among **active** packs that cover the app's bound hostnames (a `*.` wildcard covers one label only), the issuer, and a failed renewal only when a covering pack is `*_timed_out` or has validation errors; otherwise the renewal is `Unknown` because the schema carries no renewal history. A pending pack claims no expiry; dates that cannot be compared as text claim none; more than one page is refused. Like `BindDomain`, `ReadTls` is `No` without `with_domains`, `Unverified` until trusted; the adapter asks the Worker side first, so an unreachable or foreign host is never answered for. Mutations (wildcard over many labels; pending counted as active; latest instead of soonest expiry) fail tests; the conformance suite passes with `ReadTls` trusted. Version provisional; the PM allocates.

## [0.0.26] - 2026-10-31

- Domain binding for Cloudflare over the Workers REST API (`cloudflare_domains`: `Rest` trait, `Domains` list/attach/detach, `HttpRest` on httpant-client with the private-address guard on and redirects off). `CloudflareTarget::with_domains` turns `BindDomain` from `No` to `Unverified` (and `Yes` once trusted); `observe` then reads the app's bound hostnames. Shapes are those of Cloudflare's published OpenAPI schema, **not** a recorded live answer (D6): tests use a fake that answers in those shapes. Negative controls (each mutation fails a test): a hostname held by another Worker is never overwritten (no write is sent); an attach answer naming a different hostname is refused; a `success:false`, non-2xx or non-JSON answer is an error that carries Cloudflare's code and message; more than one page is refused, not truncated; ids that could add path segments are refused; the API token never appears in `Debug` or errors. The conformance suite passes with domains bound through the fake REST. `ReadTls` is still `Unverified` (certificate fields not extracted). Version provisional; the PM allocates.

## [0.0.25] - 2026-10-31

- Docs: the Workers custom-domains REST calls (list, get, attach, detach) read from Cloudflare's published OpenAPI schema into `docs/CLOUDFLARE-FACTS.md`, with what the schema does not say (idempotence, the code for a hostname held by another Worker, certificate expiry); this replaces "endpoint path not read". New `docs/RELEASE-CHECKLIST.md`: the by-hand check before a release that touches the Cloudflare adapter, until the D6 test account exists. No code change. Version provisional; the PM allocates.

## [0.0.24] - 2026-10-31

- Real health probe (`HealthProbe`, `Health`), built on the published `httpant-client` 0.5.22: one `GET` of the app's health path; `200` is healthy, any other status is reported as itself, and redirects are never followed (a `302` to a login page is `Status(302)`, not healthy). The private-address guard (`refuse_private_addresses`) is on by default and plain `http` is refused with it; `allow_private(true)` opts in for local development. `HealthProbe::into_probe` gives the closure `CloudflareTarget::new` takes. Mutations (guard off; `3xx` counted healthy) fail tests; a positive control shows the same server is healthy once opted in. Adds dependencies `httpant-client` (https) and `tokio` (rt). Not yet wired into a default; the consumer chooses the URL. Version provisional; the PM allocates.

## [0.0.23] - 2026-10-31

- Cloudflare dry-run row 2: the arguments of every other `wrangler` call the adapter makes (`deployments list`, `versions view`, `secret list`, `secret put`; now built by small functions the code and the test share) are checked by the **real** wrangler 4.141.0 with no credentials, an empty config directory and `CI=1`: it parses its arguments and stops at "set a CLOUDFLARE_API_TOKEN" before any network call, so it cannot reach a real account. Mutation (`--format` misspelled) fails it with wrangler's "Unknown argument"; a positive-control test shows an unknown flag is reported. This proves the flags exist, not what Cloudflare answers (output shapes stay UNVERIFIED until D6). The test names each call by a fixed label and never prints its arguments (CodeQL cleartext-logging alerts); `secret_put_args` is now `secret_name_put_args`: it carries the secret's NAME only, the value goes on stdin. No public API change. Version provisional; the PM allocates.

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
