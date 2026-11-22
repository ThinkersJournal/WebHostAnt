# WebHostAnt

Put a web app on a host, and prove it is there. **Early: the contract and a conformance suite exist; the Cloudflare adapter exists and a plain Worker deploy and rollback were run for real on a throwaway Worker (2026-11-08 and 2026-11-09, `examples/deploy.rs`), but a container deploy is unproven and nothing runs against a real account in CI yet** (`cloudflare::CloudflareTarget`; every operation is `Unverified` until opted in).
Not ready for developer use until it passes the fresh-eyes trial. See `ROADMAP.md` and `docs/`.

```rust
use webhostant::fake::FakeTarget;
use webhostant::{AppManifest, Config, Host, MapSecrets};

let app = AppManifest::new("my-app").setting("BASE_URL").secret("API_TOKEN");
let config = Config::from([("BASE_URL".to_string(), "https://example.invalid".to_string())]);
let mut host = Host::new(FakeTarget::new()); // a real target (Cloudflare) replaces this
let plan = host.plan(&app, &config)?; // inert: shows what would change
print!("{plan}"); // secrets appear by name only
let applied = host.apply(&plan, &MapSecrets::new().with("API_TOKEN", "s3cret"))?;
assert!(applied.read_back.healthy); // Ok means the target's own state matched, not that a command returned
# Ok::<(), webhostant::Error>(())
```

A new host adapter implements `Target` and runs `webhostant::conformance::run` against itself; the suite fails on a plan that acts, an apply that
reports success without acting, a leaked secret, an overwritten foreign app, and an unsupported operation that does not error.
