# What a real Render account showed (and what it did not)

The Render adapter (`src/render.rs`, `src/render_target.rs`) was written from Render's published schema. This file records what a **real run** has
confirmed. Everything not listed under "Confirmed" is still **UNVERIFIED**.

## The run (PM, 2026-11-10)

`examples/render_deploy.rs` at `render-example@6e44811`, run by the PM under `with-secret RENDER_API_KEY` (a Windows Hello approval), on the test
workspace, free plan, image `docker.io/library/nginx:alpine`, `--health-path /`, `--port 80`. Third attempt. Raw output was kept by the PM
(`.lane-state/render-run3.out`).

**Result: PASS.** Services in the workspace before 0, after 0 (account unchanged). In order: first apply (create, one setting) applied and read back;
second apply (two settings changed, one dummy secret added) applied 3 steps and read back, `healthy` true; rollback of that apply read back as matching the
snapshot; the service the run made was deleted.

### The two earlier runs

Runs 1 and 2 **failed**: the first deploy "did not finish within 600 s and was stopped". Cause (found by the PM): the PM's Git Bash rewrote the argument
`--health-path /` into `C:/Program Files/Git/`, so Render was given a health path that is not a URL path and the deploy never finished. It was not an
adapter defect. Run 3 set `MSYS_NO_PATHCONV=1`. Consequences recorded here: the adapter now refuses a health path that is not a URL path
(`valid_health_path`), before any call, and notes each deploy status Render reports when it changes (`deploy status: ...`) so a timeout says what Render
last said. **`--port 80` was in run 3 but not tested without it**, so the guess that nginx on port 80 needs `PORT` set is unverified either way.

## Confirmed by that run

- `GET /services` (the list, with `ownerId`) and `DELETE /services/{id}` work with an API key; the count before and after agree.
- `POST /services` for an image service on the free plan with env vars; the answer was accepted by `service_of`, and the first deploy reached live.
- Setting variables (`PUT /services/{id}/env-vars/{key}`), including the secret-names variable, read back through `GET .../env-vars`.
- One deploy at the end of an apply (`Target::finish_apply`) reached live, and the app answered its health path (`healthy` true).
- Restoring (rollback to the deploy live at the snapshot, then reconciling the variables) ended in a read-back equal to the snapshot.

## NOT shown (still UNVERIFIED)

- Render's own deploy count: the run did not list deploys, so "one deploy" is the adapter's request count, not Render's.
- Custom domains (`BindDomain`, `ReadTls`): not exercised.
- Whether Render redeploys by itself on a variable change by API (the adapter deploys explicitly either way).
- Whether Render's rollback restores variables (the adapter reconciles them regardless).
- The exact status words Render sent (the run predates the status log); `--port` as a cause; paid plans; services other than `web_service`.
- Secret values on Render are plain variables; whether the API shows a variable's value to a token holder stays assumed (the adapter assumes it does).
