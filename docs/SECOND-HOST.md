# The second host: Render, DigitalOcean App Platform and Fly.io, read against the six criteria

**Research only, no code (PM, 2026-10-31).** Written for CireSnave and the PM. Read 2026-10-31.

## The answer first

**Recommendation: Render is the second host to build.** It is the only one of the three where the published API, by its own schema, covers every step WebHostAnt
performs: create the service from a container image, set its variables and secret files, attach a custom domain and ask whether it has been verified, deploy,
read the deploy's state back, roll back, and download a database export. It also has a free plan, which could give us a real-target test account that costs
nothing (the Cloudflare one, D6, probably costs money; that is not stated either way).

Two things stop me calling it finished. A free Render web service sleeps after 15 minutes without traffic, so a school would need a paid plan, and the page I read does not say a paid plan
never sleeps. And Render's variables have no "secret" flag in the schema: whether the API shows a secret's value back to anyone holding the token is not stated.

**DigitalOcean App Platform is a close second** (its secret variables are a typed, encrypted kind and it has a rollback with a check-then-commit step), but I could not read whether
it issues and renews a certificate for an ordinary domain by itself, and I read nothing about how a school gets its data out. **Fly.io is third**: its trial is two hours of machine time or seven days, it has
no free plan, and its API has no rollback call.

The six criteria are DESIGN.md section 3.1's. "Read" means I read it from the source named; everything else is marked **UNVERIFIED** and is not a claim.

## How the facts were read

* **API files:** Render's OpenAPI JSON (`https://api-docs.render.com/openapi/render-public-api-1.json`, 417 KB, parsed, not summarised); Fly's OpenAPI 3.1 JSON
  (`https://docs.fly.io/api/machines/openapi.json`, parsed); DigitalOcean's published OpenAPI repository (`github.com/digitalocean/openapi`, `specification/DigitalOcean-public.v2.yaml`
  is an index of paths, and I read the model files named below one by one).
* **Documentation pages:** fetched and summarised by a model, so their wording is the summariser's, not checked by eye (the same caveat as `CLOUDFLARE-FACTS.md`).
  Each is marked "page".
* A schema is what the vendor says the API accepts and returns, not a recorded live answer. Nothing here was run against an account.

## The six criteria, host by host

| # | Criterion | Render | DigitalOcean App Platform | Fly.io |
|---|---|---|---|---|
| 1 | Deploy and read back by API alone, no vendor CLI | **Read, yes.** Bearer token, root `https://api.render.com/v1`. `POST /services`, `GET /services/{id}`, `POST /services/{id}/deploys`, `GET /services/{id}/deploys/{deployId}` whose `status` is one of `created, queued, build_in_progress, update_in_progress, live, deactivated, build_failed, update_failed, canceled, pre_deploy_in_progress, pre_deploy_failed`; `POST /services/{id}/rollback` (body `deployId`) | **Read, yes (paths).** `POST/GET /v2/apps`, `POST/GET /v2/apps/{app_id}/deployments`, and `POST /v2/apps/{app_id}/rollback` with `/rollback/validate`, `/rollback/commit`, `/rollback/revert` (index file); the rollback file says a rollback pins the app until committed or reverted. Request and response bodies for create and deployments: **UNVERIFIED** (not read) | **Read, yes.** Bearer token, root `https://api.machines.dev`. `POST /v1/apps`, `POST /v1/apps/{app}/machines`, `GET .../machines/{id}`, `GET .../machines/{id}/wait` (wait for a state). **No rollback call**: only `GET .../machines/{id}/versions` |
| 2 | Accepts a container image | **Read, yes.** `POST /services` body has `image {imagePath, ownerId, registryCredentialId}`; `runtime` can be `image`; `/registrycredentials` create/list/update/delete | **Read, yes.** `image` is a component source (`apps_image_source_spec.yml`); `registry_type` is `DOCKER_HUB`, `DOCR` or `GHCR` | **Read, a field exists.** `MachineConfig` has `image`; what registries it pulls from: **UNVERIFIED** |
| 3 | Secrets store reachable by API | **Partly.** `PUT /services/{id}/env-vars/{key}` and `PUT .../secret-files/{key}` exist. The `envVar` schema is only `{key, value}` (no secret flag), so **whether a read shows a secret's value is UNVERIFIED**; secret files are a separate kind | **Read, yes.** Variables have `type: GENERAL` or `SECRET`; "If the type is `SECRET`, the value will be encrypted on first submission" (`app_variable_definition.yml`). That a read never shows the value: **UNVERIFIED** | **Read, yes.** `GET/POST/DELETE /v1/apps/{app}/secrets/{name}` and a list; whether a read returns the value: **UNVERIFIED**. (The Machines docs page lists no secrets resource; the OpenAPI file does.) |
| 4 | Automatic TLS on a custom domain | **Read, yes.** Page: "Render automatically creates and renews TLS certificates for all custom domains, including wildcard domains"; authorities Let's Encrypt and Google Trust Services. `POST /services/{id}/custom-domains` (body `name`); the domain has `verificationStatus` (`verified` or `unverified`) readable by API. Whether the API can *trigger* verification: **UNVERIFIED** (page mentions a dashboard Verify button only). Certificate expiry is not in the domain schema | **Not stated for an ordinary domain.** Page: Let's Encrypt and Google Trust are the authorities; the renewal notices it describes are for wildcard tokens only. **Automatic issue and renewal for a normal domain: UNVERIFIED.** The domain spec has `domain, type, wildcard, zone, minimum_tls_version` and no certificate fields | **Read, yes.** `POST /v1/apps/{app}/certificates/acme`, `GET .../certificates/{hostname}`, `POST .../certificates/{hostname}/check` ("Check DNS and re-validate"). Page: the first 10 single-hostname certificates are free |
| 5 | A plan that does not stop the app on a timer | **Free plan fails it** (page: "goes 15 minutes without receiving any inbound traffic", 750 free instance hours a month, then suspended; a free Postgres expires after 30 days and is deleted after a 14-day grace). **A paid plan never sleeping: UNVERIFIED** (the page read covers the free plan only) | **No timer stated, but not shown absent.** Pricing page: paid containers from $5/month; it says nothing about sleeping or an uptime cap. Absence of a statement is not evidence: **UNVERIFIED** | **Fails for the trial.** Page: the trial "lasts for up to 2 hours of Machine runtime or 7 days"; "New organizations don't have a free tier"; what happens at the limit: not stated. DESIGN.md 3.1 records that the trial stopped Humboldt's machines at five minutes (a note in Humboldt's `worker/wrangler.toml`; recorded there, not re-read now) |
| 6 | Export of data so a school can leave | **Partly.** `POST /postgres/{id}/export` then `GET /postgres/{id}/export` lists exports each with a `url` ("URL to download the Postgres export"). A persistent disk: page names SCP and Magic-Wormhole only; downloading a snapshot is not mentioned (**UNVERIFIED**) | **UNVERIFIED.** Nothing about getting data out was read for App Platform or its databases | **UNVERIFIED.** Volume snapshots (`.../volumes/{id}/snapshots`) and Postgres `backups` exist as API calls; whether either can be downloaded was not read |

## What this means for WebHostAnt

* **Render is buildable through the pieces we already have.** The `Rest` trait and `HttpRest` from the Cloudflare domain work send a bearer-token call through HTTPAnt's client with the
  private-address guard on and redirects off; a Render adapter is another user of them. The marker that says "this service is ours" can be a plain variable, as with Cloudflare's `WEBHOSTANT_APP`.
* **What does not map cleanly:** Render has no certificate-expiry field, so `ReadTls` would report the name and verification status and leave expiry unknown (it stays `Unverified` or
  `No`, never made up). Restore maps to `POST /services/{id}/rollback`, which is a real call, but whether it also restores variables is **UNVERIFIED**.
* **A test account might be free.** The `plan` field accepts `free` for a web service. Whether a *free* service can run an image, and how it behaves when it sleeps during a test, is **UNVERIFIED**.
  If it works, this host could be the first one with real-target tests running in CI without waiting for CireSnave's D6 decision.

## What it would cost to build Render

About five pull requests, each red-first with a fake that answers in the schema's shapes (the same method as the Cloudflare domain work), all gated by the by-hand checklist until an account exists:

1. The Render API model: services, env vars, secret files, custom domains, deploys (list/get/create/rollback), with negative controls for `success`-style failures, wrong-owner and held-by-other cases.
2. `RenderTarget`: `locate`, `observe` (variables, domains), the "ours" marker, never overwriting a service that is not ours.
3. `apply_step`: create from an image, set variables and secret files, attach a domain, trigger a deploy and wait for `live` (a `*_failed` status is an error, never "applied").
4. `Restore` through the rollback call and the data-after-rollback line already in plans; `ReadTls` as far as the schema allows.
5. Conformance run and the by-hand release checklist for Render.

Not included: a Render account and token for real tests (a decision for CireSnave, like D6), any DNS record the school must add by hand, and Render Postgres.

## What I could not read, for the PM to decide whether it matters

* Whether a paid Render service ever sleeps (needed for criterion 5 to pass at all).
* Whether the API shows a secret's value back (decides whether Render's variables can hold secrets under WebHostAnt's rule that a secret is never read back).
* DigitalOcean: certificates for an ordinary domain, rollback and create bodies, any data export.
* Fly: what registries it pulls, whether secrets read back, downloads of snapshots and backups.
* All three: rate limits, and the error a host returns when a name is already taken by someone else (needed for the "never overwrite" rule, as with Cloudflare).

## Known limitation to tell every user (PM ruling 2026-10-31)

On Render a secret is an ordinary environment variable that the API may show to any holder of the API key; use a dedicated workspace and key. WebHostAnt
names such variables in a marker (`WEBHOSTANT_SECRETS`), reports them by name only and never logs a value, but it cannot stop Render's API from showing one.
The adapter's `help()` says this too.
