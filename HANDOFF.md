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

## State (2026-10-31)
main = d7404e9 (0.0.21). Open PR: dry-run deploy test (0.0.22 provisional). Synapse: my session was dropped by the 2026-10-30 restart; the PM said do NOT re-claim (the Synapse lane diagnoses); use claude-peers.
HTTPAnt 0.5.19 is published: server TLS (CertSource, SNI, h2, trusted proxy, cert read-back) is ready, but that feeds WebServAnt (still 0.0.0), so local serving/TLS waits. Client TLS (https) is NOT there: the PM asked httpant for it as a row; my need note is `docs/HTTPS-CLIENT-NEEDS.md` (merged); I depend only on a published version. Cloudflare `BindDomain`/`ReadTls` (REST) and a real health probe wait for it. Need list sent earlier: `docs/TLS-NEED-LIST.md`.
PM [FIX] 2026-10-31: go on the Cloudflare dry-run rows, one at a time, version at gate. Row 1 = real `wrangler deploy --dry-run` with the adapter's own args (needs no account; CI installs wrangler 4.141.0). Candidate next rows, same method: other wrangler calls that work offline (check `--help` for `--dry-run`/`--outdir`); parse-shape checks stay UNVERIFIED until D6.
Still waiting on CireSnave: D6 test account (TASK 2: run the suite against real wrangler, verify UNVERIFIED shapes in `cloudflare.rs`), D7 ConfigurAnt additions, D8 ReleaseAnt. Unbind: wait-for-consumer. Idea not asked: log "pushed N secrets" (count).
Rulings: Cloudflare only first; secrets pushed (paste fallback); presence = host API then authenticated config retrieval, no unauthenticated tier; local serving waits for WebServAnt (waits for HTTPAnt). See ROADMAP.md, docs/CONSUMER-CONTRACT.md.
Gotchas: files are CRLF-in-worktree/LF-in-blob; scripted replaces must assert count==1. Use a worktree, never switch the primary. On Windows `wrangler` is a `.cmd` shim (ProcessRunner goes through `cmd /C`).
