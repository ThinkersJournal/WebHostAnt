kind: docs: ROADMAP C5 and C6 stated true, and an EasyAnts card
---
Documentation only. The ROADMAP rows for C5 (the runnable example) and C6 (`Restore` for Cloudflare) still said both needed a by-hand run; both are built (`examples/deploy.rs`, `CloudflareTarget::restore`) and the plain-Worker shapes were closed by real runs on 2026-11-08 and 2026-11-09. The rows now say so, record the PM's 2026-11-19 decision of no further real run, and name the one thing still unproven: whether a Worker rollback reverts what a Container binding runs. Checked by reading `src/cloudflare.rs` and `docs/CLOUDFLARE-FACTS.md` at origin/main.

Adds `easyants-card.toml` at the repo root (format: ThinkersJournal/easyants `docs/CARD-FORMAT.md` at 12844fd). It is a `not-ready` card ("Name reserved, not released"): crates.io holds `webhostant` 0.0.0 only (read 2026-11-19), so no install link or example is given. No code changed.
