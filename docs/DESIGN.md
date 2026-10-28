# WebHostAnt design note

**Design only. Nothing here is built.** Written 2026-10-28 against WebHostAnt `origin/main` `1d649cd` (an empty placeholder crate, 0.0.0).
"Read" means read this session in the named repo at `origin/main` (or, for the Humboldt installer note, at the branch `docs/installer-design`,
PR #148, not yet on main). **UNVERIFIED** marks every fact not read this session, including all claims about Cloudflare, Docker, Caddy, Let's
Encrypt, Windows, Linux and every hosting company: those are recalled from general knowledge, not looked up. Not legal advice.

## 0. What was asked

CireSnave, 2026-10-28, first-hand, as relayed in the task:

> *"a WebHostAnt project that supports quick and easy deployment to most major hosts as well as the ability to host the app locally up to, and including, automatically adding TLS certificates from certificate providers like Let's Encrypt. It could be integrated with ConfigurAnt so that if WebHostAnt is part of an app, ConfigurAnt can be used to configure all of its settings."*

> *"SecretAnt should have a web interface for configuration, maintenance, and secrets management. That web interface should be hosted via WebHostAnt. WebHostAnt should be configured via ConfigurAnt. Basically, no matter which piece of our software we're talking about, it should use our other tools where possible, and beneficial."*

On the Humboldt installer: *"it should start by asking where they plan to host our app and then checking that site to see if our app is already there and live at which point it would switch from doing a clean install to reconfiguring what is already there."*

On secrets: *"Any install of any of our software would need to include setting up SecretAnt for the person doing the install"*. The PM's recommended
default (a recommendation, not a ruling) is deploy-time pushing of secrets from SecretAnt into the host's own store.

## 1. Goal and non-goals

**Goal.** A person who is not an operator can put one of our web apps on a host they choose, and keep it healthy, with the least work for us.
Concretely, WebHostAnt answers five questions for any app that opts in:

1. Is the app already there, and which version? (detect)
2. What would change if I deployed now? (plan, shown before anything happens)
3. Make it so, and prove it. (apply, then read back)
4. How do I serve it from my own machine with a real certificate? (local hosting and automatic TLS)
5. How do I undo this? (rollback)

**Non-goals.**

- Not a PaaS and not a hosted service. We run no servers for other people and keep no copy of anyone's data or keys.
- Not a general deployment tool. It deploys apps that follow our contract (section 3.2, section 8). Arbitrary Terraform-style infrastructure is out.
- Not a database provisioner. Humboldt's installer note makes the same cut: it tells the person what to create at a database provider, then checks it
  (INSTALLER-DESIGN section 2, row 3, read). Same here for v1.
- Not a secrets store (that is SecretAnt), not a configuration system (ConfigurAnt), not a web framework (HTTPAnt), not a process supervisor (SupervisorAnt).
- Not an auto-updater, no telemetry, no phone-home.
- No promise of "most major hosts". We promise a small tested list (section 2) and an interface others can fill (section 8).

## 2. Evaluating the PM's cautions

| Caution | Verdict | What the design does about it |
|---|---|---|
| Extract only after two consumers; Humboldt installer first | **Agree on adapters, disagree on the contract.** The first consumer's real needs are not yet known, so adapters must be proven inside the Humboldt installer first. But the second consumer already exists on paper: SecretAnt's web UI needs local hosting and TLS, which is a different half of this project from deploying to a remote host. | The crate starts as **types and a conformance harness only** (stub, section 11). Adapter code is written in the Humboldt installer's tree and moved here when SecretAnt (or any second app) needs the same. Local hosting may be extracted first because its second consumer is nearer. |
| "Most major hosts" is a moving target | **Agree.** Hosting APIs change under us (INSTALLER-DESIGN section 8.1 names the same drift). | Three adapters at first: Docker server, Cloudflare, and local (which also covers a bare VM). A published support matrix, versions pinned, an adapter that fails its conformance run is marked `Unverified` and refused, not guessed. |
| ACME/TLS is easy to get subtly wrong | **Agree strongly.** | Do not write our own ACME client. Section 5 compares wrapping Caddy with an ACME library and recommends Caddy first. |
| ConfigurAnt would need to grow prompts and write-back | **Partly right, and smaller than it sounds.** ConfigurAnt already has a TUI and GUI that edit a file with review and backup, and a `Plan` mechanism (read: its DESIGN items 8 and 13). What is missing is an embeddable question flow and a way to render resolved settings into a target's own form. | Section 6 lists exactly three additions. None is needed for slice 1. |
| Patent sweep (automated deployment, certificate issuance) | **Agree.** The repo's `PATENT-SWEEP.md` already says a sweep is a precondition of the first release with functionality. | Section 14 gives queries. No release with functionality before the sweep exists (PATENT-SWEEP-PROCESS.md, read). |

## 3. What "deploy" means, target by target

Every target implements the same four steps that INSTALLER-DESIGN section 2 (read) gave its drivers: **probe** (what is there), **plan** (what would
change), **apply**, **read back**. We add **rollback** and **set secret** (sections 9 and 7). The app never branches on the target outside its adapter.

### 3.1 The targets, and what each needs from the person

| # | Target | The person must have or do | Our job | Notes and UNVERIFIED facts |
|---|---|---|---|---|
| 1 | **Docker or compose on a server** (school machine, rented VM) | A machine they can log in to; Docker installed; a DNS name pointing at it; ports 80/443 reachable if they want automatic certificates (section 5.4) | Render a production compose file (app, database or URL, TLS front door, restart policy, volumes); start it; read health back | Humboldt's current `docker-compose.yml` is rehearsal only (public passwords, localhost ports, plain SMTP); INSTALLER-DESIGN section 1 says so and says a production compose with TLS does not exist (read). Remote control of Docker over SSH vs running the wizard on the box itself: **decide per slice**; running on the box is simpler and matches "works over SSH on the server that will run it" (INSTALLER-DESIGN 7.1 option A, read). |
| 2 | **Cloudflare Worker + Container** (what Humboldt runs today) | A Cloudflare account; an API token with deploy rights; a hostname (workers.dev or a zone on their account) | Probe the account for the named Worker, deploy, set secrets and vars, read back | Read: `worker/wrangler.toml` deploys `humboldt-pickup` as a Container built from the repo `Dockerfile`, with a Durable Object binding. **UNVERIFIED** (INSTALLER-DESIGN lists the same two): whether Containers is available on a free plan; whether a Container deploys through the public API without `wrangler` and Node. If not, the adapter shells out to `wrangler`, which becomes a pinned external dependency we must test against each release. |
| 3 | **A bare VM or on-premises Windows or Linux server, no Docker** | Administrator rights on the machine; a DNS name; ports as in 5.4 | This is the **local** adapter (section 5): install the app binary, a service definition (systemd unit or Windows service, **UNVERIFIED** details), the TLS front door, and a firewall note | Needs SupervisorAnt or generated service files (section 7.4). Windows service handling is where we know least; nothing about it has been read. |
| 4 | **One or two PaaS hosts** | An account, a token, a container image or a build source | Same four steps through the host's API | **No evidence has been read for any PaaS.** The only read fact: Fly.io's **trial** plan killed Humboldt's machines at five minutes of uptime, reproduced and recorded, and the Fly app was deleted 2026-10-10 (comment at the top of `worker/wrangler.toml`, read). That shows trial limits can make a host unusable for a real app; it says nothing about paid Fly. Candidates to evaluate later by the criteria below: Fly.io (paid), Render, Railway, DigitalOcean App Platform (all **UNVERIFIED**). **Recommendation: do not pick a PaaS now; add one when a real school asks for it.** |

**Criteria to pick a PaaS later (all must be shown, not recalled):** deploy and read back by API alone with no vendor CLI; accepts a container image;
has a secrets store reachable by API; automatic TLS on a custom domain; a plan that does not stop the app on a timer; export of data so a school can leave.

Deferred with no promise: Kubernetes, AWS or Azure app services, serverless without containers, Heroku-style buildpacks.

### 3.2 The contract an app must meet to be deployable

Read from INSTALLER-DESIGN and generalized (these are proposals, not built):

- It listens on one HTTP port, configured by a setting, and answers a health path (`/health` exists in Humboldt and returns only `{"status":"ok"}`, read).
- It exposes the **authenticated configuration-retrieval endpoint** of section 4.
- It declares its settings in a `configurant.toml` schema and marks which values are secret (section 6).
- It states its data, so adapters know what a rollback must not destroy (section 9).
- Migrations: forward-only is allowed but must be declared (Humboldt's are; older code refuses a newer schema, INSTALLER-DESIGN 6.1, read).

## 4. Detecting an existing install, safely

### 4.1 Presence: the host first, then an authenticated configuration request

**Withdrawn 2026-10-28:** an earlier version of this section (and INSTALLER-DESIGN 3.2) had an unauthenticated signature endpoint; CireSnave asked, first-hand,
*"why would an 'Is our app here?' respond at all to unauthenticated users?"* — an unauthenticated answer only lets an attacker fingerprint the app. Presence is now:

1. **Host-side.** Ask the host's API, with the host credential the installer already holds, whether the app is deployed (Cloudflare: list the Worker). The app takes no part.
2. **Authenticated configuration retrieval.** One endpoint on the app, answered only to an authenticated administrator or an operator token, returning the app id, version,
   schema version and the full non-secret configuration. An unauthenticated request gets the app's normal response (401 or its public page) and nothing about version or schema.

The never-list (school name, counts, addresses, people, secrets, build SHA, host name, database name or provider) still applies to the *unauthenticated* response, and the
conformance suite tests it: the app seeds data, then the test greps an unauthenticated response body for it and for the app id and version (INSTALLER-DESIGN slice 2 does the
same, read). Secrets appear in the authenticated answer by name and set/not-set only. Whether the authenticated answer includes `version` is the app's choice (D5). Full contract: `CONSUMER-CONTRACT.md` 1a.

### 4.2 Presence is never taken from the site's own say-so

A hostile site can claim to be ours, and an installer that then signs in to "reconfigure" would send a district admin's password to the wrong place. So:

- Presence is decided by the **host's API** (4.1 step 1), which no site can forge. The app is never asked to vouch for itself to an unauthenticated caller.
- Hosting-level changes use the **hosting credential against the host's own API** (Cloudflare token to Cloudflare), not against the site's URL. The adapter checks that the deployment it finds in the host API is the one serving that URL (for example, the Worker's route matches) before it sends any sign-in there.
- In-app sign-in goes only to a URL the person typed **and** that the host API confirmed, `https` only, with no redirect to another host (INSTALLER-DESIGN section 3.3, read).
- If the installer ever runs on a server (for example inside SecretAnt's web UI), a request to a person-supplied URL is a request forgery risk. Rule: a server-side request refuses private, loopback and link-local addresses unless the operator turned that on, and resolves the name once and connects to that address.

### 4.3 The decision table

From INSTALLER-DESIGN 3.3 (read), changed where 4.1 requires: **host says nothing deployed under that name and the data store is empty** means clean install; **host says ours** (and, once
authenticated, the app's answer agrees) means reconfigure or upgrade and **never** a clean-install or wipe tool; a deployment that is not ours means stop; ours but
`needs-install` means finish; unreachable means say which of network, TLS or DNS failed. Its rule about control hosts carries over: **"nothing there" is believed only if a
control URL known to answer is reachable from the same machine in the same run.** Without that, a laptop on a school network that blocks outbound traffic would report "not installed"
and then try a clean install over a live site.

## 5. Local hosting and automatic TLS

### 5.1 Why TLS is not optional

Humboldt's `WEBAUTHN_RP_ORIGIN` must be an `https://` address and `WEBAUTHN_RP_ID` cannot change later without invalidating every passkey
(INSTALLER-DESIGN section 2, read). So the public name must be settled before the first certificate. Browsers treat `localhost` as a secure context
for passkeys (**UNVERIFIED**); that matters for SecretAnt's first run (section 7.1), not for a school.

### 5.2 The three ways to prove you own a name (all UNVERIFIED, recalled)

| Challenge | Needs | Fails when |
|---|---|---|
| HTTP-01 | Port 80 reachable from the internet at the name | Behind NAT with no port forward; a school firewall; a carrier that blocks 80 |
| TLS-ALPN-01 | Port 443 reachable | Same; also cannot issue wildcards |
| DNS-01 | A DNS API token for the zone (or a delegated challenge name) | The DNS provider has no API; the token is too powerful (it can edit every record) |

Also recalled and **UNVERIFIED**: Let's Encrypt applies rate limits, offers a staging environment for tests, and will not issue for private IP
addresses or names under non-public suffixes like `.local`. Certificates are short-lived, so renewal must be automatic and monitored, with a
renewal window well before expiry.

### 5.3 Build or adopt

| Option | For | Against |
|---|---|---|
| **A. Wrap Caddy** as the TLS front door (a separate process we configure and supervise) | Automatic issuance and renewal is its core feature; supports several challenge types and DNS providers by modules; production-proven; no ACME code in our tree (all **UNVERIFIED**, none read) | A second binary to ship, pin, sign and update; its configuration is a different language to render; a DNS provider module may need a custom Caddy build; its licence and the modules' must be checked before we redistribute (Apache-2.0 is recalled, **UNVERIFIED**) |
| **B. In-process ACME with a Rust crate** (`rustls-acme` for TLS-ALPN-01 inside the app's own listener; `instant-acme` as a client library) | One binary; the certificate is held by the same process that serves it; fits our Rust-only dependency rule | We write and own renewal scheduling, certificate storage, account-key handling, challenge serving, failure alerts and hot reload; `rustls-acme` is recalled as TLS-ALPN-01 only (needs 443), `instant-acme` as a bare client (**UNVERIFIED**); maintenance state of both not checked |
| **C. A trait with both behind it** | Chooses by capability like the rest of the family; a school gets Caddy, a single-binary user gets B | Two implementations to keep honest; the conformance suite doubles |

**Recommendation: A for the first release of the Docker and local adapters, designed behind the trait of option C, and B only if a real consumer needs a
single binary.** Reasons: certificate handling is where subtle mistakes cost the most (misissuance, rate-limit lockouts, silent expiry); Caddy
shifts that to a tool used by many people. Before any code: read each candidate's licence, release history, open security advisories and (for crates) check them on crates.io, then record the result next to this note. Nothing in this paragraph was verified.

The trait needs only: `ensure_certificate(names, challenge)`, `status()` (not-after date, issuer, last renewal result), `renew_now()`, and an event
stream for AuditAnt. The adapter's capability rows (section 8) say which challenge types each provider supports.

### 5.4 What a school can realistically do

| Situation | What works | What the wizard says |
|---|---|---|
| Server has a public IP, ports 80/443 open to the internet | HTTP-01 or TLS-ALPN-01, fully automatic | Nothing hard. Offer it first |
| Server is behind NAT; IT will forward 80/443 | Same, after the forward | Explain what to ask IT, in one paragraph |
| Server is behind NAT; IT will not open ports | DNS-01 if the school's DNS provider has an API and they will create a limited token; or a tunnel service (for example Cloudflare Tunnel, **UNVERIFIED**) | Say plainly which of these exists for their provider, and that the token should be limited to one name where the provider allows |
| Intranet only, no public name | A public CA will not issue for private names (**UNVERIFIED**); use a private CA the district already trusts, or a name under a domain they own with DNS-01 | Passkeys need a trustworthy origin, so a self-signed certificate that every browser rejects is not an answer |
| No technical staff at all | Hosting on Cloudflare (target 2) or another managed host, not local | Recommend that first. Local hosting is for people with someone to call |

A port-80/443 check from outside is itself a problem: the machine cannot test its own reachability from inside the network. Plan: the wizard
asks the CA's **staging** environment to issue once and reports the result, rather than inferring reachability from local checks.

## 6. ConfigurAnt integration

### 6.1 What ConfigurAnt gives today (read, `origin/main`)

A TOML schema (`configurant.toml`), layers default < file < environment < command line, named errors, `explain`, a `Module` interface
(`list_choices`, `validate`, `on_change -> Plan`), a `Plan` that is data and is run by a front end only after confirmation, TUI and GUI editors that
edit a file with review and a `.bak`, and secret sources (`source = "<name>:<ref>"`, with a built `file` source reading a directory named by
`CONFIGURANT_SECRETS_DIR`). Secrets are enforced as environment-or-source only and are never shown.

### 6.2 The split: settings vs secrets

| Kind | Examples | Lives in |
|---|---|---|
| Plain settings about the deployment | target id, public hostname, ports, ACME contact address, challenge type, staging flag, data directory, health path, backup directory | WebHostAnt's own `configurant.toml`; the person edits it through ConfigurAnt's front ends |
| Settings the **app** has, which a host must receive | Humboldt's `APP_BASE_URL`, `WEBAUTHN_*`, feature flags | The app's `configurant.toml`; WebHostAnt only **projects** them (6.3) |
| Secrets | host API tokens, ACME account key, DNS token, database URL, `TOTP_ENCRYPTION_KEY` | SecretAnt, delivered through a ConfigurAnt secret source or the environment (section 7). Never in the plan file |

WebHostAnt registers as a ConfigurAnt **module** (`provider = "webhostant"`): `list_choices` answers "which targets satisfy these needs", `validate`
checks a hostname or target id, and `on_change` returns a `Plan` (switch target, change name, renew). That reuses the pattern ConfigurAnt already
uses for PersistAnt (DESIGN item 12, read) and keeps the front end the only thing that executes.

### 6.3 What ConfigurAnt must gain (three additions, ordered)

1. **Projection.** A function that takes a resolved config and renders the non-secret values into a target's form: an env map, a `[vars]` block, a
   compose `environment:` list. Secrets are rendered only as **references** (the `source` string), never values. Needed by slice 1 of the real adapters. Small.
2. **An embeddable question flow.** The existing TUI/GUI are whole programs that edit a file. The installer wants to ask a few questions in its own
   wizard and get validated answers back. This is the "prompts" the PM named. Medium. Until it exists, the Humboldt installer writes its own prompts
   and calls `validate`.
3. **Write-back to non-file places.** Today a confirmed edit yields new file text and a `Plan`. A deployment's "current value" lives on the host (a Worker
   var, a container env). Reading it back and comparing it with the file is what makes reconfigure honest. This is a new kind of source for **non-secret**
   values (read-only at first). Medium to large; schedule only after the Humboldt installer shows it is needed.

ConfigurAnt's roadmap row 11 ("Other modules: ports, paths, logging", open, no design) overlaps with this; the ports module is a natural first piece to share.

## 7. SecretAnt integration

SecretAnt is a placeholder with no design on main: its README says "one interface for apps, adapters to host-native stores, and deploy-time delivery" and
its ROADMAP says a design note is "in review" (both read). Everything below is therefore a requirement written for that design, not a description of it.

### 7.1 The first-secret problem (a cycle, and how to break it)

The cycle: SecretAnt's web UI is hosted by WebHostAnt; WebHostAnt is configured by ConfigurAnt; WebHostAnt needs secrets (an ACME account key, host
tokens); the secrets live in SecretAnt, which is not running yet. Break it in three rules:

1. **WebHostAnt must work with no SecretAnt.** It reads its secrets from the environment or ConfigurAnt's `file` source (read), exactly as every other tool does today. SecretAnt is an upgrade, not a prerequisite.
2. **SecretAnt's first run is local, loopback only**, served by WebHostAnt on `127.0.0.1` without a public certificate, with a one-time token and no remote bind option; this is the same pattern as INSTALLER-DESIGN's option B (read). The person creates their master secret there. Whether browsers allow the needed features on loopback is **UNVERIFIED**.
3. **Only after that** does the person turn on a public name and a certificate; the ACME account key is then created and stored in SecretAnt.

The master secret itself (how SecretAnt is unlocked) is SecretAnt's problem and must not be solved here. We only need: an unlock that does not require
the web UI to be reachable from the internet.

### 7.2 Deploy-time push, per target (the PM's recommended default)

Push means: the deploy tool reads the secret from SecretAnt on the deployer's machine and writes it into the host's own store, so the running app
needs no SecretAnt access. Cost: the secret now exists in two places, and the host's store is only as safe as the host.

| Target | Where the secret goes | Notes |
|---|---|---|
| Cloudflare | The Worker's secret store, by API or `wrangler` (**UNVERIFIED**) | Humboldt already lists its secrets in code (`install::SECRETS`, checked against the Worker's `Env` by a test, read). Reuse that list as the contract |
| Docker server | A directory of files mounted at a secrets path, mode 0600, which ConfigurAnt's `file` source reads (read: bare file names, 64 KiB cap, group/other-readable files refused on Unix) | Avoid putting secrets in `environment:` of the compose file, which is readable by `docker inspect` (**UNVERIFIED**) |
| Bare VM, Linux | The same directory, owned by the service user; or the service manager's credential feature (**UNVERIFIED**) | |
| Bare VM, Windows | A protected file with a restricted ACL, or the OS credential store (**UNVERIFIED**) | ConfigurAnt's `file` source checks Unix permission bits only (read); Windows needs its own check |
| PaaS | The host's secret or environment store by API | Per host |

**Alternative:** the app fetches from SecretAnt at start (pull). More moving parts and a runtime dependency on a secrets server. Not recommended for
a school's first install.

### 7.3 Rules

- A secret never appears in a plan file, a log line, an audit event, a command line or a process listing. Tests use a canary value and assert booleans, never printing the canary (the technique in ConfigurAnt's SECRET-SOURCES S1 to S10, read).
- Pushing uses the same dry run: the plan lists secret **names** and target stores, never values.
- After a push, read back **that the name exists** (and, where the host allows, a fingerprint), never the value.
- A rotated secret is pushed, the app restarted, health read back, and only then is the old one revoked.
- Each target adapter lists in the capability matrix whether it can set, list names, and delete a secret.

### 7.4 What else we need from the other ants (all UNVERIFIED about their state)

| Ant | Use here | State read |
|---|---|---|
| HTTPAnt (core, server, client) | The SecretAnt web UI and the configuration-retrieval endpoint are written against its server half, so WebHostAnt hosts whichever adapter is chosen. WebHostAnt itself does not wrap HTTP servers | Roadmap row "confirmed; repo to be moved"; no checkout exists locally; shape **UNVERIFIED** |
| APIAnt | Health, rate limit and versioning for the configuration-retrieval and status endpoints; rate limiting matters because authentication attempts arrive there | Roadmap row "name claimed" |
| SupervisorAnt | Keeps the app and the TLS front door running and starts them on boot in local mode | Roadmap row says survey first; nothing built. **Until it exists, the local adapter generates a service definition for the OS** and says so |
| AuditAnt | Events: plan shown, plan approved, apply, each step, read-back, rollback, certificate issued or failed, secret pushed (names only). Credential identified by fingerprint, never value | "name claimed", nothing read beyond the roadmap row |
| RedactAnt | Wraps tokens and URLs so a tool's output (for example from `wrangler` or Docker) is scrubbed before it is stored or shown; INSTALLER-DESIGN's doctor report has the same need | "name queued" |
| ReleaseAnt | **Does not fit as written**: its README says it publishes Rust crates and workspaces to crates.io. Our artifacts are container images and installer binaries. Either ReleaseAnt grows an artifact mode, or WebHostAnt ships only checksum/provenance checks and leaves releasing to CI. Raise it with the ReleaseAnt lane; do not assume | Read |
| CapAnt | The capability matrix of section 8 | Read: states Yes, No, Unverified; Unverified by default; evidence required |

## 8. Adapter interface and capability matrix

### 8.1 Operations

An adapter is a provider in CapAnt's sense (data, with a separate prober). Operation ids are open strings. The first set:

`detect.list`, `detect.config` (authenticated retrieval), `plan`, `deploy.image`, `deploy.binary`, `env.set`, `secret.set`, `secret.list_names`, `secret.delete`,
`domain.bind`, `tls.auto`, `tls.status`, `health.read`, `logs.read`, `rollback.previous`, `remove`.

Each operation is a pure request to produce a **plan step** (data), then a separate `apply(step)`; no adapter acts during `plan`. Every `apply` returns a
read-back result; "the command exited zero" is never a result (portfolio rule: an exit code is not a state read).

### 8.2 The matrix (expected, not proven)

This table is a **hypothesis from recall** and every cell is UNVERIFIED until a conformance probe turns it into a row with evidence. CapAnt's rule
is that a missing row means `Unverified` and that an unproven `yes` does not satisfy a need (read).

| Operation | Docker server | Cloudflare | Local (VM) | PaaS (unchosen) |
|---|---|---|---|---|
| `detect.list` | by label | by API | by service name | by API |
| `deploy.image` | yes | yes if Containers works (see 3.1) | no | yes |
| `deploy.binary` | no | no | yes | no |
| `secret.set` | file in mounted dir | API | file in dir | API |
| `domain.bind` | proxy config | account DNS or route | proxy config + person's DNS | host-specific |
| `tls.auto` | via the front door | provided by the host | via the front door | provided by the host |
| `rollback.previous` | previous image digest | previous version, if the host keeps one | previous directory | host-specific |

### 8.3 Conformance tests per adapter

One suite, run against every adapter, with a fake target in-process (fast, always on) and a real target (slow). The suite proves, with a
negative control each (a test that must fail when the behaviour is removed):

1. **Plan is inert:** after `plan`, the fake target's recorded state is unchanged.
2. **Read-back is real:** an `apply` that silently did nothing is detected (a mutant adapter returning success without acting fails the suite).
3. **Idempotent:** applying the same plan twice changes nothing the second time.
4. **No secret leaks:** the canary secret is absent from the plan, logs, errors, audit events and argv.
5. **Detect has a control:** probing an unreachable host is never reported as "not installed" without a reachable control.
6. **Not-ours is refused:** a target holding another app is never overwritten.
7. **Rollback restores** the previous artifact and configuration and verifies health.
8. **Unsupported means error:** an operation the adapter declares `No` or `Unverified` returns a named error, never a silent partial.

Real-target runs: Docker and local need a machine with Docker or a service manager, which GitHub-hosted runners may not give; portfolio rule is to run
such tests on the self-hosted runner and not to `#[ignore]` them (CLAUDE.md, read). Cloudflare real runs need a **dedicated test account and token**,
which does not exist (**UNVERIFIED**) and is a decision (section 15).

## 9. Upgrade and rollback

### 9.1 Upgrade (generalized from INSTALLER-DESIGN 6.1, read)

Probe installed `version` and schema, refuse a downgrade, back up and read the backup back, show the migrations that will run and any marked
irreversible, deploy the new artifact, wait for health and for the authenticated configuration answer to report the new version, read back, keep the backup.

### 9.2 Rollback, and its honest limit

- **Code and configuration** roll back cleanly: each adapter keeps the previous artifact identity (image digest, binary directory, host version
  where the host keeps one, **UNVERIFIED**) and the previous rendered configuration, and rollback switches to them and reads health back.
- **Data** does not. Humboldt's migrations are forward-only and older code refuses a newer schema (read). So rollback after a migration committed is
  only possible to a version that accepts the current schema, or by **restoring the backup taken just before**. The plan must say which, before apply.
- On one machine, deploy the new version beside the old and switch the front door only after the new one is healthy; if health fails, do not
  switch. This avoids most rollbacks.
- TLS: a failed renewal never removes the current certificate; a changed name does not delete the old one until the new one is verified.

## 10. Threat model

| Threat | Example | Mitigation |
|---|---|---|
| **Hostile network** | A school network that strips or intercepts TLS; spoofed DNS during the probe | `https` only; verify the certificate chain and name; no redirect to another host; the control-host rule (4.3); never send a credential to a URL the host API did not confirm (4.2) |
| **Hostile site pretending to be ours** | Answers the installer's requests as if it were the app | Presence comes from the host's API, not from the site (4.2); the app is asked only for authenticated retrieval, at a URL the host API confirmed |
| **Server-side request forgery** | A web UI that probes a person-supplied URL | Refuse private, loopback and link-local targets by default; pin the resolved address |
| **Stolen deploy credential** | A leaked Cloudflare token or a DNS token | Use least-privilege tokens and say which rights are needed; credentials are read for one run and not stored (INSTALLER-DESIGN section 4, read); an audit event per use; rotation steps in the docs; DNS tokens limited to one zone where the provider allows |
| **Tampered artifact** | A modified installer binary or image | Published checksums at minimum; a pinned image **digest**, never a tag; build in CI; signing is a money decision shared with the installer (INSTALLER-DESIGN 7.2, read); verify before run. Signature scheme choice **UNVERIFIED** |
| **Tampered plan** | A plan file edited between review and apply | The plan carries a hash; `apply` re-checks it and re-reads the target state |
| **Certificate misissuance or loss** | Wrong name issued; rate-limit lockout; ACME account key stolen | Staging first; issue only for names the person confirmed; account key held in SecretAnt (or a 0600 file before it); alert on any issuance we did not start; renewal failure alerts at a threshold well before expiry; CAA records recommended (**UNVERIFIED** details) |
| **Port exposure** | Opening 80/443 exposes the app and everything on that box | Say so in the wizard; bind the app to loopback and expose only the front door |
| **Supply chain** | A dependency or a Caddy module | `cargo deny` already runs in this repo (`deny.toml`); pin and review any DNS module; licence notices ship with the artifact |
| **Lost keys** | A forgotten master secret | We hold no copy and cannot recover it; the wizard says so before the first install (INSTALLER-DESIGN 8.1, read) |

## 11. Dependency order, and what can start with a stub

Build order follows PORTFOLIO-ROADMAP's suggested order (CapAnt, HTTPAnt core and conformance suite, RedactAnt and AuditAnt, SecretAnt ...), read.
Nothing in this project needs to wait for them to **start**, if each dependency is a stub behind a small trait:

| Dependency | Stub for now | Real thing replaces it when |
|---|---|---|
| CapAnt | A hard-coded table of `Yes/No/Unverified` in our own test types | CapAnt is on crates.io (its R2 is the engine; design only today, version 0.0.7) |
| ConfigurAnt | A plain TOML struct in this crate | Slice 1 below uses ConfigurAnt directly; it is published (0.2.25 on crates.io per its roadmap row 17, held for the PM's batch; **UNVERIFIED** current state) |
| SecretAnt | Environment variables and the `file` source | SecretAnt design and adapter crate exist (its adapter row is held on SecretAnt being on crates.io, read) |
| AuditAnt, RedactAnt | A trait that receives events and a wrapper type whose `Debug` shows nothing | Both published |
| SupervisorAnt | Generated service files | Its survey and first slice |
| HTTPAnt | None: WebHostAnt does not use it in slice 1 | The SecretAnt UI and the configuration-retrieval endpoint are written |
| Caddy | A version-pinned download in a documented path, checked by hash | Option C trait gains a second provider |

## 12. Effort slices

Sizes are guesses (**UNVERIFIED**): S is days, M is a couple of weeks, L is longer than that.

| # | Slice | Size | Needs |
|---|---|---|---|
| 0 | Decisions in section 15; patent sweep rows run (section 14) | S | CireSnave |
| 1 | Contract crate: operation ids, plan-step types, capability rows, the conformance suite harness, a fake target; no real adapter | M | CapAnt stub |
| 2 | Authenticated configuration-retrieval spec and a test helper (the seed-and-grep never-list test, plus a test that an unauthenticated request learns nothing) | S | none |
| 3 | The Docker-server adapter, built **inside the Humboldt installer first**, against the rehearsal stack; production compose with Caddy | L | Humboldt slices 1 and 3 |
| 4 | Local TLS: the trait and the Caddy provider, staging-CA test, renewal monitor, status | L | slice 1 |
| 5 | Cloudflare adapter, after the two UNVERIFIED facts in 3.1 are checked | M | Humboldt slice 5 |
| 6 | ConfigurAnt: projection (6.3 item 1) and the module | M | slice 1 |
| 7 | SecretAnt push: the `file`-directory path for Docker and local; names-only read-back | M | slice 3, SecretAnt design |
| 8 | Extraction: move proven adapter code out of Humboldt into this crate, once the second consumer is real | M | slices 3, 5 and a second consumer |
| 9 | Local adapter for a bare VM (service files, Windows) | L | slice 4, SupervisorAnt survey |
| 10 | SecretAnt web UI hosting (loopback first run, then public name) | M | SecretAnt design, HTTPAnt |
| 11 | Embeddable question flow and read-only write-back in ConfigurAnt (6.3 items 2 and 3) | L | ConfigurAnt lane |
| 12 | Upgrade, rollback and the signed-artifact path | M | slices 3 and 4 |
| 13 | In-process ACME provider (option B), **only if** a consumer needs one binary | L | slice 4 |

## 13. Open facts to check first (the cheapest risk reduction)

1. Is Cloudflare Containers usable on the plan a school has, and deployable without Node? (INSTALLER-DESIGN raises both; neither was answered.)
2. Caddy: licence, release cadence, security history, the DNS-provider module story, whether it runs unattended on Windows.
3. `instant-acme` and `rustls-acme`: last release, advisories, which challenges, licences.
4. What a typical school or district firewall allows (ask one real IT person; do not guess).
5. Whether the Humboldt Worker deploy can be fully driven by the API.
6. Whether a second consumer other than SecretAnt exists (an app of ours that wants to be deployable).

## 14. Patent search queries to run later

Claim nothing is clear. Run them per `PATENT-SWEEP-PROCESS.md` (queries and dates recorded, closest hits read claim by claim, a verdict per hit),
record in `PATENT-SWEEP.md`, and re-run when slice 4 adds certificate behaviour. Same families as INSTALLER-DESIGN section 9 (read) where they overlap.

1. A deployment client that probes a target address for an existing instance of the software and selects between fresh installation and reconfiguration.
2. Automated deployment of a web application to a plurality of hosting providers through provider-specific adapters selected by capability.
3. Automated issuance, renewal and installation of a TLS certificate by a hosting tool using a domain-validation challenge (HTTP, TLS-ALPN, DNS), including a staging-first check.
4. Pushing secrets from a secrets manager into a hosting provider's secret store at deployment time without exposing them to the deployer's logs.
5. An authenticated configuration-retrieval endpoint of a web application used by a client, after a host-side presence check, to determine version, schema and setup state.
6. Rollback of a deployment by switching to a retained previous artifact after a failed health check, with data-schema compatibility checking.
7. Capability matrix with evidence used to select among deployment providers and reject unproven operations.
8. Hosting a management web interface bound to loopback on first run with a one-time token, then exposing it publicly with an automatically obtained certificate.
9. Generating an audit record of a deployment plan hash, applied steps and read-back verification.

Expect broad, old art around installers, update checkers, PaaS and ACME clients; the sweep, not this list, decides anything.

## 15. Decisions for CireSnave

Each is in plain sentences. "Recommended" is the author's view; the choice is yours.

**D1. How soon do we extract WebHostAnt from the Humboldt installer?**
A. Build the contract and conformance harness here now, but write all adapter code inside the Humboldt installer first and move it after a second consumer exists. B. Build adapters here from the start, and have the Humboldt installer depend on this crate. C. Write nothing here until the Humboldt installer is finished, and extract everything afterwards.
**Recommended: A.** It follows the PM's "extract after two consumers" caution for the risky part (adapters) and still gives SecretAnt and the installer one agreed shape. B risks building the wrong interface before one real use; C wastes the time the design is cheap.

**D2. Which targets come first?**
A. Docker server and Cloudflare only (what Humboldt already needs), with local hosting for a bare machine following. B. Docker, Cloudflare and one PaaS, chosen now. C. Local hosting first, because SecretAnt's web UI needs it.
**Recommended: A.** No PaaS has been looked at, and the only read fact about one (Fly's trial killing the app) argues for caution. Pick a PaaS when a school asks. C is reasonable if SecretAnt's design lands first; local hosting is slice 4 either way.

**D3. How do we get automatic certificates?**
A. Run Caddy beside the app as a separate program we configure. B. Build certificate handling into our own Rust program with an ACME library. C. Put both behind one interface and start with Caddy.
**Recommended: C, starting with A.** Certificates fail quietly and expensively; Caddy is the more tested path. B stays available if someone needs one file to install. This needs the licence and maintenance check in section 5.3 before commitment.

**D4. How are secrets delivered to a host?**
A. Push from SecretAnt into the host's own secret store at deploy time (the PM's default). B. The running app asks SecretAnt for its secrets when it starts. C. The person pastes secrets into the host's dashboard by hand, and we only list the names they need.
**Recommended: A**, with C as the always-available fallback while SecretAnt is not built. B adds a server that must be up and reachable for the app to start, which a school cannot be expected to run.

**D5. Does the authenticated configuration answer report the full version?** (An unauthenticated tier was withdrawn 2026-10-28; see 4.1.)
A. Yes, full version and schema to an authenticated caller, so the installer can tell a school it is out of date. B. Only that the app is ours and a coarse state, even to an authenticated caller. C. Let each app choose, with A as the default.
**Recommended: C**, but note the question now concerns only what an *authenticated* caller sees (withdrawal note in 4.1). A version helps defenders and attackers alike; an app with more sensitive data may omit it even for admins.

**D6. What do we do about the real-target tests that need accounts?**
A. Create a dedicated test Cloudflare account and token held in the secrets store, and run real tests on the self-hosted runner. B. Test only against fakes and run the real target by hand before each release. C. Skip real-target tests for now.
**Recommended: A** when slice 5 starts; B until then. C breaks the standing rule that tests which cannot run on GitHub run on a self-hosted runner.

**D7. Who owns ConfigurAnt's three additions (6.3)?**
A. The ConfigurAnt lane, scheduled as its roadmap rows, with WebHostAnt as the named customer. B. WebHostAnt adds what it needs inside its own crate and offers it upstream later. C. Avoid needing them: the installer asks its own questions and WebHostAnt renders its own output.
**Recommended: A for projection (small, needed early); C for prompts and write-back until the Humboldt installer proves a need.** B would give two question flows to keep consistent.

**D8. Should ReleaseAnt cover app artifacts (images, installer binaries)?**
A. Yes, add an artifact mode to ReleaseAnt. B. No; WebHostAnt only verifies checksums and digests, and CI releases artifacts. C. Decide after the first installer release.
**Recommended: C.** ReleaseAnt's stated scope today is crates.io. We have no released installer to learn from yet, and signing is itself undecided.
