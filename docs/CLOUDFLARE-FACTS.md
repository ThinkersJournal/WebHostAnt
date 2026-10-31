# Cloudflare facts checked (DESIGN.md section 13, items 1 and 5)

Read 2026-10-29 from Cloudflare's own documentation pages, by fetching them and asking a summariser; the quotes below are what that returned, not
verified against the page by eye. Dated record: re-check before building on it.

| Question | Finding | Source |
|---|---|---|
| Is Containers usable on a free plan? | **Not stated.** The pricing page lists Free with "N/A" for memory, CPU and disk, and bills at Workers Paid rates, so a paid plan looks required, but no page sentence says so. Treat as **unverified, probably paid** | developers.cloudflare.com/containers/pricing/ |
| Can a Container be deployed without Node and `wrangler`? | **The documented path is `wrangler deploy` only.** "On deploy, Wrangler uploads your Worker, builds and pushes the container image with Docker"; "You must have Docker running locally when you run `wrangler deploy`." No REST-API path was found in the getting-started page | developers.cloudflare.com/containers/get-started/ |

**Consequence for the adapter:** plan on shelling out to a pinned `wrangler` (with Docker present on the deploying machine) rather than a pure API
client, and say so in the capability rows: `deploy.image` is `Yes` only through `wrangler`. Whether a deploy by image digest, or by API alone, exists
elsewhere in Cloudflare's docs is **not checked**. A school without Docker and Node on the installing machine cannot use this path; the installer must
say so before it starts.

## Custom domains (read 2026-10-29, same method and caveat)

From developers.cloudflare.com/workers/configuration/routing/custom-domains/: with Wrangler a Custom Domain is created by a `[[routes]]` entry with
`custom_domain = true` in the Worker config, then `wrangler deploy`; the page documents **no `--domain` flag** and **no Wrangler command to list** attached
domains. Listing is the Workers domains REST API (endpoint paths read 2026-10-31, below).

**Consequence for BindDomain:** the Cloudflare adapter cannot bind or read back a domain through the `wrangler` calls it makes today. Either it edits the
project's own Wrangler config (the consumer owns that file), or it calls the REST API (new dependency: an API token and account id, and an HTTP client
which waits for HTTPAnt). `supports(BindDomain)` and `ReadTls` stay `No`/`Unverified` for Cloudflare until one is chosen. Asked of the PM.


## Workers custom-domains REST API (read 2026-10-31 from Cloudflare's published OpenAPI schema)

Source: `https://raw.githubusercontent.com/cloudflare/api-schemas/main/openapi.json` (the file Cloudflare publishes its API reference from), read by
parsing the JSON, not through a summariser. This is the **schema**, not a recorded live answer: what a real account returns is still unchecked (D6).

| Call | Method and path | Request | Success answer |
|---|---|---|---|
| List | `GET /accounts/{account_id}/workers/domains` | optional query: `zone_id`, `zone_name`, `service`, `hostname`, `environment` | `{success, errors, messages, result: [Domain], result_info{count,page,per_page,total_count,total_pages}}` |
| Get | `GET /accounts/{account_id}/workers/domains/{domain_id}` | none | `{success, errors, messages, result: Domain}` |
| Attach | `PUT /accounts/{account_id}/workers/domains` | JSON body; `hostname` and `service` are the fields the schema marks required for the caller; `zone_id`, `zone_name`, `enabled` (default true), `previews_enabled` (default false) are also accepted | `{success, errors, messages, result: Domain}` |
| Detach | `DELETE /accounts/{account_id}/workers/domains/{domain_id}` | none | `{success: true, errors: [], messages: []}` (no `result`) |

`Domain` = `{id, cert_id, enabled, environment (deprecated), hostname, previews_enabled, service, zone_id, zone_name}`; `id`, `cert_id` and `environment`
are read-only. A 4XX answer is `{success: false, errors: [{code >= 1000, message, documentation_url?, source?{pointer}}], messages, result: null}`.
Authentication: bearer API token (`Authorization: Bearer ...`).

Not in the schema, so unknown: whether Attach is idempotent for a hostname already attached; what `errors[].code` means a hostname is held by another
Worker (matters for the "never overwrite a site that is not ours" rule); rate limits. Certificate expiry and issuer are **not** on a Domain (only `cert_id`);
the zone call `GET /zones/{zone_id}/ssl/certificate_packs` exists (query `page`, `per_page`, `status`, `deploy`), its fields were not extracted, and
`ReadTls` stays `Unverified` until they are and a real account has answered.
