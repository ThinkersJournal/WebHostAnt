//! The Render adapter: a [`Target`] that drives Render's REST API through [`RenderApi`].
//!
//! **Unverified against a real account.** Shapes are from Render's published OpenAPI file (`docs/SECOND-HOST.md`); no Render account exists for us yet.
//! So every operation reports [`Support::Unverified`] until the caller opts in with [`RenderTarget::trust_operations`]. `Restore` rolls back to the
//! deploy that was live at the snapshot and reconciles the variables; `ReadTls` reports the verified domain names and no expiry (Render's schema has
//! none). `DeleteSecret` is `No`.
//!
//! **Loud limitation: [`SECRET_LIMITATION`].** On Render a secret is an ordinary environment variable that the API may show to any holder of the
//! API key; use a dedicated workspace and key.
//!
//! Choices a reader should know:
//!
//! * **Ours.** A service is ours when its variable `WEBHOSTANT_APP` equals the app id ([`MARKER`]).
//! * **Secrets are variables.** Render's variable schema has no secret flag, so a secret is a variable whose name is listed in the variable
//!   `WEBHOSTANT_SECRETS`. Reads discard the value: a secret is reported by name only. Whether Render's API shows a variable's value to a token
//!   holder is **UNVERIFIED**; assume it does.
//! * **One deploy, at the end, that is waited for.** Whether Render redeploys on a variable change by API is **UNVERIFIED**, so the adapter
//!   deploys itself: variable changes are marked pending, and [`Target::finish_apply`] triggers a single deploy and waits until it is live. `Create`
//!   deploys on its own. A failed step leaves the variables changed and the deploy pending; the rollback point restores both.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::Duration;

use crate::cloudflare::MARKER;
use crate::cloudflare_domains::Rest;
use crate::render::{DeployStatus, RenderApi};
use crate::{
    Error, Observed, Op, Presence, Renewal, SecretValue, Snapshot, Step, Support, Target, TlsStatus,
};

/// What anyone using this adapter for secrets must be told. [`RenderTarget::help`] returns it.
pub const SECRET_LIMITATION: &str = "on Render a secret is an ordinary environment variable that the API may show to any holder of the API key; use a dedicated workspace and key";

/// The variable that lists the names (comma-separated) of the variables that are secrets.
pub const SECRETS_MARKER: &str = "WEBHOSTANT_SECRETS";

/// How long to wait for a deploy: `attempts` reads, calling `pause` between them.
pub struct Pacing {
    attempts: u32,
    pause: Box<dyn Fn()>,
}

impl Pacing {
    /// Read up to `attempts` times, sleeping `every` between reads.
    pub fn every(every: Duration, attempts: u32) -> Self {
        Self {
            attempts,
            pause: Box::new(move || std::thread::sleep(every)),
        }
    }

    /// Read up to `attempts` times, calling `pause` between reads (tests pass a no-op).
    pub fn with_pause(attempts: u32, pause: impl Fn() + 'static) -> Self {
        Self {
            attempts,
            pause: Box::new(pause),
        }
    }

    fn limit(&self) -> Duration {
        Duration::from_secs(u64::from(self.attempts) * 5)
    }
}

impl Default for Pacing {
    /// Up to ten minutes: 120 reads, five seconds apart.
    fn default() -> Self {
        Self::every(Duration::from_secs(5), 120)
    }
}

/// A Render web service that runs a container image, driven through [`RenderApi`].
pub struct RenderTarget<R: Rest> {
    api: RenderApi<R>,
    image_path: String,
    plan: String,
    health_path: String,
    probe: Box<dyn Fn(&str) -> bool>,
    pacing: Pacing,
    trusted: BTreeSet<Op>,
    log: RefCell<Vec<String>>,
    /// Variables were changed since the last deploy: `finish_apply` must deploy.
    pending_deploy: bool,
}

/// A service with its variables, if there is one.
type Found = Option<(
    crate::render::Service,
    std::collections::BTreeMap<String, String>,
)>;

fn secret_names(list: Option<&String>) -> BTreeSet<String> {
    list.map(|l| {
        l.split(',')
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .collect()
    })
    .unwrap_or_default()
}

impl<R: Rest> RenderTarget<R> {
    /// `image_path` is the container image to run; `plan` one of [`crate::render::WEB_SERVICE_PLANS`]; `health_path` what Render checks;
    /// `probe` answers whether the app's health path answers for an app id (see `HealthProbe::into_probe`).
    pub fn new(
        api: RenderApi<R>,
        image_path: impl Into<String>,
        plan: impl Into<String>,
        health_path: impl Into<String>,
        probe: impl Fn(&str) -> bool + 'static,
    ) -> Self {
        Self {
            api,
            image_path: image_path.into(),
            plan: plan.into(),
            health_path: health_path.into(),
            probe: Box::new(probe),
            pacing: Pacing::default(),
            trusted: BTreeSet::new(),
            log: RefCell::new(Vec::new()),
            pending_deploy: false,
        }
    }

    /// Change how long a deploy is waited for.
    pub fn with_pacing(mut self, pacing: Pacing) -> Self {
        self.pacing = pacing;
        self
    }

    /// Report these operations as `Yes` instead of `Unverified`. Use only after a real run has passed the conformance suite.
    pub fn trust_operations(mut self, ops: &[Op]) -> Self {
        self.trusted.extend(ops.iter().copied());
        self
    }

    /// Plain-language help for a person choosing or running this adapter: what it needs and the secret limitation.
    pub fn help(&self) -> String {
        format!(
            "Render (web service from a container image). Needs a Render API key and workspace id. Limitation: {SECRET_LIMITATION}."
        )
    }

    fn note(&self, line: &str) {
        self.log.borrow_mut().push(line.to_string());
    }

    /// The service for this app id, and whether its marker says it is ours.
    fn service_and_vars(&self, app_id: &str) -> Result<Found, Error> {
        let Some(service) = self.api.find_service(app_id)? else {
            return Ok(None);
        };
        let vars = self.api.env_vars(&service.id)?;
        Ok(Some((service, vars)))
    }

    /// Wait for a deploy to be live. A failed deploy is an error; running out of reads is `TimedOut`, never success.
    fn wait_live(&self, service_id: &str, deploy_id: &str) -> Result<(), Error> {
        for attempt in 0..self.pacing.attempts {
            if attempt > 0 {
                (self.pacing.pause)();
            }
            match self.api.deploy(service_id, deploy_id)?.status {
                DeployStatus::Live => return Ok(()),
                DeployStatus::Failed(word) => {
                    return Err(Error::Target(format!("the deploy ended as {word}")));
                }
                DeployStatus::Deactivated => {
                    return Err(Error::Target(
                        "the deploy was replaced before it went live".to_string(),
                    ));
                }
                DeployStatus::Pending | DeployStatus::InProgress | DeployStatus::Unknown(_) => {}
            }
        }
        Err(Error::TimedOut {
            what: "deploy".to_string(),
            after: self.pacing.limit(),
        })
    }

    fn redeploy(&self, service_id: &str) -> Result<(), Error> {
        let started = self.api.trigger_deploy(service_id, None)?;
        self.wait_live(service_id, &started.id)
    }
}

impl<R: Rest> std::fmt::Debug for RenderTarget<R> {
    /// Names the adapter and its non-secret settings; never a variable, a secret or the API key.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RenderTarget")
            .field("image_path", &self.image_path)
            .field("plan", &self.plan)
            .field("health_path", &self.health_path)
            .field("trusted", &self.trusted)
            .finish_non_exhaustive()
    }
}

impl<R: Rest> Target for RenderTarget<R> {
    fn name(&self) -> &str {
        "render"
    }

    fn supports(&self, op: Op) -> Support {
        match op {
            Op::DeleteSecret => Support::No,
            _ if self.trusted.contains(&op) => Support::Yes,
            _ => Support::Unverified,
        }
    }

    fn locate(&self, app_id: &str) -> Result<Presence, Error> {
        Ok(match self.service_and_vars(app_id)? {
            None => Presence::Absent,
            Some((_, vars)) if vars.get(MARKER).map(String::as_str) == Some(app_id) => {
                Presence::Ours
            }
            Some(_) => Presence::NotOurs,
        })
    }

    fn observe(&self, app_id: &str) -> Result<Observed, Error> {
        let (service, mut vars) = self
            .service_and_vars(app_id)?
            .ok_or_else(|| Error::Target("the app is not deployed".to_string()))?;
        vars.remove(MARKER);
        let secrets = secret_names(vars.remove(SECRETS_MARKER).as_ref());
        let secret_names_present: BTreeSet<String> = secrets
            .iter()
            .filter(|n| vars.contains_key(*n))
            .cloned()
            .collect();
        for n in &secrets {
            vars.remove(n); // a secret is reported by name only; its value is dropped here
        }
        let domains = self
            .api
            .custom_domains(&service.id)?
            .into_iter()
            .map(|d| d.name)
            .collect();
        Ok(Observed {
            settings: vars,
            secret_names: secret_names_present,
            domains,
            healthy: (self.probe)(app_id),
        })
    }

    fn apply_step(
        &mut self,
        app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
        let result = match step {
            Step::Create => {
                let settings =
                    std::collections::BTreeMap::from([(MARKER.to_string(), app_id.to_string())]);
                self.api
                    .create_web_service(
                        app_id,
                        &self.image_path,
                        &self.plan,
                        &self.health_path,
                        &settings,
                    )
                    .and_then(|made| match made.deploy_id {
                        Some(id) => self.wait_live(&made.service.id, &id),
                        None => self.redeploy(&made.service.id),
                    })
                    .map(|()| "create")
            }
            Step::SetSetting { name, value } => self
                .with_service(app_id, |me, id| me.api.put_env_var(id, name, value))
                .map(|()| {
                    self.pending_deploy = true;
                    "set setting"
                }),
            Step::DeleteSecret { .. } => {
                return Err(Error::Unsupported {
                    target: "render".to_string(),
                    op: Op::DeleteSecret,
                });
            }
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target("no value supplied for a secret".to_string()))?;
                self.with_service(app_id, |me, id| {
                    me.api.put_env_var(id, name, value.expose())?;
                    let mut names = secret_names(me.api.env_vars(id)?.get(SECRETS_MARKER));
                    names.insert(name.clone());
                    let list = names.into_iter().collect::<Vec<_>>().join(",");
                    me.api.put_env_var(id, SECRETS_MARKER, &list)
                })
                .map(|()| {
                    self.pending_deploy = true;
                    "set secret"
                })
            }
            Step::BindDomain { name } => self
                .with_service(app_id, |me, id| {
                    me.api.add_custom_domain(id, name).map(|_| ())
                })
                .map(|()| "bind domain"),
        };
        match result {
            Ok(what) => {
                self.note(what);
                Ok(())
            }
            Err(e) => {
                // The error text comes from Render's message and code or from this crate; neither carries a request body.
                self.note(&format!("a step failed: {e}"));
                Err(e)
            }
        }
    }

    fn finish_apply(&mut self, app_id: &str) -> Result<(), Error> {
        if !self.pending_deploy {
            return Ok(());
        }
        let result = self.with_service(app_id, |me, id| me.redeploy(id));
        match &result {
            Ok(()) => {
                self.pending_deploy = false;
                self.note("deployed once for all variable changes");
            }
            Err(e) => self.note(&format!("the final deploy failed: {e}")),
        }
        result
    }

    fn snapshot(&self, app_id: &str) -> Result<Snapshot, Error> {
        match self.service_and_vars(app_id)? {
            None => Ok(Snapshot {
                existed: false,
                observed: Observed::default(),
                handle: None,
            }),
            Some((service, vars)) if vars.get(MARKER).map(String::as_str) == Some(app_id) => {
                Ok(Snapshot {
                    existed: true,
                    observed: self.observe(app_id)?,
                    // The deploy that is live now: what `restore` rolls back to.
                    handle: self.api.live_deploy(&service.id)?.map(|d| d.id),
                })
            }
            Some(_) => Err(Error::NotOurs {
                app: app_id.to_string(),
            }),
        }
    }

    /// Put the service back to the deploy that was live at the snapshot (`POST /services/{id}/rollback`), then make its variables match the snapshot,
    /// or, when it did not exist then, delete the service this apply created (never one the marker does not show to be ours).
    ///
    /// Whether Render's rollback restores variables is **UNVERIFIED**, so the variables are always reconciled afterwards: a setting is put back to its
    /// snapshot value, a variable made after the snapshot is deleted, and the secret-names variable is put back. A secret that existed then and is gone
    /// now cannot come back (its value is unknown), and one overwritten since keeps the new value; both show up as differences in the read-back.
    /// What is NOT undone: data in any database or disk the service uses, and the custom domains attached since.
    fn restore(&mut self, app_id: &str, snapshot: &Snapshot) -> Result<(), Error> {
        let Some((service, vars)) = self.service_and_vars(app_id)? else {
            return if snapshot.existed {
                Err(Error::Target(
                    "the app is not deployed, so there is nothing to roll back".to_string(),
                ))
            } else {
                Ok(())
            };
        };
        if vars.get(MARKER).map(String::as_str) != Some(app_id) {
            return Err(Error::NotOurs {
                app: app_id.to_string(),
            });
        }
        if !snapshot.existed {
            self.api.delete_service(&service.id)?;
            self.note("delete service");
            return Ok(());
        }
        let Some(deploy) = snapshot.handle.as_deref() else {
            return Err(Error::Target(
                "the snapshot has no live deploy to roll back to".to_string(),
            ));
        };
        let started = self.api.rollback(&service.id, deploy)?;
        self.note("rollback");
        self.wait_live(&service.id, &started.id)?;

        let vars = self.api.env_vars(&service.id)?;
        let mut changed = false;
        for (key, value) in &snapshot.observed.settings {
            if vars.get(key) != Some(value) {
                self.api.put_env_var(&service.id, key, value)?;
                changed = true;
            }
        }
        for key in vars.keys() {
            let wanted = key == MARKER
                || key == SECRETS_MARKER
                || snapshot.observed.settings.contains_key(key)
                || snapshot.observed.secret_names.contains(key);
            if !wanted {
                self.api.delete_env_var(&service.id, key)?;
                changed = true;
            }
        }
        let want_list = snapshot
            .observed
            .secret_names
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join(",");
        match (want_list.is_empty(), vars.get(SECRETS_MARKER)) {
            (true, Some(_)) => {
                self.api.delete_env_var(&service.id, SECRETS_MARKER)?;
                changed = true;
            }
            (false, have) if have != Some(&want_list) => {
                self.api
                    .put_env_var(&service.id, SECRETS_MARKER, &want_list)?;
                changed = true;
            }
            _ => {}
        }
        if changed {
            self.redeploy(&service.id)?;
            self.note("restored variables");
        }
        Ok(())
    }

    /// The names Render reports as verified. Render creates and renews certificates itself for verified domains, and its domain schema carries no
    /// expiry or issuer, so those stay unset and the renewal is `Unknown`; a name that is not verified yet is left out (it has no certificate).
    fn tls_status(&self, app_id: &str) -> Result<TlsStatus, Error> {
        let (service, vars) = self
            .service_and_vars(app_id)?
            .ok_or_else(|| Error::Target("the app is not deployed".to_string()))?;
        if vars.get(MARKER).map(String::as_str) != Some(app_id) {
            return Err(Error::NotOurs {
                app: app_id.to_string(),
            });
        }
        let names = self
            .api
            .custom_domains(&service.id)?
            .into_iter()
            .filter(|d| d.verified)
            .map(|d| d.name)
            .collect();
        Ok(TlsStatus {
            names,
            not_after: None,
            issuer: None,
            last_renewal: Renewal::Unknown,
        })
    }

    fn diagnostics(&self) -> String {
        self.log.borrow().join("\n")
    }
}

impl<R: Rest> RenderTarget<R> {
    fn with_service(
        &self,
        app_id: &str,
        f: impl FnOnce(&Self, &str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let service = self
            .api
            .find_service(app_id)?
            .ok_or_else(|| Error::Target("the app is not deployed".to_string()))?;
        f(self, &service.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloudflare_domains::{Method, RestAnswer, RestCall};
    use crate::conformance::{self, Scenario};
    use serde_json::{Value, json};
    use std::cell::{Cell, RefCell};
    use std::collections::BTreeMap;

    const OWNER: &str = "tea-owner1";

    /// A pretend Render that keeps services, variables, domains and deploys, advances a deploy one step per read, and answers in the schema's shapes.
    #[derive(Default)]
    struct Server {
        services: RefCell<Vec<(String, String)>>, // id, name
        vars: RefCell<BTreeMap<String, BTreeMap<String, String>>>, // by service id
        domains: RefCell<Vec<(String, String)>>,  // service id, name
        deploys: RefCell<BTreeMap<String, usize>>, // deploy id -> reads so far
        next_deploy: Cell<u32>,
        down: Cell<bool>,
        fail_secret_put_once: Cell<bool>,
        deploy_ends_as: RefCell<Option<String>>,
        never_live: Cell<bool>,
        calls: RefCell<Vec<RestCall>>,
        /// The live deploy of each service (set when a deploy is read as live).
        live: RefCell<BTreeMap<String, String>>,
        /// The deploy ids rolled back to, in order.
        rolled_back_to: RefCell<Vec<String>>,
        /// Domain names Render has verified.
        verified: RefCell<std::collections::BTreeSet<String>>,
        /// After this many changing calls (POST, PUT, DELETE) every call fails: the network drops.
        drop_after_changes: Cell<Option<usize>>,
        changes: Cell<usize>,
    }

    fn json_answer(status: u16, v: Value) -> Result<RestAnswer, Error> {
        Ok(RestAnswer {
            status,
            body: v.to_string(),
        })
    }

    impl Server {
        fn holding_other_app() -> Self {
            let s = Self::default();
            s.services
                .borrow_mut()
                .push(("srv-9".into(), "conformance-app".into()));
            s.vars.borrow_mut().insert("srv-9".into(), BTreeMap::new());
            s
        }
        fn down() -> Self {
            let s = Self::default();
            s.down.set(true);
            s
        }
        fn dropping_after(changes: usize) -> Self {
            let s = Self::default();
            s.drop_after_changes.set(Some(changes));
            s
        }
        fn new_deploy(&self) -> String {
            let n = self.next_deploy.get() + 1;
            self.next_deploy.set(n);
            let id = format!("dep-{n}");
            self.deploys.borrow_mut().insert(id.clone(), 0);
            id
        }
        fn sid(path: &str) -> String {
            path.split('/').nth(2).unwrap().to_string()
        }
    }

    impl Rest for Server {
        fn send(&self, call: &RestCall) -> Result<RestAnswer, Error> {
            self.calls.borrow_mut().push(call.clone());
            if let Some(n) = self.drop_after_changes.get()
                && self.changes.get() >= n
            {
                self.down.set(true);
            }
            if self.down.get() {
                return Err(Error::Target("could not reach Render: send".to_string()));
            }
            let body: Value = call
                .body
                .as_ref()
                .map(|b| serde_json::from_str(b).unwrap())
                .unwrap_or(Value::Null);
            if call.method != Method::Get {
                self.changes.set(self.changes.get() + 1);
            }
            let p = call.path.as_str();
            match (call.method, p) {
                (Method::Get, "/services") => {
                    let want = call
                        .query
                        .iter()
                        .find(|(k, _)| k == "name")
                        .unwrap()
                        .1
                        .clone();
                    let rows: Vec<Value> = self
                        .services
                        .borrow()
                        .iter()
                        .filter(|(_, n)| n.contains(&want))
                        .map(|(i, n)| json!({"service": {"id": i, "name": n, "type": "web_service", "ownerId": OWNER}, "cursor": "c"}))
                        .collect();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Post, "/services") => {
                    let name = body["name"].as_str().unwrap().to_string();
                    let id = format!("srv-{}", self.services.borrow().len() + 1);
                    self.services.borrow_mut().push((id.clone(), name.clone()));
                    let mut vars = BTreeMap::new();
                    for e in body["envVars"].as_array().unwrap() {
                        vars.insert(
                            e["key"].as_str().unwrap().to_string(),
                            e["value"].as_str().unwrap().to_string(),
                        );
                    }
                    self.vars.borrow_mut().insert(id.clone(), vars);
                    let dep = self.new_deploy();
                    json_answer(
                        201,
                        json!({"service": {"id": id, "name": name, "type": "web_service", "ownerId": OWNER}, "deployId": dep}),
                    )
                }
                (Method::Get, x) if x.ends_with("/env-vars") => {
                    let sid = Self::sid(x);
                    let rows: Vec<Value> = self.vars.borrow()[&sid]
                        .iter()
                        .map(|(k, v)| json!({"envVar": {"key": k, "value": v}, "cursor": "c"}))
                        .collect();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Put, x) if x.contains("/env-vars/") => {
                    let sid = Self::sid(x);
                    let key = x.rsplit('/').next().unwrap().to_string();
                    if self.fail_secret_put_once.get() && key == "API_TOKEN" {
                        self.fail_secret_put_once.set(false);
                        return json_answer(
                            500,
                            json!({"id": "e", "message": "internal error", "code": "x"}),
                        );
                    }
                    self.vars
                        .borrow_mut()
                        .get_mut(&sid)
                        .unwrap()
                        .insert(key, body["value"].as_str().unwrap().to_string());
                    json_answer(200, json!({"key": "k", "value": "v"}))
                }
                (Method::Get, x) if x.ends_with("/custom-domains") => {
                    let sid = Self::sid(x);
                    let rows: Vec<Value> = self
                        .domains
                        .borrow()
                        .iter()
                        .filter(|(s, _)| *s == sid)
                        .map(|(_, n)| {
                            let status = if self.verified.borrow().contains(n) { "verified" } else { "unverified" };
                            json!({"customDomain": {"id": format!("cdm-{n}"), "name": n, "verificationStatus": status}, "cursor": "c"})
                        })
                        .collect();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Post, x) if x.ends_with("/custom-domains") => {
                    let n = body["name"].as_str().unwrap().to_string();
                    self.domains.borrow_mut().push((Self::sid(x), n.clone()));
                    json_answer(
                        201,
                        json!({"id": format!("cdm-{n}"), "name": n, "verificationStatus": "unverified"}),
                    )
                }
                (Method::Post, x) if x.ends_with("/deploys") => {
                    json_answer(201, json!({"id": self.new_deploy(), "status": "created"}))
                }
                // The fake's rollback leaves the variables as they are, the conservative reading of what is UNVERIFIED about Render.
                (Method::Post, x) if x.ends_with("/rollback") => {
                    self.rolled_back_to
                        .borrow_mut()
                        .push(body["deployId"].as_str().unwrap().to_string());
                    json_answer(201, json!({"id": self.new_deploy(), "status": "created"}))
                }
                (Method::Get, x) if x.ends_with("/deploys") => {
                    let sid = Self::sid(x);
                    let rows: Vec<Value> = self
                        .live
                        .borrow()
                        .get(&sid)
                        .map(|id| {
                            vec![json!({"deploy": {"id": id, "status": "live"}, "cursor": "c"})]
                        })
                        .unwrap_or_default();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Delete, x) if x.contains("/env-vars/") => {
                    let sid = Self::sid(x);
                    let key = x.rsplit('/').next().unwrap().to_string();
                    if let Some(v) = self.vars.borrow_mut().get_mut(&sid) {
                        v.remove(&key);
                    }
                    json_answer(204, Value::Null)
                }
                (Method::Delete, x) if x.matches('/').count() == 2 => {
                    let sid = Self::sid(x);
                    self.services.borrow_mut().retain(|(i, _)| *i != sid);
                    self.vars.borrow_mut().remove(&sid);
                    self.domains.borrow_mut().retain(|(s, _)| *s != sid);
                    self.live.borrow_mut().remove(&sid);
                    json_answer(204, Value::Null)
                }
                (Method::Get, x) if x.contains("/deploys/") => {
                    let id = x.rsplit('/').next().unwrap().to_string();
                    let mut d = self.deploys.borrow_mut();
                    let reads = d.get_mut(&id).unwrap();
                    *reads += 1;
                    let status = if self.never_live.get() {
                        "build_in_progress".to_string()
                    } else if let Some(end) = self.deploy_ends_as.borrow().clone() {
                        if *reads >= 2 {
                            end
                        } else {
                            "build_in_progress".into()
                        }
                    } else if *reads >= 2 {
                        self.live.borrow_mut().insert(Self::sid(x), id.clone());
                        "live".into()
                    } else {
                        "build_in_progress".into()
                    };
                    json_answer(200, json!({"id": id, "status": status}))
                }
                other => panic!("unexpected call {other:?}"),
            }
        }
    }

    fn target(s: Server) -> RenderTarget<Server> {
        let api = RenderApi::new(s, OWNER).unwrap();
        RenderTarget::new(api, "docker.io/org/app:1", "starter", "/health", |_| true)
            .with_pacing(Pacing::with_pause(5, || {}))
            .trust_operations(&[
                Op::Create,
                Op::SetSetting,
                Op::SetSecret,
                Op::BindDomain,
                Op::Restore,
                Op::ReadTls,
            ])
    }

    #[test]
    fn the_conformance_suite_passes_against_the_fake_render() {
        let failures = conformance::run(|scenario| match scenario {
            Scenario::Empty => target(Server::default()),
            // The suite's plan makes five changing calls (create 1, setting 2, secret value 3, secret-names variable 4, the one deploy 5); after the fifth the network drops.
            Scenario::DropsAfterApply => target(Server::dropping_after(5)),
            Scenario::HoldsOtherApp => target(Server::holding_other_app()),
            Scenario::Unreachable => target(Server::down()),
            Scenario::FailsLastStepOnce => {
                let s = Server::default();
                s.fail_secret_put_once.set(true);
                target(s)
            }
            Scenario::SecretsUnsupported => {
                let api = RenderApi::new(Server::default(), OWNER).unwrap();
                RenderTarget::new(api, "i", "starter", "/health", |_| true)
                    .with_pacing(Pacing::with_pause(5, || {}))
                    .trust_operations(&[Op::Create, Op::SetSetting, Op::BindDomain])
            }
        });
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn an_untrusted_adapter_refuses_to_plan() {
        let api = RenderApi::new(Server::default(), OWNER).unwrap();
        let host = crate::Host::new(RenderTarget::new(api, "i", "starter", "/health", |_| true));
        let manifest = crate::AppManifest::new("conformance-app").setting("A");
        let config = crate::Config::from([("A".to_string(), "1".to_string())]);
        assert!(matches!(
            host.plan(&manifest, &config),
            Err(Error::Unsupported { op: Op::Create, .. })
        ));
    }

    fn host_of(s: Server) -> crate::Host<RenderTarget<Server>> {
        crate::Host::new(target(s))
    }

    fn cfg(pairs: &[(&str, &str)]) -> crate::Config {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn restore_and_read_tls_are_unverified_until_trusted_and_delete_secret_is_not_claimed() {
        let api = RenderApi::new(Server::default(), OWNER).unwrap();
        let plain = RenderTarget::new(api, "i", "starter", "/health", |_| true);
        assert_eq!(plain.supports(Op::Restore), Support::Unverified);
        assert_eq!(plain.supports(Op::ReadTls), Support::Unverified);
        assert_eq!(plain.supports(Op::DeleteSecret), Support::No);
        assert_eq!(
            target(Server::default()).supports(Op::Restore),
            Support::Yes
        );
        assert_eq!(
            target(Server::default()).supports(Op::ReadTls),
            Support::Yes
        );
    }

    fn deploy_posts(host: &crate::Host<RenderTarget<Server>>) -> usize {
        host.target()
            .api
            .rest_for_test()
            .calls
            .borrow()
            .iter()
            .filter(|c| c.method == Method::Post && c.path.ends_with("/deploys"))
            .count()
    }

    #[test]
    fn a_plan_with_many_variable_changes_deploys_once_at_the_end_and_waits_for_it() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let before = deploy_posts(&host);
        let m2 = crate::AppManifest::new("my-app")
            .setting("MODE")
            .setting("A")
            .setting("B")
            .secret("TOKEN");
        let second = host
            .plan(&m2, &cfg(&[("MODE", "two"), ("A", "1"), ("B", "2")]))
            .unwrap();
        let secrets = crate::MapSecrets::new().with("TOKEN", "CANARY-once-xyz");
        host.apply_approved(&second, &second.digest(), &secrets)
            .unwrap();
        assert_eq!(
            deploy_posts(&host) - before,
            1,
            "four variable changes, one deploy"
        );
        let server = host.target().api.rest_for_test();
        let live = server.live.borrow()["srv-1"].clone();
        let last = format!("dep-{}", server.next_deploy.get());
        assert_eq!(
            live, last,
            "the one deploy was waited for until it was live"
        );
    }

    #[test]
    fn a_plan_that_changes_no_variable_triggers_no_extra_deploy() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let after_create = deploy_posts(&host);
        let again = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&again, &again.digest(), &crate::MapSecrets::new())
            .unwrap();
        assert_eq!(deploy_posts(&host), after_create);
    }

    #[test]
    fn a_deploy_that_fails_at_the_end_is_an_apply_failure_with_a_rollback_point() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        *host
            .target()
            .api
            .rest_for_test()
            .deploy_ends_as
            .borrow_mut() = Some("build_failed".to_string());
        let second = host.plan(&m1, &cfg(&[("MODE", "two")])).unwrap();
        let err = host
            .apply_approved(&second, &second.digest(), &crate::MapSecrets::new())
            .unwrap_err();
        match err {
            Error::ApplyFailed { rollback_point, .. } => {
                assert!(rollback_point.is_some(), "restore is trusted")
            }
            other => panic!("expected ApplyFailed, got {other:?}"),
        }
    }

    #[test]
    fn a_plan_that_only_adds_a_secret_still_deploys_once() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let before = deploy_posts(&host);
        let m2 = m1.clone().secret("TOKEN");
        let second = host.plan(&m2, &cfg(&[("MODE", "one")])).unwrap();
        let secrets = crate::MapSecrets::new().with("TOKEN", "CANARY-only-xyz");
        host.apply_approved(&second, &second.digest(), &secrets)
            .unwrap();
        assert_eq!(deploy_posts(&host) - before, 1);
    }

    #[test]
    fn a_failed_step_means_no_final_deploy() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let before = deploy_posts(&host);
        host.target()
            .api
            .rest_for_test()
            .fail_secret_put_once
            .set(true);
        let m2 = m1.clone().secret("API_TOKEN");
        let second = host.plan(&m2, &cfg(&[("MODE", "two")])).unwrap();
        let secrets = crate::MapSecrets::new().with("API_TOKEN", "CANARY-fail-xyz");
        assert!(
            host.apply_approved(&second, &second.digest(), &secrets)
                .is_err()
        );
        assert_eq!(
            deploy_posts(&host),
            before,
            "nothing deployed after a failed step"
        );
    }

    #[test]
    fn rolling_back_goes_to_the_deploy_live_at_the_snapshot_and_puts_variables_back() {
        let mut host = host_of(Server::default());
        let m1 = crate::AppManifest::new("my-app").setting("MODE");
        let first = host.plan(&m1, &cfg(&[("MODE", "one")])).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let live_then = host.target().api.rest_for_test().live.borrow()["srv-1"].clone();

        let m2 = crate::AppManifest::new("my-app")
            .setting("MODE")
            .setting("EXTRA")
            .secret("TOKEN");
        let second = host
            .plan(&m2, &cfg(&[("MODE", "two"), ("EXTRA", "x")]))
            .unwrap();
        let secrets = crate::MapSecrets::new().with("TOKEN", "CANARY-render-xyz");
        let applied = host
            .apply_approved(&second, &second.digest(), &secrets)
            .unwrap();
        let point = applied.rollback_point.expect("restore is trusted");
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);

        let back = host.target().observe("my-app").unwrap();
        assert_eq!(back.settings, cfg(&[("MODE", "one")]));
        assert!(
            back.secret_names.is_empty(),
            "the secret made later is gone"
        );
        let to = host
            .target()
            .api
            .rest_for_test()
            .rolled_back_to
            .borrow()
            .clone();
        assert_eq!(
            to,
            [live_then],
            "rolled back to the deploy live at the snapshot"
        );
        let vars = host.target().api.rest_for_test().vars.borrow()["srv-1"].clone();
        assert!(
            !vars.contains_key(SECRETS_MARKER),
            "the secret-names variable is put back"
        );
        assert!(!host.target().diagnostics().contains("CANARY-render-xyz"));
    }

    #[test]
    fn rolling_back_a_first_deploy_deletes_only_a_service_that_is_ours() {
        let mut host = host_of(Server::default());
        let m = crate::AppManifest::new("my-app").setting("MODE");
        let plan = host.plan(&m, &cfg(&[("MODE", "x")])).unwrap();
        let applied = host
            .apply_approved(&plan, &plan.digest(), &crate::MapSecrets::new())
            .unwrap();
        let point = applied.rollback_point.unwrap();
        assert!(!point.snapshot().existed);
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);
        assert_eq!(host.target().locate("my-app").unwrap(), Presence::Absent);

        // A service that is not ours is never deleted, whatever the snapshot says.
        let mut t = target(Server::holding_other_app());
        let snap = crate::Snapshot {
            existed: false,
            observed: Observed::default(),
            handle: None,
        };
        assert!(matches!(
            t.restore("conformance-app", &snap),
            Err(Error::NotOurs { .. })
        ));
        let calls = t.api.rest_for_test().calls.borrow();
        assert!(
            calls.iter().all(|c| c.method != Method::Delete),
            "no delete was sent"
        );
    }

    #[test]
    fn a_snapshot_without_a_live_deploy_cannot_be_rolled_back() {
        let mut t = deployed();
        let snap = crate::Snapshot {
            existed: true,
            observed: Observed::default(),
            handle: None,
        };
        assert!(t.restore("my-app", &snap).is_err());
        assert!(t.api.rest_for_test().rolled_back_to.borrow().is_empty());
    }

    #[test]
    fn read_tls_names_only_verified_domains_and_never_invents_an_expiry() {
        let s = Server::default();
        let mut t = target(s);
        t.apply_step("my-app", &Step::Create, None).unwrap();
        for name in ["a.example.org", "b.example.org"] {
            t.apply_step("my-app", &Step::BindDomain { name: name.into() }, None)
                .unwrap();
        }
        t.api
            .rest_for_test()
            .verified
            .borrow_mut()
            .insert("a.example.org".to_string());
        let status = t.tls_status("my-app").unwrap();
        assert_eq!(
            status.names,
            ["a.example.org"],
            "an unverified name has no certificate yet"
        );
        assert_eq!((status.not_after, status.issuer), (None, None));
        assert_eq!(status.last_renewal, crate::Renewal::Unknown);
        assert!(matches!(
            target(Server::holding_other_app()).tls_status("conformance-app"),
            Err(Error::NotOurs { .. })
        ));
        assert!(
            target(Server::default())
                .tls_status("nothing-here")
                .is_err()
        );
    }

    #[test]
    fn unreachable_is_never_absent() {
        assert!(target(Server::down()).locate("x").is_err());
    }

    #[test]
    fn a_service_without_the_marker_is_not_ours() {
        assert_eq!(
            target(Server::holding_other_app())
                .locate("conformance-app")
                .unwrap(),
            Presence::NotOurs
        );
    }

    fn deployed() -> RenderTarget<Server> {
        let mut t = target(Server::default());
        t.apply_step("my-app", &Step::Create, None).unwrap();
        t
    }

    #[test]
    fn a_secret_is_listed_by_name_only_and_its_value_is_in_no_read_log_or_error() {
        let mut t = deployed();
        t.apply_step(
            "my-app",
            &Step::SetSecret {
                name: "API_TOKEN".into(),
            },
            Some(&SecretValue::new("canary-secret-42")),
        )
        .unwrap();
        t.apply_step(
            "my-app",
            &Step::SetSetting {
                name: "BASE_URL".into(),
                value: "https://x.example".into(),
            },
            None,
        )
        .unwrap();
        let seen = t.observe("my-app").unwrap();
        assert!(seen.secret_names.contains("API_TOKEN"));
        assert_eq!(
            seen.settings.get("BASE_URL").map(String::as_str),
            Some("https://x.example")
        );
        assert!(
            !seen.settings.contains_key("API_TOKEN"),
            "a secret is not a setting"
        );
        assert!(!seen.settings.contains_key(MARKER) && !seen.settings.contains_key(SECRETS_MARKER));
        let everything = format!("{seen:?} {}", t.diagnostics());
        assert!(!everything.contains("canary-secret-42"));
        // Control: the value really was sent (to the variable endpoint, in the body, never in a URL).
        let sent: Vec<_> = t
            .api_calls()
            .into_iter()
            .filter(|c| {
                c.body
                    .as_deref()
                    .is_some_and(|b| b.contains("canary-secret-42"))
            })
            .collect();
        assert_eq!(sent.len(), 1);
        assert!(sent[0].query.is_empty() && !sent[0].path.contains("canary"));
    }

    #[test]
    fn the_target_s_debug_and_help_carry_no_value_and_state_the_limitation() {
        let mut t = deployed();
        t.apply_step(
            "my-app",
            &Step::SetSecret {
                name: "API_TOKEN".into(),
            },
            Some(&SecretValue::new("canary-secret-42")),
        )
        .unwrap();
        let shown = format!("{t:?} {} {}", t.help(), t.diagnostics());
        assert!(!shown.contains("canary-secret-42") && !shown.contains("API_TOKEN"));
        assert!(t.help().contains("use a dedicated workspace and key"));
        assert!(t.help().contains("ordinary environment variable"));
    }

    #[test]
    fn a_failed_secret_put_does_not_echo_the_value() {
        let s = Server::default();
        s.fail_secret_put_once.set(true);
        let mut t = target(s);
        t.apply_step("my-app", &Step::Create, None).unwrap();
        let e = t
            .apply_step(
                "my-app",
                &Step::SetSecret {
                    name: "API_TOKEN".into(),
                },
                Some(&SecretValue::new("canary-secret-42")),
            )
            .unwrap_err();
        assert!(!format!("{e} {e:?} {}", t.diagnostics()).contains("canary-secret-42"));
    }

    #[test]
    fn a_failed_deploy_is_an_error_not_applied() {
        let s = Server::default();
        *s.deploy_ends_as.borrow_mut() = Some("update_failed".into());
        let mut t = target(s);
        let e = t.apply_step("my-app", &Step::Create, None).unwrap_err();
        assert!(e.to_string().contains("update_failed"), "{e}");
    }

    #[test]
    fn a_deploy_that_never_goes_live_times_out_instead_of_succeeding() {
        let s = Server::default();
        s.never_live.set(true);
        let mut t = target(s);
        assert!(matches!(
            t.apply_step("my-app", &Step::Create, None),
            Err(Error::TimedOut { .. })
        ));
    }

    #[test]
    fn a_bound_domain_shows_in_observe() {
        let mut t = deployed();
        t.apply_step(
            "my-app",
            &Step::BindDomain {
                name: "app.example.org".into(),
            },
            None,
        )
        .unwrap();
        assert!(
            t.observe("my-app")
                .unwrap()
                .domains
                .contains("app.example.org")
        );
    }

    impl RenderTarget<Server> {
        fn api_calls(&self) -> Vec<RestCall> {
            self.api.rest_for_test().calls.borrow().clone()
        }
    }
}
