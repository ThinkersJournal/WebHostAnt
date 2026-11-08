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


## Recorded from a real account (2026-11-08, `wrangler` 4.141.0, one throwaway Worker, by hand)

PM-approved run (board 320, D6 option "by hand"): `examples/deploy.rs` on a new Worker `webhostant-check` (no container, no data, a dummy secret value that is not
a secret), under the PM's wrangler login. The account's Workers were counted and fingerprinted before and after (4, `0c269fb02b165936`, both times); the throwaway
Worker was deleted (`wrangler delete`) and `deployments list` then answered code 10007 again. Shapes only; no values, email or account id were recorded.

| Step | What ran | Result |
|---|---|---|
| Nothing trusted | the example with no `--trust` | plan refused (`Unsupported`), as the conformance suite requires |
| `Create` | `wrangler deploy --name ... --keep-vars --var WEBHOSTANT_APP:...` | applied; read back healthy |
| `SetSetting` | the same with `--var NAME:value` | applied; the value read back |
| `SetSecret` | `wrangler secret put` (value on stdin) | applied; the name read back, not the value |
| `DeleteSecret` | `wrangler secret delete` (`y` on stdin) | applied non-interactively; the name gone on read back |

Shapes seen: `deployments list --json` is an array of `{id, source, strategy, author_email, annotations{...}, versions[{version_id, percentage}], created_on}`;
`versions view --json` has `resources.bindings[{name, text, type}]` where a plain setting is `plain_text` and a secret is `secret_text`; `secret list --format json`
is `[{name, type}]` (`[]` when none); a Worker that does not exist answers code 10007. All four match what `src/cloudflare.rs` assumed.

### `Restore`, second run (same day, same Worker name, same login; account again 4 Workers, `0c269fb02b165936`, Worker deleted after)

| Step | Result |
|---|---|
| First deploy, then roll back (`--rollback-after`) | the Worker this apply created is deleted (`wrangler delete --force`); `deployments list` answers 10007 again; the read-back says the target matches the snapshot |
| Create, then apply a setting and a secret, then roll back | settings back to the snapshot; the secret made after it gone; the read-back matches |

Two behaviours the first attempt found (both now handled in `restore`, both pinned by the fake wrangler): (1) `wrangler rollback` puts the version's settings back but
does NOT touch the Worker's secret list, so a secret made after the snapshot stays; (2) once rolled back, the newest *uploaded* version is no longer the *deployed* one, and Cloudflare then refuses
any secret change (`secret delete` fails with code 10215: "the latest version of your Worker isn't currently deployed"). So the secrets the snapshot did not have are deleted BEFORE the rollback.
A secret that existed at the snapshot and was deleted since cannot come back (its value is gone); it shows as a difference in the read-back.

Not covered: a container deploy, domains and `ReadTls` (they need a zone), a token (not OAuth) login, and any plan other than the free workers.dev route.

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
