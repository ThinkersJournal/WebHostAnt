//! The Cloudflare adapter: a [`Target`] that drives a pinned `wrangler`.
//!
//! **Unverified against a real account.** The commands and flags below were read from `wrangler 4.141.0 --help`; what `wrangler` prints
//! (marked UNVERIFIED in the parsing code) is assumed, because no test account exists yet (DESIGN.md D6). So every operation reports
//! [`Support::Unverified`] and a real plan is refused until the caller opts in with [`CloudflareTarget::trust_operations`].
//! The conformance suite runs against it through a fake [`Runner`] in this module's tests.
//!
//! Needs Node, `wrangler` and (to deploy a container) Docker on the machine that runs it; see `docs/CLOUDFLARE-FACTS.md`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{Error, Observed, Op, Presence, SecretValue, Step, Support, Target};

/// The plain setting that marks a Worker as deployed by WebHostAnt for this app id. A Worker without it is never ours.
pub const MARKER: &str = "WEBHOSTANT_APP";

/// One `wrangler` invocation. `stdin` carries a secret value when there is one; `Debug` never shows it.
#[derive(Clone, PartialEq, Eq)]
pub struct Call {
    /// Arguments after `wrangler`.
    pub args: Vec<String>,
    /// Text for the process's standard input.
    pub stdin: Option<SecretValue>,
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "wrangler {}", self.args.join(" "))?;
        if self.stdin.is_some() {
            write!(f, " <stdin redacted>")?;
        }
        Ok(())
    }
}

/// What `wrangler` returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Whether it exited with status 0.
    pub success: bool,
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// Something that can run `wrangler`. Tests supply a fake; [`ProcessRunner`] runs the real one.
pub trait Runner {
    /// Run one call. `Err` means it could not be run at all (not installed, not startable).
    fn run(&self, call: &Call) -> Result<Output, Error>;
}

/// How long one `wrangler` call may run before it is stopped. Generous because a container deploy builds an image.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);

/// Runs `wrangler` from the `PATH`, stopping any call that outlives its time limit.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    timeout: Duration,
}

impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }
}

impl ProcessRunner {
    /// A runner that stops each call after `timeout`.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self { timeout }
    }
}

impl Runner for ProcessRunner {
    fn run(&self, call: &Call) -> Result<Output, Error> {
        // On Windows `wrangler` is a `.cmd` shim, which `CreateProcess` will not find by bare name.
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "wrangler"]);
            c
        } else {
            Command::new("wrangler")
        };
        cmd.args(&call.args);
        run_bounded(
            cmd,
            call.stdin.as_ref(),
            self.timeout,
            &format!("wrangler {}", call.args.join(" ")),
        )
    }
}

/// Run `cmd`, feeding it `stdin`, and stop it (and its children) if it has not exited within `timeout`.
fn run_bounded(
    mut cmd: Command,
    stdin: Option<&SecretValue>,
    timeout: Duration,
    label: &str,
) -> Result<Output, Error> {
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        Error::Target(format!(
            "could not start wrangler ({e}); is Node and wrangler installed?"
        ))
    })?;
    if let (Some(secret), Some(mut pipe)) = (stdin, child.stdin.take()) {
        use std::io::Write;
        pipe.write_all(secret.expose().as_bytes())
            .map_err(|e| Error::Target(format!("could not pass input to wrangler: {e}")))?;
    }
    let out = drain(child.stdout.take());
    let err = drain(child.stderr.take());
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() >= deadline => break None,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(Error::Target(format!("wrangler did not finish: {e}"))),
        }
    };
    let Some(status) = status else {
        kill_tree(&mut child);
        let _ = child.wait();
        return Err(Error::TimedOut {
            what: label.to_string(),
            after: timeout,
        });
    };
    Ok(Output {
        success: status.success(),
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// Read a pipe to the end on its own thread, so a full pipe never stalls the child.
fn drain<R: Read + Send + 'static>(pipe: Option<R>) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if let Some(mut p) = pipe {
            let _ = p.read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// Stop the child and what it started; on Windows `cmd` leaves `node` running if only `cmd` is killed.
fn kill_tree(child: &mut std::process::Child) {
    if cfg!(windows) {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

/// A Cloudflare Worker, driven through a [`Runner`].
pub struct CloudflareTarget<R: Runner> {
    runner: R,
    project_dir: PathBuf,
    probe: Box<dyn Fn(&str) -> bool>,
    trusted: BTreeSet<Op>,
    log: RefCell<Vec<String>>,
}

impl<R: Runner> CloudflareTarget<R> {
    /// `project_dir` holds the Worker's `wrangler` config. `probe` answers whether the app's health path answers for an app id; it is a
    /// parameter because HTTP calls belong to HTTPAnt, which is not built yet.
    pub fn new(
        runner: R,
        project_dir: impl Into<PathBuf>,
        probe: impl Fn(&str) -> bool + 'static,
    ) -> Self {
        Self {
            runner,
            project_dir: project_dir.into(),
            probe: Box::new(probe),
            trusted: BTreeSet::new(),
            log: RefCell::new(Vec::new()),
        }
    }

    /// Report these operations as `Yes` instead of `Unverified`. Use only after a real run has passed the conformance suite.
    pub fn trust_operations(mut self, ops: &[Op]) -> Self {
        self.trusted.extend(ops.iter().copied());
        self
    }

    fn call(&self, args: &[&str]) -> Call {
        Call {
            args: args.iter().map(|a| a.to_string()).collect(),
            stdin: None,
        }
    }

    /// The Worker's plain bindings, or `None` when the Worker does not exist.
    fn bindings(&self, app_id: &str) -> Result<Option<BTreeMap<String, String>>, Error> {
        let list =
            self.runner
                .run(&self.call(&["deployments", "list", "--name", app_id, "--json"]))?;
        if !list.success {
            return if looks_absent(&list.stderr) {
                Ok(None)
            } else {
                // Unreachable, unauthenticated, or anything else is never reported as "not installed".
                Err(Error::Target(format!(
                    "could not list deployments: {}",
                    first_line(&list.stderr)
                )))
            };
        }
        // UNVERIFIED shape: a JSON array of deployments, newest last, each with `versions[0].version_id`.
        let version = serde_json::from_str::<Value>(&list.stdout)
            .ok()
            .and_then(|v| {
                v.as_array()?
                    .last()?
                    .get("versions")?
                    .get(0)?
                    .get("version_id")?
                    .as_str()
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                Error::Target("could not read a version id from the deployment list".to_string())
            })?;
        let view = self
            .runner
            .run(&self.call(&["versions", "view", &version, "--name", app_id, "--json"]))?;
        if !view.success {
            return Err(Error::Target(format!(
                "could not read the deployed version: {}",
                first_line(&view.stderr)
            )));
        }
        // UNVERIFIED shape: `resources.bindings` is an array of {type, name, text}; plain settings have type "plain_text".
        let bindings = serde_json::from_str::<Value>(&view.stdout)
            .ok()
            .and_then(|v| {
                let mut map = BTreeMap::new();
                for b in v.get("resources")?.get("bindings")?.as_array()? {
                    if b.get("type")?.as_str()? == "plain_text" {
                        map.insert(
                            b.get("name")?.as_str()?.to_string(),
                            b.get("text")?.as_str()?.to_string(),
                        );
                    }
                }
                Some(map)
            })
            .ok_or_else(|| {
                Error::Target("could not read settings from the deployed version".to_string())
            })?;
        Ok(Some(bindings))
    }

    fn secret_names(&self, app_id: &str) -> Result<BTreeSet<String>, Error> {
        let out = self
            .runner
            .run(&self.call(&["secret", "list", "--name", app_id, "--format", "json"]))?;
        if !out.success {
            return Err(Error::Target(format!(
                "could not list secrets: {}",
                first_line(&out.stderr)
            )));
        }
        // UNVERIFIED shape: a JSON array of {name, type}.
        serde_json::from_str::<Value>(&out.stdout)
            .ok()
            .and_then(|v| {
                v.as_array()?
                    .iter()
                    .map(|s| Some(s.get("name")?.as_str()?.to_string()))
                    .collect::<Option<BTreeSet<_>>>()
            })
            .ok_or_else(|| Error::Target("could not read secret names".to_string()))
    }

    fn deploy(&self, app_id: &str, extra_vars: &[(&str, &str)]) -> Result<Output, Error> {
        let dir = self.project_dir.to_string_lossy().into_owned();
        let marker = format!("{MARKER}:{app_id}");
        let mut args = vec![
            "deploy".to_string(),
            "--cwd".to_string(),
            dir,
            "--name".to_string(),
            app_id.to_string(),
            "--keep-vars".to_string(),
            "--var".to_string(),
            marker,
        ];
        for (k, v) in extra_vars {
            args.push("--var".to_string());
            args.push(format!("{k}:{v}"));
        }
        self.runner.run(&Call { args, stdin: None })
    }
}

fn looks_absent(stderr: &str) -> bool {
    // UNVERIFIED wording: Cloudflare's "this Worker does not exist" is error code 10007.
    let s = stderr.to_lowercase();
    s.contains("10007") || s.contains("not found") || s.contains("does not exist")
}

fn first_line(text: &str) -> &str {
    text.lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("no message")
}

impl<R: Runner> Target for CloudflareTarget<R> {
    fn name(&self) -> &str {
        "cloudflare"
    }

    fn supports(&self, op: Op) -> Support {
        // No previous-version handle is read yet, so this adapter cannot put an app back (docs/ROLLBACK-DESIGN.md).
        // Domain binding is not built for this adapter yet (docs/DOMAINS-DESIGN.md); never claim it.
        if op == Op::Restore || op == Op::BindDomain {
            Support::No
        } else if self.trusted.contains(&op) {
            Support::Yes
        } else {
            Support::Unverified
        }
    }

    fn locate(&self, app_id: &str) -> Result<Presence, Error> {
        Ok(match self.bindings(app_id)? {
            None => Presence::Absent,
            Some(b) if b.get(MARKER).map(String::as_str) == Some(app_id) => Presence::Ours,
            Some(_) => Presence::NotOurs,
        })
    }

    fn observe(&self, app_id: &str) -> Result<Observed, Error> {
        let mut settings = self
            .bindings(app_id)?
            .ok_or_else(|| Error::Target("the app is not deployed".to_string()))?;
        settings.remove(MARKER);
        Ok(Observed {
            settings,
            secret_names: self.secret_names(app_id)?,
            domains: Default::default(),
            healthy: (self.probe)(app_id),
        })
    }

    fn apply_step(
        &mut self,
        app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
        let (call_text, out) = match step {
            Step::BindDomain { .. } => {
                return Err(Error::Unsupported {
                    target: "cloudflare".to_string(),
                    op: Op::BindDomain,
                });
            }
            Step::Create => ("deploy".to_string(), self.deploy(app_id, &[])?),
            Step::SetSetting { name, value } => (
                format!("deploy (set {name})"),
                self.deploy(app_id, &[(name, value)])?,
            ),
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target(format!("no value supplied for secret {name}")))?;
                let call = Call {
                    args: ["secret", "put", name, "--name", app_id]
                        .iter()
                        .map(|a| a.to_string())
                        .collect(),
                    stdin: Some(value.clone()),
                };
                (format!("secret put {name}"), self.runner.run(&call)?)
            }
        };
        let redact = |text: &str| match secret {
            Some(s) if !s.expose().is_empty() => text.replace(s.expose(), "[redacted]"),
            _ => text.to_string(),
        };
        if out.success {
            self.log.borrow_mut().push(call_text);
            Ok(())
        } else {
            let msg = redact(first_line(&out.stderr));
            self.log
                .borrow_mut()
                .push(format!("{call_text} failed: {msg}"));
            Err(Error::Target(format!("{call_text} failed: {msg}")))
        }
    }

    fn diagnostics(&self) -> String {
        self.log.borrow().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{self, Scenario};

    #[derive(Default)]
    struct State {
        exists: bool,
        marked: bool,
        vars: BTreeMap<String, String>,
        secrets: BTreeSet<String>,
        calls: Vec<String>,
        fail_stderr: Option<String>,
        fail_put_once: bool,
    }

    /// A pretend `wrangler` that answers in the shapes the adapter assumes.
    struct FakeWrangler(RefCell<State>);

    impl FakeWrangler {
        fn new() -> Self {
            Self(RefCell::new(State::default()))
        }
        fn other_app() -> Self {
            let f = Self::new();
            f.0.borrow_mut().exists = true;
            f
        }
        fn failing_secret_put_once() -> Self {
            let f = Self::new();
            f.0.borrow_mut().fail_put_once = true;
            f
        }
        fn failing(stderr: &str) -> Self {
            let f = Self::new();
            f.0.borrow_mut().fail_stderr = Some(stderr.to_string());
            f
        }
    }

    fn ok(stdout: impl Into<String>) -> Result<Output, Error> {
        Ok(Output {
            success: true,
            stdout: stdout.into(),
            stderr: String::new(),
        })
    }

    fn err(stderr: &str) -> Result<Output, Error> {
        Ok(Output {
            success: false,
            stdout: String::new(),
            stderr: stderr.to_string(),
        })
    }

    impl Runner for FakeWrangler {
        fn run(&self, call: &Call) -> Result<Output, Error> {
            let mut s = self.0.borrow_mut();
            if let Some(e) = &s.fail_stderr {
                return err(e);
            }
            let a: Vec<&str> = call.args.iter().map(String::as_str).collect();
            match a.as_slice() {
                ["deployments", "list", ..] => {
                    if s.exists {
                        ok(r#"[{"versions":[{"version_id":"v1"}]}]"#)
                    } else {
                        err(
                            "A request to the Cloudflare API failed. Worker not found [code: 10007]",
                        )
                    }
                }
                ["versions", "view", ..] => {
                    let mut bindings: Vec<Value> = s
                        .vars
                        .iter()
                        .map(|(k, v)| serde_json::json!({"type":"plain_text","name":k,"text":v}))
                        .collect();
                    if s.marked {
                        bindings.push(serde_json::json!({"type":"plain_text","name":MARKER,"text":"conformance-app"}));
                    }
                    ok(serde_json::json!({"resources":{"bindings":bindings}}).to_string())
                }
                ["secret", "list", ..] => {
                    let list: Vec<Value> = s
                        .secrets
                        .iter()
                        .map(|n| serde_json::json!({"name":n,"type":"secret_text"}))
                        .collect();
                    ok(Value::Array(list).to_string())
                }
                ["secret", "put", ..] if s.fail_put_once => {
                    s.fail_put_once = false;
                    err("A request to the Cloudflare API failed [code: 10000]")
                }
                ["secret", "put", name, ..] => {
                    s.calls.push(format!("secret put {name}"));
                    s.secrets.insert(name.to_string());
                    ok("")
                }
                ["deploy", rest @ ..] => {
                    s.exists = true;
                    s.marked = true;
                    let mut i = 0;
                    while i < rest.len() {
                        if rest[i] == "--var" {
                            if let Some((k, v)) = rest[i + 1].split_once(':')
                                && k != MARKER
                            {
                                s.vars.insert(k.to_string(), v.to_string());
                            }
                            i += 1;
                        }
                        i += 1;
                    }
                    s.calls.push("deploy".to_string());
                    ok("")
                }
                other => err(&format!("unexpected call {other:?}")),
            }
        }
    }

    fn target(w: FakeWrangler) -> CloudflareTarget<FakeWrangler> {
        // The probe says healthy once the Worker exists.
        CloudflareTarget::new(w, "worker", |_| true).trust_operations(&[
            Op::Create,
            Op::SetSetting,
            Op::SetSecret,
        ])
    }

    #[test]
    fn the_conformance_suite_passes_against_the_fake_wrangler() {
        let failures = conformance::run(|scenario| match scenario {
            Scenario::Empty => target(FakeWrangler::new()),
            Scenario::HoldsOtherApp => target(FakeWrangler::other_app()),
            Scenario::Unreachable => target(FakeWrangler::failing(
                "fetch failed: ENOTFOUND api.cloudflare.com",
            )),
            Scenario::FailsLastStepOnce => target(FakeWrangler::failing_secret_put_once()),
            // This adapter cannot restore, so the drop-after-apply check is skipped for it; the scenario still needs a target.
            Scenario::DropsAfterApply => target(FakeWrangler::new()),
            // Secrets left untrusted: `Unverified` is not `Yes`, so a plan that needs one must be refused.
            Scenario::SecretsUnsupported => {
                CloudflareTarget::new(FakeWrangler::new(), "worker", |_| true)
                    .trust_operations(&[Op::Create, Op::SetSetting])
            }
        });
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn an_untrusted_adapter_refuses_to_plan() {
        let host = crate::Host::new(CloudflareTarget::new(FakeWrangler::new(), "worker", |_| {
            true
        }));
        let manifest = crate::AppManifest::new("conformance-app").setting("A");
        let config = crate::Config::from([("A".to_string(), "1".to_string())]);
        assert!(matches!(
            host.plan(&manifest, &config),
            Err(Error::Unsupported { op: Op::Create, .. })
        ));
    }

    #[test]
    fn unreachable_is_never_absent() {
        let t = target(FakeWrangler::failing(
            "fetch failed: ENOTFOUND api.cloudflare.com",
        ));
        assert!(matches!(t.locate("x"), Err(Error::Target(_))));
        let t = target(FakeWrangler::failing(
            "You are not authenticated [code: 10000]",
        ));
        assert!(matches!(t.locate("x"), Err(Error::Target(_))));
    }

    #[test]
    fn a_failed_secret_put_does_not_echo_the_value() {
        let w = FakeWrangler::new();
        w.0.borrow_mut().fail_stderr = Some("bad value CANARY-xyz rejected".to_string());
        let mut t = target(w);
        let e = t
            .apply_step(
                "x",
                &Step::SetSecret {
                    name: "K".to_string(),
                },
                Some(&SecretValue::new("CANARY-xyz")),
            )
            .unwrap_err();
        assert!(!e.to_string().contains("CANARY-xyz"), "{e}");
        assert!(!t.diagnostics().contains("CANARY-xyz"));
    }

    #[test]
    fn a_call_never_shows_its_stdin() {
        let c = Call {
            args: vec!["secret".into(), "put".into(), "K".into()],
            stdin: Some(SecretValue::new("CANARY-xyz")),
        };
        assert!(!format!("{c:?}").contains("CANARY-xyz"));
    }

    #[test]
    fn a_call_that_outlives_its_limit_is_stopped_and_reported() {
        let cmd = if cfg!(windows) {
            let mut c = Command::new("ping");
            c.args(["-n", "30", "127.0.0.1"]);
            c
        } else {
            let mut c = Command::new("sleep");
            c.arg("30");
            c
        };
        let started = Instant::now();
        let e = run_bounded(cmd, None, Duration::from_millis(300), "slow thing").unwrap_err();
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "was not stopped"
        );
        assert_eq!(
            e,
            Error::TimedOut {
                what: "slow thing".into(),
                after: Duration::from_millis(300)
            }
        );
    }

    #[test]
    fn a_call_that_finishes_in_time_returns_its_output() {
        let cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "echo hi"]);
            c
        } else {
            let mut c = Command::new("echo");
            c.arg("hi");
            c
        };
        let out = run_bounded(cmd, None, Duration::from_secs(30), "echo").unwrap();
        assert!(out.success);
        assert!(out.stdout.contains("hi"));
    }
}
