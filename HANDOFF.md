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

## FIRST ON NEXT START (PM [TASK] SOAK WAVE 1, 2026-10-30; CireSnave: all lanes join Synapse for a 3-day side-by-side soak, claude-peers stays)
You are relaunched onto BOTH channels. (1) ToolSearch "synapse" must find tools; if not, say so. (2) Send the PM "soak webhostant #1" on synapse.
(3) Reply on whichever channel a message arrived on. Then resume the State below (idle; nothing unblocked).

## State
main = da42863 (0.0.19, PR #18 merged 2026-10-30; worktrees and branches cleaned). No open PR. Nothing unpushed.
No unblocked roadmap row: everything waits on CireSnave (D6/D7/D8) or HTTPAnt/WebServAnt. Check origin/main and claude-peers for new tasks.
Cloudflare `BindDomain`/`ReadTls`: PM ruled DEFERRED until the D6 test account exists and HTTPAnt can carry REST (wrangler has no domain bind/list command: docs/CLOUDFLARE-FACTS.md).
TASK 2 (blocked on CireSnave D6 test account): run the suite against real wrangler and verify the UNVERIFIED output shapes in `cloudflare.rs`. Real HTTP health probe waits for HTTPAnt. Open for CireSnave: D6, D7 ConfigurAnt additions, D8 ReleaseAnt. Unbind: PM ruled wait-for-consumer (2026-10-30). PM wakes this lane when HTTPAnt/WebServAnt unblock a row. Idea not asked: log "pushed N secrets" (count) if the PM wants it.
Rulings: Cloudflare only first; secrets pushed (paste fallback); presence = host API then authenticated config retrieval, no unauthenticated tier; local serving waits for WebServAnt (waits for HTTPAnt). See ROADMAP.md, docs/CONSUMER-CONTRACT.md.
Gotchas: files are CRLF-in-worktree/LF-in-blob; scripted replaces must assert count==1. Commit this HANDOFF.md edit in the next PR; use a worktree, never switch the primary.
