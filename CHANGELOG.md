# Changelog

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
