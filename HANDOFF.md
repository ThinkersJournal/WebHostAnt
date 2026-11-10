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

## State (2026-11-10)
PM TASK 2026-11-08 (Cloudflare path): C1-C6, C9, Render slice 3, configurant 0.7.2 merged (main 2859b4d). D6 closed; real-target test passed (PM run). No CI workflow for it (PM: self-hosted policy forbids secrets for a public repo; PM runs it by hand under `with-secret` before each release). Open PRs, stacked: #43 httpant-client 0.7.1 (0.0.44, `httpant-071`), then #44 `c8-rows` (0.0.45, ROADMAP C8a-C8e, worktree `../WebHostAnt-wt-c8-rows`), then `render-deploy-once` (0.0.46, worktree `../WebHostAnt-wt-render-once`, stacked on c8-rows).
**TASK (waiter):** when each merges: remove its worktree (check `git status --porcelain`), `git worktree prune`, `git branch -D <branch>`, confirm remote branch absent; rebase `c8-rows` with `--onto origin/main <old httpant-071 tip>` if #43 squashes first. Flaky test (re-run once, tell the PM): `a_call_that_outlives_its_limit...`.
**RULE (PM 2026-11-08):** [ASK] before EVERY real-account run. Vault runs are the PM's. A first publish of webhostant is a decision, not made. Sign GitHub posts with `— lane: webhostant`.
**NEXT:** PM TASK 2026-11-10: take C8 (webservant 0.4.4). Build C8a first (see ROADMAP). Render deploy-once hook: built (0.0.46, fake only). CireSnave 409=B: the Render account exists and the Render CLI on this box is logged in; PM wants a real test, free tier only, delete what you create, no secrets in output: [ASK] the PM before the run. C7 waits for SecretAnt row 3. No WAIT lines.

Docs: `docs/SECOND-HOST.md`, `docs/TLS-NEED-LIST.md`, `docs/HTTPS-CLIENT-NEEDS.md`, `docs/CONSUMER-CONTRACT.md`, ROADMAP.md. Gotchas: CRLF-in-worktree/LF-in-blob (scripted replaces assert count==1); worktree, never switch the primary; `wrangler` is a `.cmd` shim; tests must print fixed labels (CodeQL); `supertool` untracked, leave alone.
