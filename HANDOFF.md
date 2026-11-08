# HANDOFF - WebHostAnt lane (current state, written by the PM 2026-10-28)

You are the **WebHostAnt lane**. Read `C:\Projects\AGENT-ONBOARDING.md` once, then `docs/DESIGN.md` here (421 lines, written by a PM-launched
agent; every unverified fact in it is marked UNVERIFIED). Messages: Synapse and claude-peers (PM role `pm`). Models: sonnet only.

## What you are building, and the rule that governs it
Deploy a web app to common hosts and serve it locally with automatic TLS, configured by ConfigurAnt, secrets from SecretAnt.
**CireSnave 2026-10-28 (first-hand, board 320):** *"We build tools before we build projects built on top of those tools. Humboldt needs
WebHostAnt or it keeps building the equivalent internally. Spin out WebHostAnt now. Have Humboldt be the initial goal-setter for what
WebHostAnt needs to provide. Try to keep WebHostAnt non-Humboldt-specific in the process. The interface for WebHostAnt's seam between it
and other projects will initially be between it and Humboldt but won't always be. Create WebHostAnt as its own project with input flowing
from Humboldt...not as part of Humboldt that we will eventually have to rip out and create seams for."*
So: **adapters live HERE from the start**; Humboldt's installer depends on this crate; nothing Humboldt-specific goes in. The seam is a
documented consumer contract; Humboldt is its first consumer, not its owner.

## The open design question you must answer first (CireSnave, same message)
*"Caddy is a web server. Does it support our goals of web server library agnosticism? If not, Caddy is the wrong choice there. That's why I
pointed you toward web-server-abstraction in the past. Do we need a WebServAnt project that is a web server library agnostic web server?"*
PM's starting view (check it, do not just adopt it): (a) HTTPAnt (the reshaped web-server-abstraction: core + server + client, adapter chosen by
id) is the library-agnostic layer and is still a 0.0.0 placeholder; (b) TLS/edge must be a WebHostAnt **capability interface** with an
in-process implementation path (ACME library; lightbulb has a working `src/tls.rs` on instant-acme to learn from) AND external-edge adapters
(Cloudflare, Caddy, nginx); Caddy may be ONE adapter, never the required one; (c) a standalone agnostic web-server program ("WebServAnt")
is what HTTPAnt + in-process ACME would naturally produce; recommend whether to defer it. Put your answer in `docs/EDGE-AND-AGNOSTICISM.md`
with options A/B/C for CireSnave, plain sentences, hello-world first (EXPECTATIONS 6.24).

## First tasks, in order
1. `docs/CONSUMER-CONTRACT.md`: what a consumer must provide and what WebHostAnt returns, derived from Humboldt's installer design
   (`git -C C:\Projects\HumboldtUnifiedKidTracker show origin/main:docs/INSTALLER-DESIGN.md`) and generalised. Ask the Humboldt lane (claude-peers,
   cwd `HumboldtUnifiedKidTracker`) for its must-haves; strike every Humboldt-specific name. List what stays Humboldt's.
2. `docs/EDGE-AND-AGNOSTICISM.md` as above.
3. First slice of code: the contract types + conformance harness (no adapters yet), with a ten-line hello-world that compiles in CI.
Ant-first rule: do not hand-roll config loading, secrets, redaction, audit, HTTP scaffolding: use ConfigurAnt/PersistAnt/CapAnt where they
exist (all other Ants are 0.0.0 placeholders: stub behind a trait and say so). One PR at a time, red-first, version bump every PR (placeholders:
PM allocates at gate). Memory rule: CARGO_BUILD_JOBS=2, one build at a time. Repos: ThinkersJournal/WebHostAnt, crate `webhostant` 0.0.0 published.

## FIRST ON NEXT START
Relaunch pins `--model sonnet`. Read C:\Projects\THROTTLE.md first. Channel: Synapse only (`pm@ciresnave`); my session was dropped by the 2026-10-30 restart, PM said do NOT re-claim. Do not loop on GitHub PR status.

## State (2026-11-08)
PM TASK 2026-11-08 (Cloudflare path): ROADMAP rows C1-C8 merged (#31); C1 (#32, 0.0.33), C2 (#33, 0.0.34) and C3 readiness wait (#34, 0.0.35, main d612235) merged. In review: C4 preflight (0.0.36, branch `c4-preflight`, on main, [READY] sent). Worktree `../WebHostAnt-wt-c4-preflight`.
**TASK (waiter):** when C4 merges: remove that worktree (check `git status --porcelain`), `git worktree prune`, `git branch -D c4-preflight`, confirm remote branch absent. The timing test `a_call_that_outlives_its_limit...` can red in a loaded run: re-run once and tell the PM.
**NEXT unblocked row: C5** a runnable example (plan, show, approve digest, apply, read back) against any Worker project dir (`examples/preflight.rs` exists); its REAL run on a throwaway Worker needs CireSnave's wrangler login or the D6 account: [ASK] the PM before that run, never before. C6 (Cloudflare `Restore`) needs C5's real run.
No WAIT lines. Render slice 3 stays held behind the Cloudflare rows.
Render test account needs (do NOT ask; PM puts on board 320 next to D6): dedicated workspace + API key + workspace id + public test image. Cloudflare real-token rows wait for D6: run `docs/RELEASE-CHECKLIST.md` by hand before releases touching cloudflare.rs. D7, D8, local serving/TLS (WebServAnt 0.0.0) still wait.

Docs: `docs/SECOND-HOST.md`, `docs/TLS-NEED-LIST.md`, `docs/HTTPS-CLIENT-NEEDS.md`, `docs/CONSUMER-CONTRACT.md`, ROADMAP.md. Gotchas: CRLF-in-worktree/LF-in-blob (scripted replaces assert count==1); worktree, never switch the primary; `wrangler` is a `.cmd` shim; tests must print fixed labels (CodeQL); `supertool` untracked, leave alone; this HANDOFF is an uncommitted local edit (commit with the next PR).
