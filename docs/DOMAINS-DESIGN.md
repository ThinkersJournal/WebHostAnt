# Setting up domains

**Proposal, nothing built.** Written 2026-10-29 by the WebHostAnt lane against `origin/main` `1b20bcb` (0.0.12). Inputs: `CONSUMER-CONTRACT.md` section 2
(domain/TLS line), `src/target.rs` (`TlsStatus`, `Op::ReadTls`), `src/host.rs` (`plan`, `apply`, `verify`). How Cloudflare binds a custom domain, and what its
output looks like, is **UNVERIFIED** until a real account exists (D6).

## Hello-world first

```rust
let app = AppManifest::new("my-app").domain("app.example.org");   // new: a name the consumer wants served
let plan = host.plan(&app, &config)?;                              // now may contain Step::BindDomain
host.apply(&plan, &secrets)?;                                      // read-back checks the name is bound
assert!(host.tls_status("my-app")?.names.contains(&"app.example.org".to_string()));
```

## What is missing today

A consumer can read which names are bound (`Host::tls_status`, 0.0.12) but cannot ask for one to be bound. The contract promises domain status; it
says nothing a consumer can use to get a name onto the app.

## Proposed additions

1. `AppManifest::domain(name)`: a host name, declared like a setting. Names are validated at declaration (letters, digits, hyphens, dots; no scheme, port,
   path, wildcard or IP literal); a bad name is `Error::InvalidDomain` at `plan`, before the target is touched.
2. `Op::BindDomain` and `Step::BindDomain { name }`. A step names the domain and nothing else: no DNS records, no certificate material.
3. `Observed.domains: BTreeSet<String>`: the names the target says are bound. `plan` adds a `BindDomain` step for each declared name not in it.
   Names bound but not declared are **left alone and reported** (the plan never unbinds; removal is a separate, explicit operation, not designed here).
4. `verify` reads the bound names back and fails with `ReadBackMismatch { details: ["domain <name>"] }` if a declared name is not bound. It does **not**
   require a valid certificate: issuance can lag a bind, so certificate state stays in `tls_status`, where `Renewal::Unknown` already says "not known yet".
5. Rollback: `Snapshot.observed.domains` records the bound names; restore does not unbind a name an apply added (unbinding can take a site down for
   visitors and is not undone by a code restore). `check_restored` therefore does not compare domains, and `Rollback::Restored` is documented as
   "code, settings and secret names" as today. A domain the apply added is listed in `Applied` so the caller can see it.
6. Conformance: new checks run for targets that declare `Support::Yes` for `BindDomain`: bind then read back; second apply is a no-op (idempotent);
   a name held by another app on the host is refused (`NotOurs`-style, never overwritten, per contract rule 3); the fake gains a fault that reports a
   name as bound when it is not, which the suite must catch.

## What stays outside

DNS. WebHostAnt does not edit the person's DNS in this slice. If the host needs a record the person must add (Cloudflare: the zone is usually on the same
account, which may make it automatic; **UNVERIFIED**), `plan` states the record text as an instruction and the bind step reports "pending DNS" as a
named error, not success.

## Decisions for the PM

- Version: this PR is docs only; the code PR that follows is a new capability (0.0.13 or whatever the PM allocates).
- Whether unbind belongs on the roadmap now or waits for a consumer to ask.
