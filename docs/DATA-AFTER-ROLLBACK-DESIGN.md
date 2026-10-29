# Saying, before an apply, whether data can follow a rollback

**Proposal, nothing built.** Written 2026-10-29 by the WebHostAnt lane against `origin/main` `544cd4a`. Inputs: `CONSUMER-CONTRACT.md` section 1 rows 5 and 6 and
section 2 (Rollback line), `ROLLBACK-DESIGN.md` point 5. The contract promises that rollback "states **before apply** whether data can follow (the schema
accepts the old code, or only a backup restore will do)". Today the plan says only whether the *target* can undo (`Plan::undoable`); it says nothing about data.

## Hello-world first

```rust
let app = AppManifest::new("my-app").migrations(Migrations::ForwardOnlyOlderRefuses);  // new: the consumer's declared policy
let plan = host.plan_release(&app, &config, Release::MigratesSchema)?;                  // new: this release changes the schema
println!("{plan}");   // "after a rollback: old code will refuse the new data; only a backup restore will do"
assert_eq!(plan.data_after_rollback(), DataAfterRollback::NeedsBackupRestore);
```

## What WebHostAnt can and cannot know

WebHostAnt never sees the consumer's schema or runs its migrations (they stay the consumer's, contract section 5). So it cannot *discover* whether a
release migrates; the consumer must say so at plan time. It then combines two declared facts and states the result. It does not check them.

1. `Migrations` on the manifest (row 6): `NoData` (nothing to protect), `Reversible`, `ForwardOnlyOlderTolerates` (additive-only: the migration only adds, so older code still runs on the new data), `ForwardOnlyOlderRefuses`.
2. `Release` at plan time: `MigratesSchema` or `NoSchemaChange`. Default for the existing `Host::plan` is **`Unknown`**, which yields `Unknown` below: never
   a guess in the comforting direction.

## The statement

`DataAfterRollback`: `NoData`, `Follows` (no schema change, or older code tolerates or the migration reverses), `NeedsBackupRestore` (forward-only and older
code refuses), `Unknown` (the consumer did not say). `Plan` Display prints one plain sentence for each, first line, before the steps. The statement is part of
`Plan::digest`, so an approved plan cannot have its data warning edited away (a test edits the sentence away and the plan is refused). The line is printed
as its own first line, starting `DATA:`, so it cannot be mistaken for a step. `Host::rollback` is unchanged: it still touches code, settings and secret
names only, and never data. `NeedsBackupRestore` is information, not a block; blocking would stop the person recovering code when they accept the risk.

## Conformance

The statement is derived from the manifest and the release flag alone, not from the target, so the checks live in unit tests, not in the target suite:
each policy and release pair gives its one statement; `Unknown` is the default; changing either input changes the digest; a forward-only-refuses plan that
migrates never prints "follows".

## Decided by the PM (2026-10-29)

- The four policy names stand; "older code tolerates only additive changes" is `ForwardOnlyOlderTolerates`, so no fifth.
- `Host::plan_release` sits beside `Host::plan`; `Host::plan` keeps `Unknown` as its default.
- The code PR is 0.0.19.
