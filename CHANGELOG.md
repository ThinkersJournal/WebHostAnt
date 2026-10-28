# Changelog

## [0.0.4] - 2026-10-28

- Rulings applied (docs only): Cloudflare is the only first target, secrets are pushed with a manual-paste fallback, and presence is a host-side check then an authenticated configuration retrieval, with no unauthenticated tier (`docs/CONSUMER-CONTRACT.md` 1a, 2a).

## [0.0.3] - 2026-10-28

- `docs/CONSUMER-CONTRACT.md` (what a consumer provides and gets back, generalised from Humboldt's installer design) and `docs/EDGE-AND-AGNOSTICISM.md` (edge/TLS as a capability interface; updated for the build-order ruling: local serving waits for WebServAnt, which waits for HTTPAnt). Docs only.

## [0.0.2] - 2026-10-28

- HANDOFF for the new WebHostAnt lane; ROADMAP records the spin-out ruling (board 320). Docs only.

## [0.0.1] - 2026-10-28

- Design note `docs/DESIGN.md`: goals, targets, detection, local TLS (build or adopt), ConfigurAnt and SecretAnt integration, adapter interface, threat model, slices, patent queries, decisions. Docs only; still no functionality, not published.

## [0.0.0] - 2026-10-28

- Name reserved with an empty placeholder crate.
