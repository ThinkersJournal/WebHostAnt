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
//! * `BIND`: where to listen, as `ip:port`. It must be a loopback address unless [`LocalTarget::allow_non_loopback`] was called. Without it the
//!   site listens on `127.0.0.1` at a free port.
//! * `PROXY`: one forwarded path, as `/prefix=http://upstream`.
//!
//! Secrets, domains, certificates and `Restore` are declared `No` here: automatic certificates are row C8d and restore is C8c.
//!
//! The target owns a small thread runtime that keeps the site alive between calls, and dropping the target stops every site. Do not call it from
//! inside an async runtime (`block_on` would panic): [`crate::Host`] is synchronous.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use webservant::{RunningSite, Settings, Site};

use crate::{
    Error, Health, HealthProbe, Observed, Op, Presence, SecretValue, Step, Support, Target,
};

const FILES: &str = "FILES";
const BIND: &str = "BIND";
const PROXY: &str = "PROXY";

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

    fn check(&self, name: &str, value: &str) -> Result<(), Error> {
        match name {
            FILES if value.trim().is_empty() => Err(refuse(FILES, "is empty")),
            FILES => Ok(()),
            BIND => {
                let addr: SocketAddr = value
                    .parse()
                    .map_err(|_| refuse(BIND, "is not `ip:port`"))?;
                if !addr.ip().is_loopback() && !self.non_loopback {
                    return Err(refuse(
                        BIND,
                        "is not a loopback address (call `allow_non_loopback` to listen beyond this machine)",
                    ));
                }
                Ok(())
            }
            PROXY => match value.split_once('=') {
                Some((prefix, upstream))
                    if prefix.starts_with('/') && upstream.starts_with("http") =>
                {
                    Ok(())
                }
                _ => Err(refuse(PROXY, "is not `/prefix=http://upstream`")),
            },
            other => Err(Error::Target(format!(
                "`{other}` is not a setting a local site takes (FILES, BIND, PROXY); it was not applied"
            ))),
        }
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
            Op::Create | Op::SetSetting if self.trusted.contains(&op) => Support::Yes,
            Op::Create | Op::SetSetting => Support::Unverified,
            // No secret store or environment, no certificates yet (C8d), no restore yet (C8c).
            Op::SetSecret | Op::DeleteSecret | Op::Restore | Op::ReadTls | Op::BindDomain => {
                Support::No
            }
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
                self.check(name, value)?;
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
        let settings = self.settings_of(app_id, &app.settings);
        let site = Site::from_settings(&settings)
            .map_err(|e| Error::Target(format!("the settings do not make a site: {e}")))?;
        let old = self.apps.get_mut(app_id).and_then(|a| a.running.take());
        if let Some(old) = old {
            self.runtime
                .block_on(old.shutdown())
                .map_err(|e| Error::Target(format!("could not stop the running site: {e}")))?;
            self.log.push("stopped the running site".to_string());
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
            .trust_operations(&[Op::Create, Op::SetSetting])
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
    fn secrets_domains_tls_and_restore_are_declared_no() {
        let t = target();
        for op in [
            Op::SetSecret,
            Op::DeleteSecret,
            Op::Restore,
            Op::ReadTls,
            Op::BindDomain,
        ] {
            assert_eq!(t.supports(op), Support::No, "{op:?}");
        }
        assert_eq!(t.supports(Op::Create), Support::Yes);
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
}
