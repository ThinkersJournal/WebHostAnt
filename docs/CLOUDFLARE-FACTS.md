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
domains. Listing is the Workers domains REST API (endpoint path not read).

**Consequence for BindDomain:** the Cloudflare adapter cannot bind or read back a domain through the `wrangler` calls it makes today. Either it edits the
project's own Wrangler config (the consumer owns that file), or it calls the REST API (new dependency: an API token and account id, and an HTTP client
which waits for HTTPAnt). `supports(BindDomain)` and `ReadTls` stay `No`/`Unverified` for Cloudflare until one is chosen. Asked of the PM.
