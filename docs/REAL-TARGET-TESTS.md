# Real-target tests (the D6 Cloudflare test account)

`tests/real_cloudflare.rs` is what `examples/deploy.rs` did by hand on 2026-11-08, as a test: create a throwaway Worker, set a setting, set a secret
(a dummy value), delete the secret, make a later change and roll it back, then roll back a first deploy on a second throwaway Worker.

**It is a real run only when `WEBHOSTANT_REAL_TARGET=1`.** On GitHub-hosted runners the flag is not set: the test prints one fixed line and returns, and
the rest of the file (the flag rules, the Workers-list parsing, the throwaway names) is ordinary tests. With the flag set, a missing or blank variable FAILS the
run by name; it never skips. It is not `#[ignore]` and not behind a feature (portfolio rule 5.1g/5.1h): it always compiles.

| Variable | From | Notes |
|---|---|---|
| `WEBHOSTANT_REAL_TARGET` | the runner's job | `1` for a real run |
| `CLOUDFLARE_ACCOUNT_ID` | the environment | not a secret; the git-excluded `webhostant.local.toml` holds the value for a person, CI substitutes its own |
| `CLOUDFLARE_API_TOKEN` | the secrets store (`CLOUDFLARE_WEBHOSTANT_TEST_API_TOKEN`), delivered by the PM's `with-secret` | the one secret; `wrangler` reads the same two variables, so it targets the test account too |

Needs on the runner: Node, `wrangler` 4.141.0 and, for no part of this test, Docker (the Workers are plain, no container).

Safety, in the order a run does it: the account's Workers are counted and fingerprinted first (names are not printed); every Worker made is named
`webhostant-ci-...` and deleted by a guard that also runs on a panic; at the end the fingerprint must be unchanged, so **a leftover Worker fails the run**.
Nothing prints a token, a secret value, a Worker name or the account id. The account needs a workers.dev subdomain (register one once in the dashboard);
the test fails by saying so if it has none.

Run by hand (the PM, with the vault): `with-secret CLOUDFLARE_WEBHOSTANT_TEST_API_TOKEN ... -- cargo test --test real_cloudflare -- --nocapture` with
`WEBHOSTANT_REAL_TARGET=1` and `CLOUDFLARE_ACCOUNT_ID` set. **Nobody runs it on the vault without the PM's go.**

## First real run (PM, 2026-11-09)

The PM ran `cargo test --test real_cloudflare` through the vault on the test account (workers.dev subdomain `webhostant-test-account`) at ref 69a22a0: **5 passed, 0 failed, 347 s**. The account's Workers were 0 before and 0 after, the same fingerprint (`e3b0c44298fc1c14`, the empty list), so cleanup left nothing. An earlier attempt stopped only because the account had no workers.dev subdomain yet (the failure the test names). This is a PM run, not CI: the self-hosted-runner workflow does not exist yet. It exercised Create, SetSetting, SetSecret, DeleteSecret and Restore (a later change, and a first deploy) for real with an API token; containers, domains and `ReadTls` remain unrun.
