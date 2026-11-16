kind: Raise webservant to 0.4.12
---
The `local` feature depends on `webservant` 0.4.12 (published on crates.io; was 0.4.4), taking `httpant-server` 0.5.22 to 0.7.10 in the lockfile. Verified: `cargo clippy --all-targets --all-features -- -D warnings` clean and `cargo test --all-features` pass (186 + 5 + 2), unchanged from 0.4.4.
