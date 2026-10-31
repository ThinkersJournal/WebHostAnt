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
Relaunch pins `--model sonnet`. Reply on the channel a message arrived on. Synapse: my session was dropped by the 2026-10-30 restart; the PM said do NOT re-claim (the Synapse lane diagnoses); use claude-peers (PM = cwd `C:\Projects`, id changes on restart: list_peers).

## State (2026-10-31)
main = ccfc7c2 (0.0.28). No open PR, no extra worktrees. Merged this session: #23 HealthProbe (0.0.24), #24 Cloudflare domains REST facts + RELEASE-CHECKLIST (0.0.25), #25 BindDomain over REST (0.0.26), #26 ReadTls from certificate packs (0.0.27), #27 ConfigurAnt projection, feature `configurant` (0.0.28; CI now --all-features). All REST shapes are from Cloudflare's published OpenAPI schema (docs/CLOUDFLARE-FACTS.md), NOT live answers.
Channel: **Synapse only** (CireSnave ordered claude-peers out 2026-10-31); PM role `pm@ciresnave`. GitHub GraphQL budget was exhausted until 16:53Z 2026-10-31: do not loop on PR status.
**Next:** PM said take row 3 further: the second host adapter's read-only parts (no token) is open but the second host is not ruled (Cloudflare is the only ruled target): [ASK] sent, wait for the answer. Cloudflare rows that need the real token wait for D6 (CireSnave): run RELEASE-CHECKLIST.md by hand before a release touching cloudflare.rs.
Other waits: D7 ConfigurAnt question flow, D8 ReleaseAnt. Local serving/TLS waits for WebServAnt (0.0.0). Unbind: wait-for-consumer. Idea not asked: log "pushed N secrets" (count).

Docs: `docs/TLS-NEED-LIST.md`, `docs/HTTPS-CLIENT-NEEDS.md` (both sent to httpant). Rulings: Cloudflare only first; secrets pushed (paste fallback); presence = host API then authenticated config retrieval, no unauthenticated tier. See ROADMAP.md, docs/CONSUMER-CONTRACT.md.
Gotchas: files are CRLF-in-worktree/LF-in-blob; scripted replaces must assert count==1. Use a worktree, never switch the primary. On Windows `wrangler` is a `.cmd` shim. Tests that print argv trip CodeQL cleartext-logging: print fixed labels.
