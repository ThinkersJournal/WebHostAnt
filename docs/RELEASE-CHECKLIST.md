# By-hand release checklist (D6, until a Cloudflare test account exists)

CIRESNAVE-DECISIONS D6: real-target tests need a dedicated Cloudflare test account whose token sits in the secrets store. Until it exists, **before each
published release that touches `src/cloudflare.rs`**, someone with their own Cloudflare account runs this by hand and records the result in the release PR.
Nothing here is automated, and nothing here may use a production account or a token with more than the rights listed.

1. Make a throwaway Worker project (the smallest `wrangler` config; no data stores) and a token limited to that one account: Workers Scripts edit only.
2. Run `cargo test` with the repo's wrangler pin installed (CI already runs the dry-run rows); note the wrangler version.
3. Run the conformance suite (`conformance::run`) against `CloudflareTarget` with `ProcessRunner`, **trusting no operation**. It must refuse the plan.
4. Opt in one operation at a time (`trust_operations`), in this order: `Create`, `SetSetting`, `SetSecret`, `Restore`. After each, read the real
   `wrangler deployments list --json`, `versions view --json`, `secret list --format json` output and compare it with the shapes marked UNVERIFIED in
   `src/cloudflare.rs`; paste the **shapes** (field names), never values, into the PR.
5. Confirm by eye: the secret value appears in no log line, no PR text and no `Debug` output; the "does not exist" wording is code 10007 as assumed.
6. Delete the throwaway Worker and revoke the token. Record both in the PR.
7. A shape that differs from the code is a bug in the code: fix it in the same release or leave that operation `Unverified` and say so in the changelog.

For the custom-domain REST rows (when built) add: the token also needs Workers Routes/Domains edit on one test zone; attach and detach one hostname; record the
real `errors[].code` for a hostname held by another Worker.
