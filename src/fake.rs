//! An in-memory target, for the README example, consumers' tests and the conformance suite's own tests.
//!
//! It is not a real host. The `Fault` variants are deliberate defects: the conformance suite must fail on each.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    Error, Observed, Op, Presence, Renewal, SecretValue, Snapshot, Step, Support, Target, TlsStatus,
};

/// A deliberate defect, to prove the conformance suite can see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    /// Behaves correctly.
    None,
    /// `apply_step` reports success and does nothing.
    SilentNoop,
    /// Writes secret values into its log.
    LeaksSecret,
    /// `locate` creates the app, so planning changes the target.
    ActsDuringPlan,
    /// When unreachable, `locate` says `Absent` instead of failing.
    UnreachableIsAbsent,
    /// `restore` reports success and does nothing.
    RestoreDoesNothing,
    /// A bind step reports success and binds nothing.
    DomainNotBound,
    /// `tls_status` answers with an empty status when the host cannot be reached.
    TlsEmptyInsteadOfError,
}

/// An in-memory target.
#[derive(Debug)]
pub struct FakeTarget {
    fault: Fault,
    holds_other_app: bool,
    unreachable: bool,
    secrets_support: Support,
    present: Cell<bool>,
    healthy: bool,
    settings: BTreeMap<String, String>,
    secret_values: BTreeMap<String, String>,
    domains: BTreeSet<String>,
    log: Vec<String>,
    calls: usize,
    steps_done: usize,
    fail_call: Option<usize>,
    down_after: Option<usize>,
}

impl FakeTarget {
    /// An empty, correct target.
    pub fn new() -> Self {
        Self {
            fault: Fault::None,
            holds_other_app: false,
            unreachable: false,
            secrets_support: Support::Yes,
            present: Cell::new(false),
            healthy: false,
            settings: BTreeMap::new(),
            secret_values: BTreeMap::new(),
            domains: BTreeSet::new(),
            log: Vec::new(),
            calls: 0,
            steps_done: 0,
            fail_call: None,
            down_after: None,
        }
    }

    /// Make the `n`th call to `apply_step` (counting from 0) fail once; later calls work.
    pub fn failing_step_once(mut self, n: usize) -> Self {
        self.fail_call = Some(n);
        self
    }

    /// Make the host unreachable once `n` steps have been applied, as if the network dropped mid-deploy.
    pub fn going_down_after(mut self, n: usize) -> Self {
        self.down_after = Some(n);
        self
    }

    /// Give it a deliberate defect.
    pub fn with_fault(mut self, fault: Fault) -> Self {
        self.fault = fault;
        self
    }

    /// Make it hold some other app under every id.
    pub fn holding_other_app(mut self) -> Self {
        self.holds_other_app = true;
        self
    }

    /// Make every call to the host fail, as if the network were down.
    pub fn unreachable(mut self) -> Self {
        self.unreachable = true;
        self
    }

    /// Declare what it says about setting secrets.
    pub fn with_secrets_support(mut self, support: Support) -> Self {
        self.secrets_support = support;
        self
    }
}

impl Default for FakeTarget {
    fn default() -> Self {
        Self::new()
    }
}

impl Target for FakeTarget {
    fn name(&self) -> &str {
        "fake"
    }

    fn supports(&self, op: Op) -> Support {
        match op {
            Op::SetSecret => self.secrets_support,
            Op::Create | Op::SetSetting | Op::Restore | Op::ReadTls | Op::BindDomain => {
                Support::Yes
            }
        }
    }

    fn locate(&self, _app_id: &str) -> Result<Presence, Error> {
        if self.unreachable {
            return if self.fault == Fault::UnreachableIsAbsent {
                Ok(Presence::Absent)
            } else {
                Err(Error::Target("host unreachable".to_string()))
            };
        }
        if self.fault == Fault::ActsDuringPlan {
            self.present.set(true);
        }
        Ok(if self.holds_other_app {
            Presence::NotOurs
        } else if self.present.get() {
            Presence::Ours
        } else {
            Presence::Absent
        })
    }

    fn observe(&self, _app_id: &str) -> Result<Observed, Error> {
        if self.unreachable {
            return Err(Error::Target("host unreachable".to_string()));
        }
        Ok(Observed {
            settings: self.settings.clone(),
            secret_names: self.secret_values.keys().cloned().collect::<BTreeSet<_>>(),
            domains: self.domains.clone(),
            healthy: self.present.get() && self.healthy,
        })
    }

    fn apply_step(
        &mut self,
        _app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
        let call = self.calls;
        self.calls += 1;
        if self.fail_call == Some(call) {
            self.fail_call = None;
            return Err(Error::Target("injected failure".to_string()));
        }
        if self.unreachable {
            return Err(Error::Target("host unreachable".to_string()));
        }
        if self.holds_other_app {
            return Err(Error::Target("holds another app".to_string()));
        }
        if self.fault == Fault::SilentNoop {
            return Ok(());
        }
        match step {
            Step::Create => {
                self.present.set(true);
                self.healthy = true;
                self.log.push("created".to_string());
            }
            Step::SetSetting { name, value } => {
                self.settings.insert(name.clone(), value.clone());
                self.log.push(format!("set {name}"));
            }
            Step::BindDomain { name } => {
                if self.fault != Fault::DomainNotBound {
                    self.domains.insert(name.clone());
                }
                self.log.push(format!("bound {name}"));
            }
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target(format!("no value for {name}")))?
                    .expose()
                    .to_string();
                if self.fault == Fault::LeaksSecret {
                    self.log.push(format!("set secret {name}={value}"));
                } else {
                    self.log.push("set secret".to_string());
                }
                self.secret_values.insert(name.clone(), value);
            }
        }
        self.steps_done += 1;
        if self.down_after == Some(self.steps_done) {
            self.unreachable = true;
        }
        Ok(())
    }

    fn tls_status(&self, _app_id: &str) -> Result<TlsStatus, Error> {
        if self.fault == Fault::TlsEmptyInsteadOfError && self.unreachable {
            return Ok(TlsStatus {
                names: Vec::new(),
                not_after: None,
                issuer: None,
                last_renewal: Renewal::Unknown,
            });
        }
        if self.unreachable {
            return Err(Error::Target("host unreachable".to_string()));
        }
        Ok(TlsStatus {
            names: vec!["app.example.invalid".to_string()],
            not_after: Some("2099-01-01T00:00:00Z".to_string()),
            issuer: Some("Fake CA".to_string()),
            last_renewal: Renewal::Succeeded,
        })
    }

    fn restore(&mut self, _app_id: &str, snapshot: &Snapshot) -> Result<(), Error> {
        if self.unreachable {
            return Err(Error::Target("host unreachable".to_string()));
        }
        if self.fault == Fault::RestoreDoesNothing {
            return Ok(());
        }
        if snapshot.existed {
            self.settings = snapshot.observed.settings.clone();
            // A secret's old value cannot be read back, so only the name set is restored.
            let keep = &snapshot.observed.secret_names;
            self.secret_values.retain(|name, _| keep.contains(name));
        } else {
            self.present.set(false);
            self.healthy = false;
            self.settings.clear();
            self.secret_values.clear();
        }
        self.log.push("restored".to_string());
        Ok(())
    }

    fn diagnostics(&self) -> String {
        self.log.join("\n")
    }
}
