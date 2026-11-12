//! Serve an app from this machine with WebServAnt: a [`Target`] for ROADMAP row C8b.
//!
//! What it does: `Create` registers the app, each `SetSetting` records one setting, and [`Target::finish_apply`] starts the site (or restarts it
//! when it is already running) from the settings in force, once, after every step. `observe` reads the settings it holds and asks the running
//! site, over a real request to the address it is bound to, whether it answers its health path.
//!
//! The settings it takes are three, and any other name is refused when the step is applied, never recorded as if it were applied (WebServAnt
//! has no environment and no secret store):
//!
//! * `FILES`: the folder to serve.
//! * `BIND`: where to listen, as `ip:port`. It must be a loopback address unless [`LocalTarget::allow_non_loopback`] was called or `ACME_CONTACT` is already held (the explicit opt-in to automatic certificates, which need port 80). Without it the
//!   site listens on `127.0.0.1` at a free port.
//! * `PROXY`: one forwarded path, as `/prefix=http://upstream`.
//!
//! `HTTP_BIND` and the `ACME_*` settings (row C8d) turn on automatic certificates: `ACME_CONTACT` is the switch, Let's Encrypt STAGING the default
//! authority. Secrets and domains are declared `No` here. `Restore` (row C8c) puts the settings back to a
//! snapshot and restarts the site from them, or stops the site and forgets the app when it did not exist before.
//!
//! The target owns a small thread runtime that keeps the site alive between calls, and dropping the target stops every site. Do not call it from
//! inside an async runtime (`block_on` would panic): [`crate::Host`] is synchronous.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use webservant::{Acme, AcmeClient, Reachability, RunningSite, Settings, Site};

use crate::{
    Error, Health, HealthProbe, Observed, Op, Presence, SecretValue, Snapshot, Step, Support,
    Target, TlsStatus,
};

const FILES: &str = "FILES";
const BIND: &str = "BIND";
const PROXY: &str = "PROXY";
const HTTP_BIND: &str = "HTTP_BIND";
const ACME_CONTACT: &str = "ACME_CONTACT";
const ACME_STAGING: &str = "ACME_STAGING";
const ACME_DIRECTORY: &str = "ACME_DIRECTORY";
const ACME_ROOT: &str = "ACME_ROOT";
const ACME_STORE: &str = "ACME_STORE";
const ACME_NAMES: &str = "ACME_NAMES";

/// Lets one certificate authority client be shared between the sites a target starts (each start needs its own boxed client).
struct SharedClient(std::sync::Arc<dyn AcmeClient>);

impl AcmeClient for SharedClient {
    fn obtain<'a>(
        &'a self,
        names: &'a [String],
        challenges: &'a webservant::Challenges,
        kind: webservant::ChallengeKind,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<webservant::Issued, webservant::AcmeError>>
                + Send
                + 'a,
        >,
    > {
        self.0.obtain(names, challenges, kind)
    }
}

struct App {
    /// Created by this target. `false` for an id the caller declared taken by something else.
    ours: bool,
    settings: BTreeMap<String, String>,
    running: Option<RunningSite>,
}

/// A [`Target`] that serves an app from this machine. See the module documentation.
pub struct LocalTarget {
    runtime: tokio::runtime::Runtime,
    apps: BTreeMap<String, App>,
    log: Vec<String>,
    health_path: String,
    trusted: BTreeSet<Op>,
    non_loopback: bool,
    acme_client: Option<std::sync::Arc<dyn AcmeClient>>,
}

impl LocalTarget {
    /// A target with no apps. It answers the health path `/` and trusts nothing yet: see [`LocalTarget::trust_operations`].
    pub fn new() -> Result<Self, Error> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .map_err(|e| Error::Target(format!("could not start the local runtime: {e}")))?;
        Ok(Self {
            runtime,
            apps: BTreeMap::new(),
            log: Vec::new(),
            health_path: "/".to_string(),
            trusted: BTreeSet::new(),
            non_loopback: false,
            acme_client: None,
        })
    }

    /// The path `observe` asks for, which must answer with a success or redirect status.
    pub fn with_health_path(mut self, path: impl Into<String>) -> Self {
        self.health_path = path.into();
        self
    }

    /// Declare `ops` as `Yes`. Only `Create` and `SetSetting` are ever served; the rest stay `No`. Until a real run has passed the conformance
    /// suite an adapter's operations are `Unverified` by default, so a plan refuses them.
    pub fn trust_operations(mut self, ops: &[Op]) -> Self {
        self.trusted.extend(ops.iter().copied());
        self
    }

    /// Allow `BIND` to name an address that is not loopback (it then listens beyond this machine). Off by default.
    pub fn allow_non_loopback(mut self) -> Self {
        self.non_loopback = true;
        self
    }

    /// Ask this certificate authority client instead of the built-in one, for every site with `ACME_CONTACT` set. This is how a test supplies a
    /// fake CA (WebServAnt's own seam); a real run does not call it.
    pub fn with_acme_client(mut self, client: impl AcmeClient) -> Self {
        self.acme_client = Some(std::sync::Arc::new(client));
        self
    }

    /// Say that `app_id` is already taken by a site this target did not create: `locate` answers `NotOurs` and nothing is changed there.
    pub fn holding_foreign(mut self, app_id: &str) -> Self {
        self.apps.insert(
            app_id.to_string(),
            App {
                ours: false,
                settings: BTreeMap::new(),
                running: None,
            },
        );
        self
    }

    /// The address the app's site is bound to, if it is running (useful with a free port).
    pub fn address(&self, app_id: &str) -> Option<SocketAddr> {
        self.apps
            .get(app_id)?
            .running
            .as_ref()
            .map(RunningSite::addr)
    }

    fn check(&self, app_id: &str, name: &str, value: &str) -> Result<(), Error> {
        match name {
            FILES if value.trim().is_empty() => Err(refuse(FILES, "is empty")),
            FILES => Ok(()),
            BIND | HTTP_BIND => {
                let addr: SocketAddr = value
                    .parse()
                    .map_err(|_| refuse(name, "is not `ip:port`"))?;
                // `ACME_CONTACT` is the explicit opt-in to listening beyond this machine (PM ruling, option A): a certificate authority has to
                // reach port 80. It must already be held, so set it before a wide `BIND`.
                let opted_in = self.non_loopback
                    || self
                        .apps
                        .get(app_id)
                        .is_some_and(|a| a.settings.contains_key(ACME_CONTACT));
                if !addr.ip().is_loopback() && !opted_in {
                    return Err(refuse(
                        name,
                        "is not a loopback address (set `ACME_CONTACT` first, or call `allow_non_loopback`, to listen beyond this machine)",
                    ));
                }
                Ok(())
            }
            ACME_CONTACT if value.contains('@') && !value.contains(char::is_whitespace) => Ok(()),
            ACME_CONTACT => Err(refuse(ACME_CONTACT, "is not an email address")),
            ACME_STAGING if matches!(value, "true" | "false") => Ok(()),
            ACME_STAGING => Err(refuse(ACME_STAGING, "is not `true` or `false`")),
            ACME_DIRECTORY if value.starts_with("https://") || value.starts_with("http://") => {
                Ok(())
            }
            ACME_DIRECTORY => Err(refuse(ACME_DIRECTORY, "is not a URL")),
            ACME_ROOT | ACME_STORE if !value.trim().is_empty() => Ok(()),
            ACME_ROOT | ACME_STORE => Err(refuse(name, "is empty")),
            ACME_NAMES if value.split(',').all(|n| !n.trim().is_empty()) => Ok(()),
            ACME_NAMES => Err(refuse(ACME_NAMES, "has an empty name in its list")),
            PROXY => match value.split_once('=') {
                Some((prefix, upstream))
                    if prefix.starts_with('/') && upstream.starts_with("http") =>
                {
                    Ok(())
                }
                _ => Err(refuse(PROXY, "is not `/prefix=http://upstream`")),
            },
            other => Err(Error::Target(format!(
                "`{other}` is not a setting a local site takes (FILES, BIND, PROXY, HTTP_BIND and the ACME_* settings); it was not applied"
            ))),
        }
    }

    /// The site the held settings describe, with automatic certificates when `ACME_CONTACT` is held. Let's Encrypt STAGING unless `ACME_STAGING` is
    /// `false` or `ACME_DIRECTORY` names another authority: production is never reached by leaving a setting out.
    fn site_of(&self, app_id: &str, held: &BTreeMap<String, String>) -> Result<Site, Error> {
        let settings = self.settings_of(app_id, held);
        let site = Site::from_settings(&settings)
            .map_err(|e| Error::Target(format!("the settings do not make a site: {e}")))?;
        let Some(contact) = held.get(ACME_CONTACT) else {
            return Ok(site);
        };
        let mut acme = Acme::new(contact.clone());
        if let Some(dir) = held.get(ACME_DIRECTORY) {
            acme = acme.directory(dir.clone());
        } else if held.get(ACME_STAGING).map(String::as_str) != Some("false") {
            acme = acme.staging();
        }
        if let Some(root) = held.get(ACME_ROOT) {
            acme = acme.ca_root_pem_file(root);
        }
        if let Some(store) = held.get(ACME_STORE) {
            acme = acme.store(store);
        }
        for name in held.get(ACME_NAMES).into_iter().flat_map(|v| v.split(',')) {
            acme = acme.also_name(name.trim());
        }
        if let Some(client) = &self.acme_client {
            acme = acme.client(SharedClient(std::sync::Arc::clone(client)));
        }
        Ok(site.acme(acme))
    }

    /// Ask "will automatic certificates work here?" with one real issuance against the configured authority (staging by default), using a
    /// throwaway store, then stop. The app must hold `ACME_CONTACT`. It starts its own listeners at the held `BIND` and `HTTP_BIND`, so with
    /// fixed ports it must be called before the site is started, not while it runs. An `Err` is a problem with the settings or the listeners;
    /// the answer about reachability is in the [`Reachability`].
    pub fn check_reachable(&self, app_id: &str) -> Result<Reachability, Error> {
        let app = self
            .apps
            .get(app_id)
            .filter(|a| a.ours)
            .ok_or_else(|| Error::NotOurs {
                app: app_id.to_string(),
            })?;
        if !app.settings.contains_key(ACME_CONTACT) {
            return Err(Error::Target(format!(
                "`{ACME_CONTACT}` is not set, so there are no automatic certificates to check"
            )));
        }
        let site = self.site_of(app_id, &app.settings)?;
        self.runtime
            .block_on(site.check_reachable())
            .map_err(|e| Error::Target(format!("the reachability check could not run: {e}")))
    }

    fn stop(&mut self, site: RunningSite) -> Result<(), Error> {
        self.runtime
            .block_on(site.shutdown())
            .map_err(|e| Error::Target(format!("could not stop the running site: {e}")))?;
        self.log.push("stopped the running site".to_string());
        Ok(())
    }

    fn settings_of(&self, app_id: &str, held: &BTreeMap<String, String>) -> Settings {
        let mut s = Settings {
            name: app_id.to_string(),
            // A free loopback port unless BIND says otherwise.
            bind: Some(
                held.get(BIND)
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1:0".to_string()),
            ),
            ..Settings::default()
        };
        if let Some(files) = held.get(FILES) {
            s.files = Some(files.into());
        }
        // With automatic certificates the plain-HTTP listener (challenge and redirect) also defaults to a free loopback port.
        if held.contains_key(ACME_CONTACT) {
            s.http_bind = Some(
                held.get(HTTP_BIND)
                    .cloned()
                    .unwrap_or_else(|| "127.0.0.1:0".to_string()),
            );
        }
        if let Some((prefix, upstream)) = held.get(PROXY).and_then(|v| v.split_once('=')) {
            s.proxy.push(webservant::ProxyRoute {
                prefix: prefix.to_string(),
                upstream: upstream.to_string(),
                ..webservant::ProxyRoute::default()
            });
        }
        s
    }
}

fn refuse(name: &str, why: &str) -> Error {
    Error::Target(format!("setting `{name}` {why}; it was not applied"))
}

impl Target for LocalTarget {
    fn name(&self) -> &str {
        "local (WebServAnt)"
    }

    fn supports(&self, op: Op) -> Support {
        match op {
            Op::Create | Op::SetSetting | Op::Restore | Op::ReadTls
                if self.trusted.contains(&op) =>
            {
                Support::Yes
            }
            Op::Create | Op::SetSetting | Op::Restore | Op::ReadTls => Support::Unverified,
            // No secret store or environment, and the site's own name is its only domain.
            Op::SetSecret | Op::DeleteSecret | Op::BindDomain => Support::No,
        }
    }

    fn locate(&self, app_id: &str) -> Result<Presence, Error> {
        Ok(match self.apps.get(app_id) {
            None => Presence::Absent,
            Some(app) if app.ours => Presence::Ours,
            Some(_) => Presence::NotOurs,
        })
    }

    fn observe(&self, app_id: &str) -> Result<Observed, Error> {
        let app = self
            .apps
            .get(app_id)
            .filter(|a| a.ours)
            .ok_or_else(|| Error::NotOurs {
                app: app_id.to_string(),
            })?;
        let healthy = app.running.as_ref().is_some_and(|site| {
            let url = format!("{}{}", site.url().trim_end_matches('/'), self.health_path);
            HealthProbe::new().allow_private(true).check(&url) == Health::Healthy
        });
        Ok(Observed {
            settings: app.settings.clone(),
            secret_names: BTreeSet::new(),
            domains: BTreeSet::new(),
            healthy,
        })
    }

    fn apply_step(
        &mut self,
        app_id: &str,
        step: &Step,
        _secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
        match step {
            Step::Create => {
                if self.apps.get(app_id).is_some_and(|a| !a.ours) {
                    return Err(Error::NotOurs {
                        app: app_id.to_string(),
                    });
                }
                self.apps.entry(app_id.to_string()).or_insert(App {
                    ours: true,
                    settings: BTreeMap::new(),
                    running: None,
                });
                self.log.push("created the app".to_string());
                Ok(())
            }
            Step::SetSetting { name, value } => {
                self.check(app_id, name, value)?;
                let app = self
                    .apps
                    .get_mut(app_id)
                    .filter(|a| a.ours)
                    .ok_or_else(|| Error::NotOurs {
                        app: app_id.to_string(),
                    })?;
                app.settings.insert(name.clone(), value.clone());
                self.log.push(format!("set setting {name}"));
                Ok(())
            }
            Step::SetSecret { .. } | Step::DeleteSecret { .. } | Step::BindDomain { .. } => {
                Err(Error::Unsupported {
                    target: self.name().to_string(),
                    op: step.op(),
                })
            }
        }
    }

    fn finish_apply(&mut self, app_id: &str) -> Result<(), Error> {
        let Some(app) = self.apps.get(app_id).filter(|a| a.ours) else {
            return Ok(());
        };
        // Nothing changed and it is serving: leave it be.
        let site = self.site_of(app_id, &app.settings)?;
        let old = self.apps.get_mut(app_id).and_then(|a| a.running.take());
        if let Some(old) = old {
            self.stop(old)?;
        }
        let running = self
            .runtime
            .block_on(site.start())
            .map_err(|e| Error::Target(format!("the site did not start: {e}")))?;
        self.log.push(format!("started on {}", running.addr()));
        if let Some(app) = self.apps.get_mut(app_id) {
            app.running = Some(running);
        }
        Ok(())
    }

    fn tls_status(&self, app_id: &str) -> Result<TlsStatus, Error> {
        let site = self
            .apps
            .get(app_id)
            .filter(|a| a.ours)
            .ok_or_else(|| Error::NotOurs {
                app: app_id.to_string(),
            })?
            .running
            .as_ref()
            .ok_or_else(|| {
                Error::Target("the site is not running, so it serves no certificate".into())
            })?;
        // The site's name is the app id, which is also the name its certificate is asked for.
        self.runtime.block_on(crate::local::read_tls(site, app_id))
    }

    fn restore(&mut self, app_id: &str, snapshot: &Snapshot) -> Result<(), Error> {
        if self.apps.get(app_id).is_some_and(|a| !a.ours) {
            return Err(Error::NotOurs {
                app: app_id.to_string(),
            });
        }
        if !snapshot.existed {
            if let Some(app) = self.apps.remove(app_id) {
                if let Some(site) = app.running {
                    self.stop(site)?;
                }
                self.log.push("removed the app".to_string());
            }
            return Ok(());
        }
        let held = &snapshot.observed.settings;
        let app = self.apps.entry(app_id.to_string()).or_insert(App {
            ours: true,
            settings: BTreeMap::new(),
            running: None,
        });
        app.settings = held.clone();
        self.log.push("restored the settings".to_string());
        if held.contains_key(FILES) || held.contains_key(PROXY) {
            // `finish_apply` stops what runs and starts the restored settings.
            self.finish_apply(app_id)
        } else {
            // Nothing the restored settings could serve: the site that ran stays stopped.
            if let Some(old) = self.apps.get_mut(app_id).and_then(|a| a.running.take()) {
                self.stop(old)?;
            }
            Ok(())
        }
    }

    fn diagnostics(&self) -> String {
        self.log.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{self, Profile, Scenario};
    use crate::{AppManifest, Config, Host, MapSecrets};

    const ID: &str = "conformance-app";

    /// A folder with an index page, so the health path answers.
    fn site_folder() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("index.html"), "<p>ok</p>").unwrap();
        dir
    }

    fn target() -> LocalTarget {
        LocalTarget::new()
            .unwrap()
            .trust_operations(&[Op::Create, Op::SetSetting, Op::Restore])
    }

    fn files_profile(dir: &tempfile::TempDir) -> Profile {
        Profile {
            setting: (FILES.to_string(), dir.path().display().to_string()),
            secret: None,
        }
    }

    /// The suite's scenarios, made from a real local target plus the faults a local target has no way to have by itself.
    struct Scenic {
        inner: LocalTarget,
        scenario: Scenario,
        changed: usize,
        failed_once: bool,
    }

    impl Scenic {
        fn new(scenario: Scenario) -> Self {
            let inner = match scenario {
                Scenario::HoldsOtherApp => target().holding_foreign(ID),
                _ => target(),
            };
            Self {
                inner,
                scenario,
                changed: 0,
                failed_once: false,
            }
        }

        fn down(&self) -> Result<(), Error> {
            let gone = self.scenario == Scenario::Unreachable
                || (self.scenario == Scenario::DropsAfterApply && self.changed >= 2);
            if gone {
                Err(Error::Target("the local host is not reachable".into()))
            } else {
                Ok(())
            }
        }
    }

    impl Target for Scenic {
        fn name(&self) -> &str {
            self.inner.name()
        }
        fn supports(&self, op: Op) -> Support {
            self.inner.supports(op)
        }
        fn locate(&self, app_id: &str) -> Result<Presence, Error> {
            self.down()?;
            self.inner.locate(app_id)
        }
        fn observe(&self, app_id: &str) -> Result<Observed, Error> {
            self.down()?;
            self.inner.observe(app_id)
        }
        fn apply_step(
            &mut self,
            app_id: &str,
            step: &Step,
            secret: Option<&SecretValue>,
        ) -> Result<(), Error> {
            self.down()?;
            // The suite's plan is create (0) then one setting (1); the last one fails once.
            if self.scenario == Scenario::FailsLastStepOnce
                && self.changed == 1
                && !self.failed_once
            {
                self.failed_once = true;
                return Err(Error::Target("the step failed once".into()));
            }
            self.inner.apply_step(app_id, step, secret)?;
            self.changed += 1;
            Ok(())
        }
        fn finish_apply(&mut self, app_id: &str) -> Result<(), Error> {
            self.down()?;
            self.inner.finish_apply(app_id)
        }
        fn restore(&mut self, app_id: &str, snapshot: &Snapshot) -> Result<(), Error> {
            self.down()?;
            self.inner.restore(app_id, snapshot)
        }
        fn diagnostics(&self) -> String {
            self.inner.diagnostics()
        }
    }

    #[test]
    fn the_conformance_suite_passes_against_a_real_loopback_site() {
        let dir = site_folder();
        let failures = conformance::run_with(&files_profile(&dir), Scenic::new);
        assert!(failures.is_empty(), "{failures:?}");
    }

    fn manifest() -> AppManifest {
        AppManifest::new(ID).setting(FILES).setting(BIND)
    }

    fn config(dir: &tempfile::TempDir, bind: &str) -> Config {
        Config::from([
            (FILES.to_string(), dir.path().display().to_string()),
            (BIND.to_string(), bind.to_string()),
        ])
    }

    #[test]
    fn an_applied_app_answers_a_real_request_on_loopback() {
        let dir = site_folder();
        let mut host = Host::new(target());
        let plan = host
            .plan(&manifest(), &config(&dir, "127.0.0.1:0"))
            .unwrap();
        let done = host
            .apply_approved(&plan, &plan.digest(), &MapSecrets::new())
            .unwrap();
        assert!(done.read_back.healthy);
        let addr = host.target().address(ID).expect("running");
        assert!(addr.ip().is_loopback());
        assert_ne!(addr.port(), 0);
    }

    #[test]
    fn a_changed_setting_restarts_the_site_once_and_a_second_plan_is_empty() {
        let dir = site_folder();
        let other = site_folder();
        let mut host = Host::new(target());
        let first = host
            .plan(&manifest(), &config(&dir, "127.0.0.1:0"))
            .unwrap();
        host.apply_approved(&first, &first.digest(), &MapSecrets::new())
            .unwrap();
        let starts = |h: &Host<LocalTarget>| h.target().diagnostics().matches("started on").count();
        assert_eq!(starts(&host), 1);
        assert!(
            host.plan(&manifest(), &config(&dir, "127.0.0.1:0"))
                .unwrap()
                .is_empty()
        );
        let second = host
            .plan(&manifest(), &config(&other, "127.0.0.1:0"))
            .unwrap();
        assert_eq!(second.steps().len(), 1);
        host.apply_approved(&second, &second.digest(), &MapSecrets::new())
            .unwrap();
        assert_eq!(starts(&host), 2);
        assert_eq!(host.target().diagnostics().matches("stopped").count(), 1);
    }

    #[test]
    fn a_setting_it_does_not_take_is_refused_not_recorded() {
        let dir = site_folder();
        let mut host = Host::new(target());
        let manifest = AppManifest::new(ID).setting("BASE_URL");
        let config = Config::from([("BASE_URL".to_string(), "https://x.invalid".to_string())]);
        let plan = host.plan(&manifest, &config).unwrap();
        let err = host
            .apply_approved(&plan, &plan.digest(), &MapSecrets::new())
            .unwrap_err();
        let _ = dir;
        assert!(format!("{err}").contains("not a setting a local site takes"));
        assert!(
            !host
                .target()
                .observe(ID)
                .map(|o| o.settings.contains_key("BASE_URL"))
                .unwrap_or(false)
        );
    }

    #[test]
    fn a_bind_beyond_loopback_is_refused_unless_allowed() {
        let mut strict = target();
        strict.apply_step(ID, &Step::Create, None).unwrap();
        let wide = Step::SetSetting {
            name: BIND.into(),
            value: "0.0.0.0:0".into(),
        };
        let err = strict.apply_step(ID, &wide, None).unwrap_err();
        assert!(format!("{err}").contains("loopback"));
        let mut open = target().allow_non_loopback();
        open.apply_step(ID, &Step::Create, None).unwrap();
        open.apply_step(ID, &wide, None).unwrap();
        // `ACME_CONTACT` held first is the other opt-in; set after the wide bind it does not rescue it.
        let contact = Step::SetSetting {
            name: ACME_CONTACT.into(),
            value: "ops@example.org".into(),
        };
        let mut acme = target();
        acme.apply_step(ID, &Step::Create, None).unwrap();
        assert!(acme.apply_step(ID, &wide, None).is_err());
        acme.apply_step(ID, &contact, None).unwrap();
        acme.apply_step(ID, &wide, None).unwrap();
        let http_wide = Step::SetSetting {
            name: HTTP_BIND.into(),
            value: "0.0.0.0:80".into(),
        };
        acme.apply_step(ID, &http_wide, None).unwrap();
        let bad = Step::SetSetting {
            name: BIND.into(),
            value: "not-an-address".into(),
        };
        assert!(open.apply_step(ID, &bad, None).is_err());
    }

    #[test]
    fn proxy_and_files_values_are_checked() {
        let mut t = target();
        t.apply_step(ID, &Step::Create, None).unwrap();
        let set = |name: &str, value: &str| Step::SetSetting {
            name: name.into(),
            value: value.into(),
        };
        assert!(t.apply_step(ID, &set(FILES, "  "), None).is_err());
        assert!(t.apply_step(ID, &set(PROXY, "api=http://x"), None).is_err());
        assert!(t.apply_step(ID, &set(PROXY, "/api"), None).is_err());
        assert!(
            t.apply_step(ID, &set(PROXY, "/api=http://127.0.0.1:3000"), None)
                .is_ok()
        );
    }

    #[test]
    fn secrets_and_domains_are_declared_no_and_reading_the_certificate_needs_trust() {
        let t = target();
        for op in [Op::SetSecret, Op::DeleteSecret, Op::BindDomain] {
            assert_eq!(t.supports(op), Support::No, "{op:?}");
        }
        // Served by `tls_status`, but like every operation it is `Unverified` until the target is told to trust it.
        assert_eq!(t.supports(Op::ReadTls), Support::Unverified);
        assert_eq!(
            LocalTarget::new()
                .unwrap()
                .trust_operations(&[Op::ReadTls])
                .supports(Op::ReadTls),
            Support::Yes
        );
        assert_eq!(t.supports(Op::Create), Support::Yes);
        assert_eq!(t.supports(Op::Restore), Support::Yes);
        assert_eq!(
            LocalTarget::new().unwrap().supports(Op::Restore),
            Support::Unverified
        );
        // Not trusted yet: a plan refuses it.
        assert_eq!(
            LocalTarget::new().unwrap().supports(Op::SetSetting),
            Support::Unverified
        );
    }

    #[test]
    fn a_site_with_nothing_to_serve_does_not_start() {
        let mut host = Host::new(target());
        let manifest = AppManifest::new(ID).setting(BIND);
        let config = Config::from([(BIND.to_string(), "127.0.0.1:0".to_string())]);
        let plan = host.plan(&manifest, &config).unwrap();
        let err = host
            .apply_approved(&plan, &plan.digest(), &MapSecrets::new())
            .unwrap_err();
        assert!(matches!(err, Error::ApplyFailed { .. }), "{err:?}");
        assert!(host.target().address(ID).is_none());
    }

    #[test]
    fn a_health_path_that_is_not_there_reads_back_unhealthy() {
        let dir = site_folder();
        let mut t = LocalTarget::new()
            .unwrap()
            .trust_operations(&[Op::Create, Op::SetSetting])
            .with_health_path("/no-such-page");
        let set = |name: &str, value: String| Step::SetSetting {
            name: name.into(),
            value,
        };
        t.apply_step(ID, &Step::Create, None).unwrap();
        t.apply_step(ID, &set(FILES, dir.path().display().to_string()), None)
            .unwrap();
        t.finish_apply(ID).unwrap();
        assert!(!t.observe(ID).unwrap().healthy);
        // The same site answers its own index page.
        let mut ok = target();
        ok.apply_step(ID, &Step::Create, None).unwrap();
        ok.apply_step(ID, &set(FILES, dir.path().display().to_string()), None)
            .unwrap();
        ok.finish_apply(ID).unwrap();
        assert!(ok.observe(ID).unwrap().healthy);
        // Before it is started nothing answers.
        let mut idle = target();
        idle.apply_step(ID, &Step::Create, None).unwrap();
        assert!(!idle.observe(ID).unwrap().healthy);
    }

    #[test]
    fn creating_over_a_site_that_is_not_ours_is_refused_and_changes_nothing() {
        let mut t = target().holding_foreign(ID);
        let before = t.diagnostics();
        let err = t.apply_step(ID, &Step::Create, None).unwrap_err();
        assert!(matches!(err, Error::NotOurs { .. }), "{err:?}");
        assert_eq!(t.diagnostics(), before);
        assert_eq!(t.locate(ID), Ok(Presence::NotOurs));
        // Nor can a setting be written into it.
        let set = Step::SetSetting {
            name: FILES.into(),
            value: "x".into(),
        };
        assert!(matches!(
            t.apply_step(ID, &set, None),
            Err(Error::NotOurs { .. })
        ));
    }

    /// A folder holding one page, called `name`.
    fn folder_with(name: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(name), "<p>ok</p>").unwrap();
        dir
    }

    #[test]
    fn a_rollback_serves_the_old_folder_again() {
        let (a, b) = (folder_with("a.html"), folder_with("b.html"));
        // The health path is a page only folder `a` has, so "healthy" says which folder is being served.
        let mut host = Host::new(target().with_health_path("/a.html"));
        let first = host.plan(&manifest(), &config(&a, "127.0.0.1:0")).unwrap();
        host.apply_approved(&first, &first.digest(), &MapSecrets::new())
            .unwrap();
        let second = host.plan(&manifest(), &config(&b, "127.0.0.1:0")).unwrap();
        // Folder b has no a.html, so the read-back says unhealthy and the apply fails, with a point to go back to.
        let Err(Error::ApplyFailed {
            cause,
            rollback_point: Some(point),
            ..
        }) = host.apply_approved(&second, &second.digest(), &MapSecrets::new())
        else {
            panic!("expected a failed apply with a rollback point");
        };
        assert!(
            matches!(*cause, Error::ReadBackMismatch { .. }),
            "{cause:?}"
        );
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);
        let seen = host.target().observe(ID).unwrap();
        assert_eq!(
            seen.settings.get(FILES),
            Some(&a.path().display().to_string())
        );
        assert!(seen.healthy, "folder a is served again");
        assert_eq!(
            host.target()
                .diagnostics()
                .matches("restored the settings")
                .count(),
            1
        );
    }

    #[test]
    fn rolling_back_a_first_apply_stops_the_site_and_forgets_the_app() {
        let dir = site_folder();
        let mut host = Host::new(target());
        let plan = host
            .plan(&manifest(), &config(&dir, "127.0.0.1:0"))
            .unwrap();
        let done = host
            .apply_approved(&plan, &plan.digest(), &MapSecrets::new())
            .unwrap();
        let addr = host.target().address(ID).unwrap();
        let point = done.rollback_point.unwrap();
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);
        assert_eq!(host.target().locate(ID), Ok(Presence::Absent));
        assert!(host.target().address(ID).is_none());
        assert!(
            host.target()
                .diagnostics()
                .contains("stopped the running site")
        );
        assert!(std::net::TcpStream::connect(addr).is_err());
    }

    #[test]
    fn restoring_settings_with_nothing_to_serve_stops_the_site() {
        let dir = site_folder();
        let mut t = target();
        let set = |name: &str, value: String| Step::SetSetting {
            name: name.into(),
            value,
        };
        t.apply_step(ID, &Step::Create, None).unwrap();
        let bare = t.snapshot(ID).unwrap();
        t.apply_step(ID, &set(FILES, dir.path().display().to_string()), None)
            .unwrap();
        t.finish_apply(ID).unwrap();
        let addr = t.address(ID).unwrap();
        t.restore(ID, &bare).unwrap();
        assert_eq!(t.locate(ID), Ok(Presence::Ours));
        assert!(t.address(ID).is_none());
        assert!(t.observe(ID).unwrap().settings.is_empty());
        assert!(std::net::TcpStream::connect(addr).is_err());
    }

    #[test]
    fn restoring_a_proxy_only_app_starts_it() {
        let mut t = target();
        let snapshot = Snapshot {
            existed: true,
            observed: Observed {
                settings: BTreeMap::from([(
                    PROXY.to_string(),
                    "/api=http://127.0.0.1:9".to_string(),
                )]),
                ..Observed::default()
            },
            handle: None,
        };
        t.apply_step(ID, &Step::Create, None).unwrap();
        t.restore(ID, &snapshot).unwrap();
        assert!(t.address(ID).is_some());
    }

    #[test]
    fn restoring_over_a_site_that_is_not_ours_is_refused() {
        let mut t = target().holding_foreign(ID);
        let before = t.diagnostics();
        let snapshot = Snapshot {
            existed: false,
            observed: Observed::default(),
            handle: None,
        };
        assert!(matches!(
            t.restore(ID, &snapshot),
            Err(Error::NotOurs { .. })
        ));
        assert_eq!(t.diagnostics(), before);
        assert_eq!(t.locate(ID), Ok(Presence::NotOurs));
    }

    #[test]
    fn dropping_the_target_frees_the_port() {
        let dir = site_folder();
        let mut host = Host::new(target());
        let plan = host
            .plan(&manifest(), &config(&dir, "127.0.0.1:0"))
            .unwrap();
        host.apply_approved(&plan, &plan.digest(), &MapSecrets::new())
            .unwrap();
        let addr = host.target().address(ID).unwrap();
        drop(host);
        // The listener is gone: nothing accepts there any more.
        assert!(std::net::TcpStream::connect(addr).is_err());
    }

    // ---- C8d: automatic certificates, against a fake certificate authority (no network) ----

    /// A certificate authority that issues a self-signed certificate named "Fake CA" for the asked names, and refuses from its `fail_from`-th ask on.
    struct FakeCa {
        asks: std::sync::atomic::AtomicUsize,
        fail_from: usize,
    }

    impl FakeCa {
        fn new(fail_from: usize) -> Self {
            Self {
                asks: std::sync::atomic::AtomicUsize::new(0),
                fail_from,
            }
        }
    }

    impl AcmeClient for FakeCa {
        fn obtain<'a>(
            &'a self,
            names: &'a [String],
            _challenges: &'a webservant::Challenges,
            _kind: webservant::ChallengeKind,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = Result<webservant::Issued, webservant::AcmeError>>
                    + Send
                    + 'a,
            >,
        > {
            let ask = self.asks.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let fail = ask >= self.fail_from;
            Box::pin(async move {
                if fail {
                    return Err(webservant::AcmeError(
                        "the fake CA could not reach port 80".to_string(),
                    ));
                }
                let mut params = rcgen::CertificateParams::new(names.to_vec()).unwrap();
                params
                    .distinguished_name
                    .push(rcgen::DnType::CommonName, "Fake CA");
                let key = rcgen::KeyPair::generate().unwrap();
                let cert = params.self_signed(&key).unwrap();
                Ok(webservant::Issued {
                    chain_pem: cert.pem(),
                    key_pem: key.serialize_pem(),
                })
            })
        }
    }

    /// An app named `app.test` holding settings for a site with automatic certificates from `ca`, not yet started.
    fn acme_target(
        ca: FakeCa,
        files: &tempfile::TempDir,
        store: &tempfile::TempDir,
    ) -> LocalTarget {
        let mut t = LocalTarget::new()
            .unwrap()
            .trust_operations(&[Op::Create, Op::SetSetting, Op::ReadTls])
            .with_acme_client(ca);
        t.apply_step("app.test", &Step::Create, None).unwrap();
        for (name, value) in [
            (FILES, files.path().display().to_string()),
            (ACME_CONTACT, "ops@example.org".to_string()),
            (ACME_STORE, store.path().display().to_string()),
        ] {
            t.apply_step(
                "app.test",
                &Step::SetSetting {
                    name: name.to_string(),
                    value,
                },
                None,
            )
            .unwrap();
        }
        t
    }

    #[test]
    fn a_site_with_automatic_certificates_reports_the_issued_certificate_read_back_from_its_listener()
     {
        let (files, store) = (site_folder(), tempfile::tempdir().unwrap());
        let mut t = acme_target(FakeCa::new(usize::MAX), &files, &store);
        t.finish_apply("app.test").unwrap();
        assert_eq!(t.supports(Op::ReadTls), Support::Yes);
        let status = t.tls_status("app.test").unwrap();
        assert_eq!(status.issuer.as_deref(), Some("CN=Fake CA"));
        assert_eq!(status.names, vec!["app.test".to_string()]);
        assert_eq!(status.last_renewal, crate::Renewal::Succeeded);
        assert!(status.not_after.is_some());
    }

    #[test]
    fn a_failed_renewal_is_reported_and_the_served_certificate_stays() {
        let (files, store) = (site_folder(), tempfile::tempdir().unwrap());
        // The first ask (at start) succeeds, every later one is refused.
        let mut t = acme_target(FakeCa::new(1), &files, &store);
        t.finish_apply("app.test").unwrap();
        let certs = t.apps["app.test"]
            .running
            .as_ref()
            .unwrap()
            .certificates()
            .unwrap();
        assert!(t.runtime.block_on(certs.renew_now()).is_err());
        let status = t.tls_status("app.test").unwrap();
        assert!(
            matches!(&status.last_renewal, crate::Renewal::Failed(why) if why.contains("port 80")),
            "got {:?}",
            status.last_renewal
        );
        assert_eq!(status.issuer.as_deref(), Some("CN=Fake CA"));
    }

    #[test]
    fn the_reachability_check_answers_from_one_issuance_and_leaves_the_store_empty() {
        let (files, store) = (site_folder(), tempfile::tempdir().unwrap());
        let t = acme_target(FakeCa::new(usize::MAX), &files, &store);
        let ok = t.check_reachable("app.test").unwrap();
        assert!(ok.is_reachable());
        assert_eq!(
            std::fs::read_dir(store.path()).unwrap().count(),
            0,
            "the check must use a throwaway store, not the configured one"
        );
        let (files, store) = (site_folder(), tempfile::tempdir().unwrap());
        let t = acme_target(FakeCa::new(0), &files, &store);
        let bad = t.check_reachable("app.test").unwrap();
        assert!(!bad.is_reachable());
        assert!(bad.failure.unwrap().contains("port 80"));
    }

    #[test]
    fn the_reachability_check_needs_a_contact_and_the_acme_settings_are_validated() {
        let files = site_folder();
        let mut t = target();
        t.apply_step(ID, &Step::Create, None).unwrap();
        let err = t.check_reachable(ID).unwrap_err().to_string();
        assert!(err.contains(ACME_CONTACT), "{err}");
        for (name, value) in [
            (ACME_CONTACT, "not an email"),
            (ACME_STAGING, "maybe"),
            (ACME_DIRECTORY, "ftp://ca"),
            (ACME_NAMES, "a.test,,b.test"),
            (ACME_STORE, " "),
            (HTTP_BIND, "0.0.0.0:80"),
        ] {
            let step = Step::SetSetting {
                name: name.to_string(),
                value: value.to_string(),
            };
            assert!(
                t.apply_step(ID, &step, None).is_err(),
                "{name}={value} should be refused"
            );
        }
        drop(files);
    }
}
