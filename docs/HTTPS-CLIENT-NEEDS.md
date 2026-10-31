# What WebHostAnt needs from the HTTPAnt client for https (need note)

Written 2026-10-31 by the WebHostAnt lane, against WebHostAnt `origin/main` `ee3816a` and HTTPAnt `origin/main` `1e926bd` (0.5.19: client matrix says TLS `no`;
README: an `https://` URL is an error). For the HTTPAnt lane and the PM. **Needs, not a design.** WebHostAnt would be the client's first consumer.
Facts about Cloudflare's API are UNVERIFIED (`docs/CLOUDFLARE-FACTS.md`: the endpoint paths were never read).

## Hello-world first (what WebHostAnt would write)

```rust
// Intended, not built. Two uses: ask Cloudflare's API, and probe the deployed app's health path.
let api = httpant::Client::builder().timeout(Duration::from_secs(30)).build()?;
let r = api.get("https://api.cloudflare.com/client/v4/accounts/ID/workers/domains")
    .bearer(token)                          // token from SecretAnt; never in a URL, log or error
    .send().await?;                         // r.status(), r.headers(), r.json::<T>()
let up = probe.get("https://my-app.example.workers.dev/health").redirects(0).send().await?.status() == 200;
```

## The needs

| # | Need | Level |
|---|---|---|
| 1 | `https://` works: rustls (as the server side already is), TLS 1.2/1.3 only, certificates **verified by default** against a bundled root set, host name checked. | must |
| 2 | Turning verification off exists only as a loudly named option, off by default, refused by default in release builds of WebHostAnt (we will not use it). | should |
| 3 | A way to trust an extra root (a test CA), so WebHostAnt's own tests can run https against a local fake server. Without it the client cannot be tested here. | must |
| 4 | Option to use the operating system's certificate store (schools behind an inspecting proxy install their own root). Off or on by default is HTTPAnt's call; it must work on Windows. | should |
| 5 | GET, POST, PUT, PATCH, DELETE with request headers and a body (JSON bytes); `bearer(token)` or a header whose value is **redacted** in `Debug`, logs and errors. | must |
| 6 | Response gives status, all headers (repeats kept), and the body as bytes with a **size limit** (the existing 2 MiB default is fine); `429` and `Retry-After` reach us unchanged. | must |
| 7 | Timeouts, finite by default: connect, whole request, and idle between reads; each settable. A timeout error says which one fired. | must |
| 8 | Redirects: **not followed by default** (a health probe answered by a redirect to a login page is not "healthy"; we need to see the `3xx`). Opt-in follow with a cap; never https to http; `Authorization` dropped when the host changes. | must |
| 9 | Proxy: an explicit proxy setting (CONNECT for https); reading `HTTPS_PROXY`/`NO_PROXY` only if asked. School networks need it; a missing proxy must not be mistaken for "Cloudflare is down". | should |
| 10 | **Private-address guard** for the health probe (CONSUMER-CONTRACT section 5, threat 4): option to refuse loopback, private and link-local addresses, checked on the address actually connected to (not only the name), including after a redirect. | must |
| 11 | Caller sets `User-Agent`; no retry of non-idempotent requests by the client (retry policy is ours, built on need 6). | should |
| 12 | Errors are the existing kind: one sentence ending with what to do; the token, the URL's query string and header values never appear in them. | must |
| 13 | The client capability row `tls` becomes measured, not declared (today the matrix says it is not measured), by a case against a local https peer with a known CA. | should |

## What stays WebHostAnt's

Which endpoints, token scopes, retry and back-off, deciding what "healthy" means, secret retrieval. The client carries bytes and tells the truth about
the connection; it does not learn Cloudflare.

## Questions back to HTTPAnt

1. Root set: bundled (webpki-roots), OS store, or both? (Needs 1 and 4.)
2. Is the private-address check (10) possible below the hyper connector, so DNS rebinding cannot slip past it?
3. Earliest version with need 1, 3, 5-8, 10, 12 (the must rows)? WebHostAnt depends only on a published version.
