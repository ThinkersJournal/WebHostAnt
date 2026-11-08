//! The Cloudflare adapter: a [`Target`] that drives a pinned `wrangler`.
//!
//! **Checked once against a real account (2026-11-08, `wrangler` 4.141.0, a throwaway Worker; `docs/CLOUDFLARE-FACTS.md`):** `Create`, `SetSetting`,
//! `SetSecret` and `DeleteSecret` each ran for real and read back correctly, and the answer shapes below match. That is one run by hand, not a test account
//! (DESIGN.md D6), so every operation still reports [`Support::Unverified`] and a real plan is refused until the caller opts in with
//! [`CloudflareTarget::trust_operations`]. `Restore` (below) ran once for real on a throwaway Worker (docs/CLOUDFLARE-FACTS.md).
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

use crate::cloudflare_domains::{Domains, Rest, tls_findings};
use crate::{
    Error, Observed, Op, Presence, SecretValue, Snapshot, Step, Support, Target, TlsStatus,
};

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

impl Call {
    /// The call as a person may read it in a log or an error: a secret's name is withheld (which secrets a host holds is not for logs).
    fn describe(&self) -> String {
        if self.args.len() >= 3 && self.args[0] == "secret" && self.args[1] == "put" {
            let rest = self.args[3..].join(" ");
            return format!("wrangler secret put <name withheld> {rest}");
        }
        format!("wrangler {}", self.args.join(" "))
    }
}

impl std::fmt::Debug for Call {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.describe())?;
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

/// Runs `wrangler` (or another program, see [`ProcessRunner::program`]) from the `PATH`, stopping any call that outlives its time limit.
#[derive(Debug, Clone)]
pub struct ProcessRunner {
    timeout: Duration,
    program: &'static str,
}

impl Default for ProcessRunner {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            program: "wrangler",
        }
    }
}

impl ProcessRunner {
    /// A runner that stops each call after `timeout`.
    pub fn with_timeout(timeout: Duration) -> Self {
        Self {
            timeout,
            ..Self::default()
        }
    }

    /// A runner for another program found the same way (the preflight runs `docker` through one). The name is a fixed string, never input.
    pub fn program(program: &'static str, timeout: Duration) -> Self {
        Self { timeout, program }
    }
}

impl Runner for ProcessRunner {
    fn run(&self, call: &Call) -> Result<Output, Error> {
        // On Windows `wrangler` is a `.cmd` shim, which `CreateProcess` will not find by bare name.
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", self.program]);
            c
        } else {
            Command::new(self.program)
        };
        cmd.args(&call.args);
        run_bounded(cmd, call.stdin.as_ref(), self.timeout, &call.describe())
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

type Probe = Box<dyn Fn(&str) -> bool>;

/// A Cloudflare Worker, driven through a [`Runner`].
pub struct CloudflareTarget<R: Runner> {
    runner: R,
    project_dir: PathBuf,
    probe: Box<dyn Fn(&str) -> bool>,
    ready: Option<Probe>,
    domains: Option<Domains<Box<dyn Rest>>>,
    trusted: BTreeSet<Op>,
    log: RefCell<Vec<String>>,
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

fn deployments_list_args(app_id: &str) -> Vec<String> {
    strings(&["deployments", "list", "--name", app_id, "--json"])
}

fn versions_view_args(version: &str, app_id: &str) -> Vec<String> {
    strings(&["versions", "view", version, "--name", app_id, "--json"])
}

fn secret_list_args(app_id: &str) -> Vec<String> {
    strings(&["secret", "list", "--name", app_id, "--format", "json"])
}

/// Arguments of `wrangler secret put`: the secret's NAME only. The value never goes in an argument; it travels on stdin
/// (`Call::stdin`), so nothing built here can leak it.
fn secret_name_put_args(name: &str, app_id: &str) -> Vec<String> {
    strings(&["secret", "put", name, "--name", app_id])
}

/// Arguments of `wrangler rollback`: the version id (checked by [`valid_version_id`] before any call) and the app name. No secret is involved.
fn rollback_args(version: &str, app_id: &str) -> Vec<String> {
    strings(&[
        "rollback",
        version,
        "--name",
        app_id,
        "--message",
        "webhostant rollback",
        "--yes",
    ])
}

/// Arguments of `wrangler delete`, used only to undo a first deploy (see `restore`).
fn delete_worker_args(app_id: &str, project_dir: &str) -> Vec<String> {
    strings(&["delete", "--name", app_id, "--cwd", project_dir, "--force"])
}

/// A Worker version id as Cloudflare prints it: hex digits and dashes. Anything else could be read as a flag, so it is never passed on.
fn valid_version_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Arguments of `wrangler secret delete`: the NAME only. Wrangler asks "are you sure"; the answer travels on stdin.
fn secret_delete_args(name: &str, app_id: &str) -> Vec<String> {
    strings(&["secret", "delete", name, "--name", app_id])
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
            ready: None,
            domains: None,
            trusted: BTreeSet::new(),
            log: RefCell::new(Vec::new()),
        }
    }

    /// After every deploy this adapter makes (`Create` and each `SetSetting`), ask `ready` whether the app is up and fail the step if it is not. Use
    /// a waiting probe here ([`crate::HealthProbe::into_waiting_probe`]) for a container, which needs minutes. Left unset, a deploy is not waited for.
    /// Planning and read-back use the plain `probe` given to `new`, so a down app never makes a plan slow.
    pub fn with_ready_wait(mut self, ready: impl Fn(&str) -> bool + 'static) -> Self {
        self.ready = Some(Box::new(ready));
        self
    }

    /// Bind domains through the Workers REST API ([`Domains`]). Without this, `BindDomain` is `No`. Still `Unverified` until trusted.
    pub fn with_domains(mut self, domains: Domains<Box<dyn Rest>>) -> Self {
        self.domains = Some(domains);
        self
    }

    /// Report these operations as `Yes` instead of `Unverified`. Use only after a real run has passed the conformance suite.
    pub fn trust_operations(mut self, ops: &[Op]) -> Self {
        self.trusted.extend(ops.iter().copied());
        self
    }

    fn call(&self, args: Vec<String>) -> Call {
        Call { args, stdin: None }
    }

    /// The id of the version that is deployed now, or `None` when the Worker does not exist.
    fn current_version(&self, app_id: &str) -> Result<Option<String>, Error> {
        let list = self.runner.run(&self.call(deployments_list_args(app_id)))?;
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
        // Shape checked against a real account 2026-11-08: a JSON array of deployments, newest last (a setting changed after several deploys read back
        // correctly), each with `versions[0].version_id`.
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
        Ok(Some(version))
    }

    /// The Worker's plain bindings, or `None` when the Worker does not exist.
    fn bindings(&self, app_id: &str) -> Result<Option<BTreeMap<String, String>>, Error> {
        let Some(version) = self.current_version(app_id)? else {
            return Ok(None);
        };
        let view = self
            .runner
            .run(&self.call(versions_view_args(&version, app_id)))?;
        if !view.success {
            return Err(Error::Target(format!(
                "could not read the deployed version: {}",
                first_line(&view.stderr)
            )));
        }
        // Shape checked against a real account 2026-11-08: `resources.bindings` is an array of {type, name, text}; plain settings have type "plain_text" (a
        // secret appears as type "secret_text").
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
        let out = self.runner.run(&self.call(secret_list_args(app_id)))?;
        if !out.success {
            return Err(Error::Target(format!(
                "could not list secrets: {}",
                first_line(&out.stderr)
            )));
        }
        // Shape checked against a real account 2026-11-08: a JSON array of {name, type} (empty array when none).
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
        let args = self.deploy_args(app_id, extra_vars);
        self.runner.run(&Call { args, stdin: None })
    }

    fn deploy_args(&self, app_id: &str, extra_vars: &[(&str, &str)]) -> Vec<String> {
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
        args
    }
}

fn looks_absent(stderr: &str) -> bool {
    // Checked against a real account 2026-11-08: a Worker that does not exist answers with error code 10007.
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
        // Domain binding needs the REST API configured (`with_domains`); without it never claim it.
        if matches!(op, Op::BindDomain | Op::ReadTls) && self.domains.is_none() {
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
            domains: match &self.domains {
                Some(d) => d
                    .list(Some(app_id), None)?
                    .into_iter()
                    .map(|x| x.hostname)
                    .collect(),
                None => Default::default(),
            },
            healthy: (self.probe)(app_id),
        })
    }

    fn snapshot(&self, app_id: &str) -> Result<Snapshot, Error> {
        match self.locate(app_id)? {
            Presence::Ours => Ok(Snapshot {
                existed: true,
                observed: self.observe(app_id)?,
                // The version deployed now: what `restore` rolls back to.
                handle: self.current_version(app_id)?,
            }),
            Presence::Absent => Ok(Snapshot {
                existed: false,
                observed: Observed::default(),
                handle: None,
            }),
            Presence::NotOurs => Err(Error::NotOurs {
                app: app_id.to_string(),
            }),
        }
    }

    /// Put the Worker back to the version that was deployed at the snapshot (`wrangler rollback`), or, when it did not exist then, delete the Worker
    /// this apply created (never one the marker does not show to be ours).
    ///
    /// What this does NOT undo: data in any store the Worker uses; a container's image or running instances (UNVERIFIED: whether a Worker rollback
    /// reverts what a Container binding runs); a secret's old VALUE (only which names exist is read back); domain bindings (a separate REST call).
    fn restore(&mut self, app_id: &str, snapshot: &Snapshot) -> Result<(), Error> {
        let (text, call) = if snapshot.existed {
            let version = snapshot.handle.as_deref().filter(|v| valid_version_id(v));
            let Some(version) = version else {
                return Err(Error::Target(
                    "the snapshot has no usable version id to roll back to".to_string(),
                ));
            };
            ("rollback", self.call(rollback_args(version, app_id)))
        } else {
            match self.locate(app_id)? {
                // Nothing was created, so there is nothing to remove.
                Presence::Absent => return Ok(()),
                Presence::Ours => {}
                Presence::NotOurs => {
                    return Err(Error::NotOurs {
                        app: app_id.to_string(),
                    });
                }
            }
            let dir = self.project_dir.to_string_lossy().into_owned();
            ("delete", self.call(delete_worker_args(app_id, &dir)))
        };
        if snapshot.existed {
            // Checked on a real account 2026-11-08: a rollback puts the version's settings back but leaves a secret made after that version in the
            // Worker's secret list, and once rolled back the latest version is no longer the deployed one, so Cloudflare refuses to change a secret
            // (code 10215). So the secrets the snapshot did not have are deleted BEFORE the rollback, while the latest version is still deployed. A
            // secret that existed then and was deleted since cannot come back (its value is gone) and shows as a difference.
            let extra: Vec<String> = self
                .secret_names(app_id)?
                .difference(&snapshot.observed.secret_names)
                .cloned()
                .collect();
            for name in extra {
                let call = Call {
                    args: secret_delete_args(&name, app_id),
                    stdin: Some(SecretValue::new("y")),
                };
                let out = self.runner.run(&call)?;
                if !out.success {
                    let msg = first_line(&out.stderr).to_string();
                    self.log
                        .borrow_mut()
                        .push(format!("secret delete failed: {msg}"));
                    return Err(Error::Target(format!(
                        "could not delete a secret made after the snapshot, so nothing was rolled back: {msg}"
                    )));
                }
                self.log.borrow_mut().push("secret delete".to_string());
            }
        }
        let out = self.runner.run(&call)?;
        if !out.success {
            let msg = first_line(&out.stderr).to_string();
            self.log.borrow_mut().push(format!("{text} failed: {msg}"));
            return Err(Error::Target(format!("{text} failed: {msg}")));
        }
        self.log.borrow_mut().push(text.to_string());
        Ok(())
    }

    fn apply_step(
        &mut self,
        app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
        let deploys = matches!(step, Step::Create | Step::SetSetting { .. });
        let (call_text, out) = match step {
            Step::BindDomain { name } => {
                let Some(domains) = &self.domains else {
                    return Err(Error::Unsupported {
                        target: "cloudflare".to_string(),
                        op: Op::BindDomain,
                    });
                };
                domains.attach(name, app_id)?;
                self.log.borrow_mut().push("bind domain".to_string());
                return Ok(());
            }
            Step::Create => ("deploy".to_string(), self.deploy(app_id, &[])?),
            Step::SetSetting { name, value } => (
                format!("deploy (set {name})"),
                self.deploy(app_id, &[(name, value)])?,
            ),
            Step::DeleteSecret { name } => {
                let call = Call {
                    args: secret_delete_args(name, app_id),
                    stdin: Some(SecretValue::new("y")),
                };
                ("secret delete".to_string(), self.runner.run(&call)?)
            }
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target("no value supplied for a secret".to_string()))?;
                let call = Call {
                    args: secret_name_put_args(name, app_id),
                    stdin: Some(value.clone()),
                };
                ("secret put".to_string(), self.runner.run(&call)?)
            }
        };
        let redact = |text: &str| match secret {
            Some(s) if !s.expose().is_empty() => text.replace(s.expose(), "[redacted]"),
            _ => text.to_string(),
        };
        if out.success {
            self.log.borrow_mut().push(call_text);
            if deploys
                && let Some(ready) = &self.ready
                && !ready(app_id)
            {
                self.log
                    .borrow_mut()
                    .push("deployed, but the app did not become healthy".to_string());
                return Err(Error::Target(
                    "deployed, but the app did not become healthy in time; read the Worker's logs, then plan again".to_string(),
                ));
            }
            Ok(())
        } else {
            let msg = redact(first_line(&out.stderr));
            self.log
                .borrow_mut()
                .push(format!("{call_text} failed: {msg}"));
            Err(Error::Target(format!("{call_text} failed: {msg}")))
        }
    }

    fn tls_status(&self, app_id: &str) -> Result<TlsStatus, Error> {
        let Some(domains) = &self.domains else {
            return Err(Error::Unsupported {
                target: "cloudflare".to_string(),
                op: Op::ReadTls,
            });
        };
        // Ask the Worker side first: an unreachable or foreign host must not be answered for from the zone side alone.
        if self.locate(app_id)? != Presence::Ours {
            return Err(Error::Target(
                "the app is not ours or not deployed".to_string(),
            ));
        }
        let names: Vec<String> = domains
            .list(Some(app_id), None)?
            .into_iter()
            .map(|d| d.hostname)
            .collect();
        Ok(tls_findings(&names, &domains.certificate_packs()?))
    }

    fn diagnostics(&self) -> String {
        self.log.borrow().join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conformance::{self, Scenario};
    use crate::{Config, Host};

    #[derive(Default)]
    struct State {
        exists: bool,
        marked: bool,
        vars: BTreeMap<String, String>,
        secrets: BTreeSet<String>,
        /// Every version ever made: its plain settings and secret names.
        history: Vec<(BTreeMap<String, String>, BTreeSet<String>)>,
        /// Deployments, oldest first, each an index into `history`.
        deployments: Vec<usize>,
        calls: Vec<String>,
        fail_stderr: Option<String>,
        fail_put_once: bool,
        /// After this many successful changes, every call fails (the network drops).
        drop_after: Option<usize>,
        changes: usize,
        /// Every call that reached the fake, successful or not (arguments only).
        seen: Vec<Vec<String>>,
    }

    impl State {
        /// A secret can only be changed while the newest version is the deployed one (code 10215 on a real account, 2026-11-08).
        fn latest_is_deployed(&self) -> bool {
            match (self.deployments.last(), self.history.len().checked_sub(1)) {
                (Some(d), Some(last)) => *d == last,
                _ => true,
            }
        }

        /// A change makes a new version and deploys it, as a real Worker does (a secret put does too: checked 2026-11-08).
        fn new_version(&mut self) {
            self.history.push((self.vars.clone(), self.secrets.clone()));
            self.deployments.push(self.history.len() - 1);
        }
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
        fn dropping_after(n: usize) -> Self {
            let f = Self::new();
            f.0.borrow_mut().drop_after = Some(n);
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
            let out = self.answer(call);
            let first = call.args.first().map(String::as_str);
            let second = call.args.get(1).map(String::as_str);
            let changes = matches!(first, Some("deploy" | "rollback" | "delete"))
                || (first == Some("secret") && matches!(second, Some("put" | "delete")));
            if changes && out.as_ref().is_ok_and(|o| o.success) {
                let mut s = self.0.borrow_mut();
                s.changes += 1;
                if s.drop_after == Some(s.changes) {
                    s.fail_stderr = Some("fetch failed: ENOTFOUND api.cloudflare.com".to_string());
                }
            }
            out
        }
    }

    impl FakeWrangler {
        fn answer(&self, call: &Call) -> Result<Output, Error> {
            let mut s = self.0.borrow_mut();
            s.seen.push(call.args.clone());
            if let Some(e) = &s.fail_stderr {
                return err(e);
            }
            let a: Vec<&str> = call.args.iter().map(String::as_str).collect();
            match a.as_slice() {
                ["deployments", "list", ..] => {
                    if s.exists {
                        if s.deployments.is_empty() {
                            return ok(r#"[{"versions":[{"version_id":"a1"}]}]"#);
                        }
                        let list: Vec<Value> = s
                            .deployments
                            .iter()
                            .map(|i| serde_json::json!({"versions":[{"version_id":format!("a{i}")}]}))
                            .collect();
                        ok(Value::Array(list).to_string())
                    } else {
                        err(
                            "A request to the Cloudflare API failed. Worker not found [code: 10007]",
                        )
                    }
                }
                ["versions", "view", id, ..] => {
                    let at = id.trim_start_matches('a').parse::<usize>().ok();
                    let vars = at
                        .and_then(|i| s.history.get(i))
                        .map_or(&s.vars, |(v, _)| v);
                    let mut bindings: Vec<Value> = vars
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
                ["secret", "put" | "delete", ..] if !s.latest_is_deployed() => err(
                    "Secret edit failed: the latest version of your Worker isn't currently deployed [code: 10215]",
                ),
                ["secret", "put", name, ..] => {
                    s.calls.push(format!("secret put {name}"));
                    s.secrets.insert(name.to_string());
                    s.new_version();
                    ok("")
                }
                ["secret", "delete", name, ..] => {
                    s.calls.push(format!("secret delete {name}"));
                    if s.secrets.remove(*name) {
                        s.new_version();
                        ok("")
                    } else {
                        err("Secret not found [code: 10056]")
                    }
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
                    s.new_version();
                    ok("")
                }
                ["rollback", id, ..] => {
                    let at = id.trim_start_matches('a').parse::<usize>().ok();
                    let Some(i) = at.filter(|i| *i < s.history.len()) else {
                        return err("Version not found [code: 10211]");
                    };
                    // As on a real account (2026-11-08): the version's settings come back, but the Worker's secret list is not touched.
                    s.vars = s.history[i].0.clone();
                    s.deployments.push(i);
                    s.calls.push("rollback".to_string());
                    ok("")
                }
                ["delete", ..] => {
                    s.calls.push("delete".to_string());
                    *s = State {
                        calls: std::mem::take(&mut s.calls),
                        ..State::default()
                    };
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
            Op::DeleteSecret,
            Op::Restore,
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
            // Once all three steps have applied, every call fails: a rollback then must say `Unknown`, never `Restored`.
            Scenario::DropsAfterApply => target(FakeWrangler::dropping_after(3)),
            // Secrets left untrusted: `Unverified` is not `Yes`, so a plan that needs one must be refused.
            Scenario::SecretsUnsupported => {
                CloudflareTarget::new(FakeWrangler::new(), "worker", |_| true)
                    .trust_operations(&[Op::Create, Op::SetSetting])
            }
        });
        assert!(failures.is_empty(), "{failures:?}");
    }

    fn with_rest_domains(w: FakeWrangler, trust_bind: bool) -> CloudflareTarget<FakeWrangler> {
        use crate::cloudflare_domains::{Domains, Rest, tests::FakeRest};
        let rest: Box<dyn Rest> = Box::new(FakeRest::default());
        let ops: &[Op] = if trust_bind {
            &[
                Op::Create,
                Op::SetSetting,
                Op::SetSecret,
                Op::BindDomain,
                Op::ReadTls,
            ]
        } else {
            &[Op::Create, Op::SetSetting, Op::SetSecret]
        };
        CloudflareTarget::new(w, "worker", |_| true)
            .with_domains(Domains::new(rest, "acct1", "zone1").unwrap())
            .trust_operations(ops)
    }

    #[test]
    fn the_conformance_suite_passes_with_domains_bound_through_the_rest_api() {
        let failures = conformance::run(|scenario| match scenario {
            Scenario::Empty | Scenario::DropsAfterApply => {
                with_rest_domains(FakeWrangler::new(), true)
            }
            Scenario::HoldsOtherApp => with_rest_domains(FakeWrangler::other_app(), true),
            Scenario::Unreachable => with_rest_domains(
                FakeWrangler::failing("fetch failed: ENOTFOUND api.cloudflare.com"),
                true,
            ),
            Scenario::FailsLastStepOnce => {
                with_rest_domains(FakeWrangler::failing_secret_put_once(), true)
            }
            Scenario::SecretsUnsupported => {
                CloudflareTarget::new(FakeWrangler::new(), "worker", |_| true)
                    .trust_operations(&[Op::Create, Op::SetSetting])
            }
        });
        assert!(failures.is_empty(), "{failures:?}");
    }

    #[test]
    fn bind_domain_is_no_without_rest_and_unverified_until_trusted() {
        let plain = target(FakeWrangler::new());
        assert_eq!(plain.supports(Op::BindDomain), Support::No);
        let configured = with_rest_domains(FakeWrangler::new(), false);
        assert_eq!(configured.supports(Op::BindDomain), Support::Unverified);
        let trusted = with_rest_domains(FakeWrangler::new(), true);
        assert_eq!(trusted.supports(Op::BindDomain), Support::Yes);
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
    fn a_secret_the_app_must_not_have_is_deleted_through_wrangler() {
        const NAME: &str = "ZZ_FAKE_SECRET_NAME_92";
        let mut host = Host::new(target(FakeWrangler::new()));
        let want = crate::AppManifest::new("conformance-app").secret(NAME);
        let plan = host.plan(&want, &Config::new()).unwrap();
        let secrets = crate::MapSecrets::new().with(NAME, "CANARY-xyz");
        host.apply_approved(&plan, &plan.digest(), &secrets)
            .unwrap();
        let drop = crate::AppManifest::new("conformance-app").without_secret(NAME);
        let plan = host.plan(&drop, &Config::new()).unwrap();
        assert_eq!(plan.steps().len(), 1);
        host.apply_approved(&plan, &plan.digest(), &crate::MapSecrets::new())
            .unwrap();
        assert!(host.target().runner.0.borrow().secrets.is_empty());
        assert!(host.plan(&drop, &Config::new()).unwrap().is_empty());
        // The name stays out of the log.
        assert!(!host.target().diagnostics().contains(NAME));
    }

    fn manifest_for(setting: &str, secret: Option<&str>) -> crate::AppManifest {
        let mut m = crate::AppManifest::new("conformance-app").setting(setting);
        if let Some(s) = secret {
            m = m.secret(s);
        }
        m
    }

    #[test]
    fn rolling_back_returns_settings_and_secret_names_to_the_snapshot() {
        let mut host = Host::new(target(FakeWrangler::new()));
        let one = Config::from([("MODE".to_string(), "one".to_string())]);
        let two = Config::from([("MODE".to_string(), "two".to_string())]);
        let first = host.plan(&manifest_for("MODE", None), &one).unwrap();
        host.apply_approved(&first, &first.digest(), &crate::MapSecrets::new())
            .unwrap();
        let second = host
            .plan(&manifest_for("MODE", Some("TOKEN")), &two)
            .unwrap();
        let secrets = crate::MapSecrets::new().with("TOKEN", "CANARY-xyz");
        let applied = host
            .apply_approved(&second, &second.digest(), &secrets)
            .unwrap();
        let point = applied
            .rollback_point
            .expect("restore is trusted, so there is a point");
        let seen = host.target().observe("conformance-app").unwrap();
        assert_eq!(seen.settings.get("MODE").map(String::as_str), Some("two"));
        assert!(seen.secret_names.contains("TOKEN"));
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);
        let back = host.target().observe("conformance-app").unwrap();
        assert_eq!(back.settings.get("MODE").map(String::as_str), Some("one"));
        assert!(
            back.secret_names.is_empty(),
            "the secret made after the snapshot is gone"
        );
        assert!(!host.target().diagnostics().contains("CANARY-xyz"));
    }

    #[test]
    fn rolling_back_a_first_deploy_deletes_only_a_worker_that_is_ours() {
        let mut host = Host::new(target(FakeWrangler::new()));
        let plan = host
            .plan(
                &manifest_for("MODE", None),
                &Config::from([("MODE".to_string(), "x".to_string())]),
            )
            .unwrap();
        let applied = host
            .apply_approved(&plan, &plan.digest(), &crate::MapSecrets::new())
            .unwrap();
        let point = applied.rollback_point.unwrap();
        assert!(!point.snapshot().existed);
        assert_eq!(host.rollback(&point), crate::Rollback::Restored);
        assert_eq!(
            host.target().locate("conformance-app").unwrap(),
            Presence::Absent
        );
        // A Worker that is not ours is never deleted, whatever the snapshot says.
        let mut t = target(FakeWrangler::other_app());
        let snap = Snapshot {
            existed: false,
            observed: Observed::default(),
            handle: None,
        };
        assert!(matches!(
            t.restore("conformance-app", &snap),
            Err(Error::NotOurs { .. })
        ));
        assert!(
            !t.runner.0.borrow().calls.iter().any(|c| c == "delete"),
            "no delete was sent"
        );
    }

    #[test]
    fn a_version_id_that_could_be_a_flag_is_never_sent() {
        let mut t = target(FakeWrangler::new());
        for bad in ["--force", "a b", "", "x;rm", &"a".repeat(65)] {
            let snap = Snapshot {
                existed: true,
                observed: Observed::default(),
                handle: Some(bad.to_string()),
            };
            assert!(
                matches!(t.restore("conformance-app", &snap), Err(Error::Target(_))),
                "{bad}"
            );
        }
        let none = Snapshot {
            existed: true,
            observed: Observed::default(),
            handle: None,
        };
        assert!(t.restore("conformance-app", &none).is_err());
        assert!(t.runner.0.borrow().seen.is_empty(), "no call was made");
    }

    #[test]
    fn the_rollback_and_delete_calls_never_wait_for_an_answer() {
        // Wrangler asks "are you sure"; with no terminal that would hang or abort, so both carry the flag that accepts.
        assert!(rollback_args("abc-123", "x").contains(&"--yes".to_string()));
        assert!(delete_worker_args("x", ".").contains(&"--force".to_string()));
    }

    #[test]
    fn restore_is_unverified_until_trusted() {
        let plain = CloudflareTarget::new(FakeWrangler::new(), "worker", |_| true);
        assert_eq!(plain.supports(Op::Restore), Support::Unverified);
        assert_eq!(
            target(FakeWrangler::new()).supports(Op::Restore),
            Support::Yes
        );
    }

    #[test]
    fn a_failed_rollback_is_an_error_and_says_so() {
        let mut t = target(FakeWrangler::new());
        let snap = Snapshot {
            existed: true,
            observed: Observed::default(),
            handle: Some("ff".to_string()),
        };
        let e = t.restore("conformance-app", &snap).unwrap_err();
        assert!(e.to_string().contains("rollback failed"), "{e}");
    }

    #[test]
    fn a_deploy_is_waited_for_when_asked_and_a_secret_put_is_not() {
        use std::cell::Cell;
        use std::rc::Rc;
        let asked = Rc::new(Cell::new(0));
        let seen = asked.clone();
        let mut t = target(FakeWrangler::new()).with_ready_wait(move |_| {
            seen.set(seen.get() + 1);
            true
        });
        t.apply_step("conformance-app", &Step::Create, None)
            .unwrap();
        t.apply_step(
            "conformance-app",
            &Step::SetSetting {
                name: "A".into(),
                value: "1".into(),
            },
            None,
        )
        .unwrap();
        assert_eq!(asked.get(), 2);
        t.apply_step(
            "conformance-app",
            &Step::SetSecret { name: "K".into() },
            Some(&SecretValue::new("v")),
        )
        .unwrap();
        assert_eq!(asked.get(), 2, "a secret put does not deploy");
    }

    #[test]
    fn a_deploy_whose_app_never_comes_up_fails_the_step() {
        let mut t = target(FakeWrangler::new()).with_ready_wait(|_| false);
        let e = t
            .apply_step("conformance-app", &Step::Create, None)
            .unwrap_err();
        assert!(e.to_string().contains("did not become healthy"), "{e}");
        assert!(t.diagnostics().contains("did not become healthy"));
        // Without the wait, the same deploy is a success.
        let mut plain = target(FakeWrangler::new());
        plain
            .apply_step("conformance-app", &Step::Create, None)
            .unwrap();
    }

    fn shell(script: &str) -> Command {
        if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", script]);
            c
        } else {
            let mut c = Command::new("sh");
            c.args(["-c", script]);
            c
        }
    }

    #[test]
    fn a_notice_on_stderr_does_not_make_a_successful_call_fail() {
        // Humboldt's demo: npm printed a notice on stderr and a PowerShell script read it as an error. Only the exit code decides.
        let notice = if cfg!(windows) {
            "echo npm notice a new version is available 1>&2"
        } else {
            "echo npm notice a new version is available >&2"
        };
        let out = run_bounded(shell(notice), None, Duration::from_secs(30), "notice").unwrap();
        assert!(out.success);
        assert!(out.stderr.contains("notice"), "the notice was captured");
        let failed = run_bounded(shell("exit 3"), None, Duration::from_secs(30), "exit").unwrap();
        assert!(!failed.success);
    }

    #[test]
    fn a_failed_secret_delete_is_an_error_and_the_name_stays_out_of_it() {
        const NAME: &str = "ZZ_FAKE_SECRET_NAME_93";
        let mut t = target(FakeWrangler::new());
        let step = Step::DeleteSecret {
            name: NAME.to_string(),
        };
        // The fake answers "Secret not found" for a secret it does not hold.
        let e = t.apply_step("x", &step, None).unwrap_err();
        assert!(!e.to_string().contains(NAME), "{e}");
        assert!(!t.diagnostics().contains(NAME));
    }

    #[test]
    fn no_log_error_or_call_text_carries_a_secret_name() {
        const NAME: &str = "ZZ_FAKE_SECRET_NAME_91";
        let step = Step::SetSecret {
            name: NAME.to_string(),
        };
        let value = SecretValue::new("CANARY-xyz");
        // A success, a failure, and the text of the call itself.
        let mut ok = target(FakeWrangler::new());
        ok.apply_step("x", &step, Some(&value)).unwrap();
        assert!(
            !ok.diagnostics().contains(NAME),
            "{}",
            ok.diagnostics().len()
        );
        let w = FakeWrangler::new();
        w.0.borrow_mut().fail_stderr = Some("rejected".to_string());
        let mut bad = target(w);
        let e = bad.apply_step("x", &step, Some(&value)).unwrap_err();
        assert!(!e.to_string().contains(NAME));
        assert!(!bad.diagnostics().contains(NAME));
        let missing = bad.apply_step("x", &step, None).unwrap_err();
        assert!(!missing.to_string().contains(NAME));
        let call = Call {
            args: ["secret", "put", NAME, "--name", "x"]
                .iter()
                .map(|a| a.to_string())
                .collect(),
            stdin: Some(value),
        };
        assert!(!format!("{call:?}").contains(NAME));
        assert!(call.describe().contains("--name x"));
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

    /// A dry run: real `wrangler deploy --dry-run` builds the Worker and exits before any upload, so it needs no account (DESIGN.md D6).
    /// It proves the real wrangler accepts the exact flags `deploy` sends; the Cloudflare side still waits for a test account.
    fn dry_run_deploy(extra_flag: Option<&str>) -> Output {
        let dir = std::env::temp_dir().join(format!(
            "webhostant-dry-{}-{}",
            std::process::id(),
            extra_flag.is_some()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("index.js"),
            "export default { fetch() { return new Response(\"ok\"); } };
",
        )
        .unwrap();
        std::fs::write(
            dir.join("wrangler.toml"),
            "name = \"dry-app\"
main = \"index.js\"
compatibility_date = \"2025-01-01\"
",
        )
        .unwrap();
        let t = CloudflareTarget::new(ProcessRunner::default(), &dir, |_| true);
        let mut args = t.deploy_args("dry-app", &[("APP_BASE_URL", "https://example.test")]);
        args.push("--dry-run".to_string());
        args.extend(extra_flag.map(str::to_string));
        let out = ProcessRunner::default().run(&Call { args, stdin: None });
        let _ = std::fs::remove_dir_all(&dir);
        out.expect("wrangler must be installed (npm i -g wrangler); CI installs it")
    }

    #[test]
    fn real_wrangler_accepts_the_deploy_arguments_in_a_dry_run() {
        let out = dry_run_deploy(None);
        assert!(out.success, "wrangler refused: {}", out.stderr);
        assert!(
            out.stdout.contains("--dry-run: exiting now"),
            "{}",
            out.stdout
        );
        // Variables are shown by name with the value hidden, so no value leaks into the output we keep.
        assert!(out.stdout.contains("env.APP_BASE_URL"), "{}", out.stdout);
        assert!(!out.stdout.contains("example.test"), "{}", out.stdout);
    }

    #[test]
    fn the_dry_run_can_fail() {
        // Positive control: the same call with a flag wrangler does not know must fail, so the test above is not a pass-through.
        let out = dry_run_deploy(Some("--no-such-flag"));
        assert!(!out.success);
    }

    /// Run real `wrangler` with no credentials and an empty config directory, so it checks its arguments and then stops at "set a
    /// CLOUDFLARE_API_TOKEN" before any network call (it cannot touch a real account, even on a machine where someone is logged in).
    fn without_credentials(args: &[String]) -> String {
        let cfg = std::env::temp_dir().join(format!("webhostant-noauth-{}", std::process::id()));
        std::fs::create_dir_all(&cfg).unwrap();
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "wrangler"]);
            c
        } else {
            Command::new("wrangler")
        };
        let out = cmd
            .args(args)
            .env("CI", "1")
            .env("XDG_CONFIG_HOME", &cfg)
            .env("WRANGLER_SEND_METRICS", "false")
            .env_remove("CLOUDFLARE_API_TOKEN")
            .env_remove("CLOUDFLARE_API_KEY")
            .env_remove("CLOUDFLARE_EMAIL")
            .env_remove("CLOUDFLARE_ACCOUNT_ID")
            .stdin(Stdio::null())
            .output()
            .expect("wrangler must be installed (npm i -g wrangler); CI installs it");
        let _ = std::fs::remove_dir_all(&cfg);
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    }

    const NOT_PARSED: [&str; 4] = [
        "Unknown argument",
        "Unknown command",
        "Not enough non-option arguments",
        "Too many non-option arguments",
    ];

    #[test]
    fn real_wrangler_accepts_the_arguments_of_every_read_and_secret_call() {
        // Messages name the call by a fixed label, never by its arguments.
        for (label, args) in [
            ("deployments list", deployments_list_args("some-app")),
            (
                "versions view",
                versions_view_args("0000-version", "some-app"),
            ),
            ("secret list", secret_list_args("some-app")),
            ("secret put", secret_name_put_args("SOME_NAME", "some-app")),
            ("secret delete", secret_delete_args("SOME_NAME", "some-app")),
            ("rollback", rollback_args("0000-version", "some-app")),
            ("delete", delete_worker_args("some-app", ".")),
        ] {
            let text = without_credentials(&args);
            assert!(
                text.contains("non-interactive environment"),
                "`wrangler {label}` never got past argument checking"
            );
            for bad in NOT_PARSED {
                assert!(
                    !text.contains(bad),
                    "`wrangler {label}` was refused with: {bad}"
                );
            }
        }
    }

    #[test]
    fn the_argument_check_can_fail() {
        // Positive control: an unknown flag on a real call is reported by wrangler, so the test above is not a pass-through.
        let mut args = deployments_list_args("some-app");
        args.push("--no-such-flag".to_string());
        let text = without_credentials(&args);
        assert!(text.contains("Unknown argument"), "{text}");
        assert!(!text.contains("non-interactive environment"), "{text}");
    }
}
