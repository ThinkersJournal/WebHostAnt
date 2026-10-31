//! The Render adapter: a [`Target`] that drives Render's REST API through [`RenderApi`].
//!
//! **Unverified against a real account.** Shapes are from Render's published OpenAPI file (`docs/SECOND-HOST.md`); no Render account exists for us yet.
//! So every operation reports [`Support::Unverified`] until the caller opts in with [`RenderTarget::trust_operations`]. `Restore` and `ReadTls` are
//! `No` in this slice.
//!
//! Choices a reader should know:
//!
//! * **Ours.** A service is ours when its variable `WEBHOSTANT_APP` equals the app id ([`MARKER`]).
//! * **Secrets are variables.** Render's variable schema has no secret flag, so a secret is a variable whose name is listed in the variable
//!   `WEBHOSTANT_SECRETS`. Reads discard the value: a secret is reported by name only. Whether Render's API shows a variable's value to a token
//!   holder is **UNVERIFIED**; assume it does.
//! * **Every variable change is followed by a deploy that is waited for**, because whether Render redeploys on a variable change by API is
//!   **UNVERIFIED**. A plan with many variables therefore waits for many deploys; a `Target` hook that deploys once at the end is a follow-up.

use std::cell::RefCell;
use std::collections::BTreeSet;
use std::time::Duration;

use crate::cloudflare::MARKER;
use crate::cloudflare_domains::Rest;
use crate::render::{DeployStatus, RenderApi};
use crate::{Error, Observed, Op, Presence, SecretValue, Step, Support, Target};

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

impl<R: Rest> Target for RenderTarget<R> {
    fn name(&self) -> &str {
        "render"
    }

    fn supports(&self, op: Op) -> Support {
        match op {
            Op::Restore | Op::ReadTls => Support::No,
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
                .with_service(app_id, |me, id| {
                    me.api.put_env_var(id, name, value)?;
                    me.redeploy(id)
                })
                .map(|()| "set setting"),
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target("no value supplied for a secret".to_string()))?;
                self.with_service(app_id, |me, id| {
                    me.api.put_env_var(id, name, value.expose())?;
                    let mut names = secret_names(me.api.env_vars(id)?.get(SECRETS_MARKER));
                    names.insert(name.clone());
                    let list = names.into_iter().collect::<Vec<_>>().join(",");
                    me.api.put_env_var(id, SECRETS_MARKER, &list)?;
                    me.redeploy(id)
                })
                .map(|()| "set secret")
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
            if self.down.get() {
                return Err(Error::Target("could not reach Render: send".to_string()));
            }
            let body: Value = call
                .body
                .as_ref()
                .map(|b| serde_json::from_str(b).unwrap())
                .unwrap_or(Value::Null);
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
                        .map(|(_, n)| json!({"customDomain": {"id": format!("cdm-{n}"), "name": n, "verificationStatus": "unverified"}, "cursor": "c"}))
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
            .trust_operations(&[Op::Create, Op::SetSetting, Op::SetSecret, Op::BindDomain])
    }

    #[test]
    fn the_conformance_suite_passes_against_the_fake_render() {
        let failures = conformance::run(|scenario| match scenario {
            Scenario::Empty | Scenario::DropsAfterApply => target(Server::default()),
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

    #[test]
    fn restore_and_read_tls_are_not_claimed() {
        let t = target(Server::default());
        assert_eq!(t.supports(Op::Restore), Support::No);
        assert_eq!(t.supports(Op::ReadTls), Support::No);
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
