# Rollback and partial apply

**Built in 0.0.11** (PM rulings 2026-10-29, below). Names differ slightly from the proposal: `StepOutcome`, `Error::ApplyFailed { steps, cause, rollback_point }`, `Host::can_roll_back`, `Plan::undoable`. Originally written 2026-10-29 by the WebHostAnt lane against `origin/main` `0b1e96f`. Inputs: `CONSUMER-CONTRACT.md` section 2
(rollback line) and the consumer lane's guarantees list (peer message, 2026-10-29). Exact wording of what a host call returns is **UNVERIFIED** until a
real Cloudflare account exists (D6).

## Hello-world first

```rust
let applied = host.apply(&plan, &secrets)?;          // today
let outcome = host.rollback(&applied.rollback_point.unwrap()); // new: None when the target cannot undo
assert_eq!(outcome, Rollback::Restored);                       // or Rollback::Unknown { reason, details }; never a bare Ok
```

## What is missing today

`Target` can only move forward (`apply_step`). `Host::apply` stops at the first failing step and returns an error, so the caller cannot tell which steps
ran. Neither can a conformance check say "rollback works".

## Proposed additions

1. **Per-step result.** `apply` records `Applied | Skipped | Failed` per step (in the error too), so a partial apply is visible and a re-run resumes
   without repeating finished steps. Steps are already idempotent; this makes it observable.
2. **`Target::snapshot(app_id) -> Result<Snapshot, Error>`**, read-only, taken by `Host::apply` before the first step. A `Snapshot` is the `Observed`
   state plus an adapter-private handle (Cloudflare: the previous version id). It holds secret names only, never values.
3. **`Target::restore(app_id, &Snapshot) -> Result<(), Error>`**, optional (`Op::Restore`, `Support` like any op).
4. **`Host::rollback(&RollbackPoint) -> Rollback`** calls `restore`, then reads back like `apply`. The result is an enum, not `Result<(), _>`:
   `Restored`, `Unchanged` (nothing had been applied), or `Unknown { details }` when the read-back does not match or the host is unreachable.
   A failed or unverifiable rollback is never `Restored`.
5. **Data is out of scope for the adapter.** Rollback restores code, settings and secret *names*. It never deletes or downgrades a data store; a
   destructive step needs a separate, explicit flag on the plan (not designed here). Whether data can follow is the consumer's declared migration
   policy (contract section 1, rows 5 and 6) and is stated before apply.
6. **Secrets on rollback.** A secret overwritten by an apply cannot be read back from the host, so rollback restores the prior *name set* only and reports
   `Unknown` for any secret whose old value is needed; the consumer re-pushes from SecretAnt.

## Conformance checks this enables

- apply, then rollback: observed state equals the snapshot.
- fail the third of four steps (fake fault): `apply` reports which ran; rollback returns to the snapshot; a re-run of the plan completes.
- a fault where `restore` does nothing but returns Ok must be caught by the read-back (`Rollback::Unknown`).
- an unreachable host during rollback yields `Unknown`, never `Restored`.

## Ruled by the PM, 2026-10-29

- Keep `Unknown`: after a failed or unverifiable restore the state is genuinely unknown, and `Failed` would claim knowledge we lack. It carries a `reason` (and `details`, the names that differed, when the read-back ran).
- Adapters that cannot snapshot declare `Op::Restore` = `No`; `Plan` then prints "this target cannot undo these changes" and `apply` returns no rollback point. The Cloudflare adapter does this until it reads a previous-version id (UNVERIFIED, D6).
- Not covered here: TLS read-back, host-call timeouts. Both are separate slices.
