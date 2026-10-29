//! An in-memory target, for the README example, consumers' tests and the conformance suite's own tests.
//!
//! It is not a real host. The `Fault` variants are deliberate defects: the conformance suite must fail on each.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, Observed, Op, Presence, SecretValue, Step, Support, Target};

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
    log: Vec<String>,
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
            log: Vec::new(),
        }
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
            Op::Create | Op::SetSetting => Support::Yes,
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
            healthy: self.present.get() && self.healthy,
        })
    }

    fn apply_step(
        &mut self,
        _app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error> {
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
            Step::SetSecret { name } => {
                let value = secret
                    .ok_or_else(|| Error::Target(format!("no value for {name}")))?
                    .expose()
                    .to_string();
                if self.fault == Fault::LeaksSecret {
                    self.log.push(format!("set secret {name}={value}"));
                } else {
                    self.log.push(format!("set secret {name}"));
                }
                self.secret_values.insert(name.clone(), value);
            }
        }
        Ok(())
    }

    fn diagnostics(&self) -> String {
        self.log.join("\n")
    }
}
