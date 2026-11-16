kind: C8d: an example for the staging reachability run
---
Adds `examples/local_staging.rs`: builds a `LocalTarget` site with `ACME_CONTACT` (authority left at its Let's Encrypt STAGING default), calls `check_reachable` once and prints the answer. `--fake-ca` runs it offline against a local stand-in. Verified offline: exit 0 with the fake CA; with the stand-in made to refuse, exit 1 and its reason printed (mutation applied and reverted); clippy clean. Not yet run against real staging.
