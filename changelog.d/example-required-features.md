kind: Fix: the `local_staging` example builds only with `local`
---
`examples/local_staging.rs` uses `webservant` and `local_target`, both behind the `local` feature, so `cargo check --all-targets` without the feature failed with E0432 (red Portfolio job on main since #57). The example now declares `required-features = ["local"]`. Verified: `cargo check --all-targets` failed before the change, and passes both without the feature and with `--features local`.
