# What WebHostAnt needs from a server's TLS surface (need list for HTTPAnt)

Written 2026-10-30 by the WebHostAnt lane, against `origin/main` `da42863`. For the HTTPAnt lane. **Needs, not a design:** nothing here is built, and
the claims about ACME and rustls behaviour are recalled, not looked up (UNVERIFIED). Rulings behind it: local serving and automatic TLS wait for
WebServAnt, which waits for HTTPAnt (CireSnave 2026-10-28, `docs/EDGE-AND-AGNOSTICISM.md`); **HTTPAnt must not become a TLS stack.**

## The idea in one sentence

HTTPAnt serves requests over a connection it is *handed* a certificate for; something else (WebServAnt in-process, or an outside edge such as
Cloudflare or Caddy) gets, stores and renews that certificate. HTTPAnt needs only a small hole to plug a certificate source into, and a way to say
"I am not terminating TLS."

```rust
// Intended, not built. HTTPAnt side: the app never sees TLS.
let server = httpant::Server::builder()
    .adapter("axum")
    .tls(tls_source)        // a trait object WebServAnt/WebHostAnt supplies; omit = plain HTTP behind an edge
    .build()?;
```

## Who terminates

| # | Need | Level |
|---|---|---|
| 1 | Two modes chosen by configuration, not code: **terminate here** (HTTPAnt's listener speaks TLS, using the supplied source) and **edge terminates** (HTTPAnt serves plain HTTP and trusts forwarded headers only from a configured proxy address). | must |
| 2 | In edge mode, the request exposes the original scheme and client address only when the proxy is a configured trusted one; never from an arbitrary peer. | must |
| 3 | Same app code and same adapter behaviour in both modes. | must |

## Certificate source (the hole HTTPAnt exposes)

| # | Need | Level |
|---|---|---|
| 4 | HTTPAnt defines a **trait** for "give me the certificate (chain and key) for this server name"; it does not implement ACME, storage, account keys or renewal. | must |
| 5 | The trait is called per handshake (or per name), not once at startup, so a new certificate is used by the next connection. | must |
| 6 | The source can be a fixed file pair, a self-signed certificate (for tests), or whatever WebServAnt supplies; HTTPAnt needs no knowledge of which. | must |
| 7 | Private key bytes never appear in HTTPAnt's logs, errors or `Debug` output (secret redaction by type). | must |
| 8 | The trait may be async and may fail; a failure refuses that handshake and is reported, never falls back to an unrelated certificate. | must |

## SNI

| # | Need | Level |
|---|---|---|
| 9 | The server name the client asked for is passed to the certificate source, so one listener can hold several names. | must |
| 10 | A name the source does not know gets a clean refusal (or a configured default), not a crash and not another site's certificate. | must |
| 11 | A client that sends no server name (an IP address) is handled by an explicit, documented rule. | should |

## ALPN

| # | Need | Level |
|---|---|---|
| 12 | Offer `h2` and `http/1.1`, configurable, with the chosen protocol visible to the adapter. | must |
| 13 | Answer the ACME `acme-tls/1` protocol on the same port **when the source asks**, so TLS-ALPN-01 validation works on 443 without a second listener. The challenge certificate comes from the source; HTTPAnt only routes the handshake. | should |
| 14 | Serve a path prefix (`/.well-known/acme-challenge/`) from a handler the source registers, before the app's routes, so HTTP-01 works on port 80. | must |

## Renewal without restart

| # | Need | Level |
|---|---|---|
| 15 | Replacing the certificate for a name takes effect for **new** connections with no restart and no dropped existing connection. | must |
| 16 | A renewal that fails leaves the current certificate in service (WebHostAnt's rule, `docs/DESIGN.md` section 9). HTTPAnt keeps the last good one until told otherwise. | must |
| 17 | Graceful shutdown and listener rebind (port or address change) without losing in-flight requests. | should |

## Listeners and policy

| # | Need | Level |
|---|---|---|
| 18 | Separate configurable listeners for 80 and 443, with an optional redirect from 80 to 443 that leaves the challenge path alone. | must |
| 19 | Minimum TLS 1.2, 1.3 preferred; no weaker option is offered. The cipher and version policy is HTTPAnt's default, not each consumer's. | must |
| 20 | Handshake timeouts, so a client that opens a connection and says nothing cannot hold it forever (same spirit as the limits now on by default). | must |
| 21 | Optional HSTS header, off by default, set by the consumer. | should |

## What HTTPAnt reports (so WebHostAnt can prove the state)

| # | Need | Level |
|---|---|---|
| 22 | Per listener: TLS on or off, protocols offered, and for each name the certificate's not-after date and issuer *as actually served* (read back from the live listener, not from the source's say-so). | must |
| 23 | Events for AuditAnt: certificate swapped, handshake refused (name unknown / source failed), listener rebound. No key material in them. | should |
| 24 | A way to run the same checks against a staging CA, which is the source's concern; HTTPAnt only needs to accept any certificate the source returns. | should |

## What stays outside HTTPAnt

ACME protocol, challenge choice, account keys, certificate storage, renewal scheduling and its alerts (WebServAnt in-process, or an external edge adapter
in WebHostAnt); DNS-01 and its provider tokens; deciding which names to request. If HTTPAnt starts to grow any of these, the seam has moved.

## Questions back to HTTPAnt

1. Which of the six adapters can take a per-handshake certificate callback today, and which only a fixed file pair? (Need 5 and 15 depend on it.)
2. Is there already a TLS library choice inside any adapter that the trait in need 4 must not contradict (rustls vs native-tls)?
3. Can the ALPN `acme-tls/1` routing (need 13) be done by every adapter, or must it be done below them, in a shared listener?
