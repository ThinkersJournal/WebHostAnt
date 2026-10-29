//! The interface a host adapter implements.

use std::collections::{BTreeMap, BTreeSet};

use crate::{Error, SecretValue};

/// An operation a target may or may not support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Op {
    /// Create the app on the target.
    Create,
    /// Set a plain setting.
    SetSetting,
    /// Set a secret.
    SetSecret,
}

/// What a target declares for an operation. Only `Yes` satisfies a plan; `Unverified` is the default for anything not proven.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Support {
    /// Proven.
    Yes,
    /// Known not to work.
    No,
    /// Not proven either way.
    Unverified,
}

/// One change a plan would make. Secrets are named, never valued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Create the app.
    Create,
    /// Set a plain setting to a value.
    SetSetting {
        /// Setting name.
        name: String,
        /// Its new value.
        value: String,
    },
    /// Set a secret; the value is supplied at apply time.
    SetSecret {
        /// Secret name.
        name: String,
    },
}

impl Step {
    /// The operation this step needs.
    pub fn op(&self) -> Op {
        match self {
            Step::Create => Op::Create,
            Step::SetSetting { .. } => Op::SetSetting,
            Step::SetSecret { .. } => Op::SetSecret,
        }
    }
}

/// What a target holds under an app id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    /// Nothing under that id.
    Absent,
    /// This app.
    Ours,
    /// Something else is there.
    NotOurs,
}

/// What a target reports about the app, read from the target itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observed {
    /// Current plain settings.
    pub settings: BTreeMap<String, String>,
    /// Names of secrets that exist (never values).
    pub secret_names: BTreeSet<String>,
    /// Whether the app answers its health path.
    pub healthy: bool,
}

/// A place an app can be put. `locate` and `observe` take `&self` so planning cannot change a target.
pub trait Target {
    /// A short stable name, for messages.
    fn name(&self) -> &str;
    /// Whether this target supports an operation.
    fn supports(&self, op: Op) -> Support;
    /// What is under this app id.
    fn locate(&self, app_id: &str) -> Result<Presence, Error>;
    /// The app's state as the target reports it. Only meaningful when `locate` says `Ours`.
    fn observe(&self, app_id: &str) -> Result<Observed, Error>;
    /// Make one change. `secret` is `Some` only for `Step::SetSecret`.
    fn apply_step(
        &mut self,
        app_id: &str,
        step: &Step,
        secret: Option<&SecretValue>,
    ) -> Result<(), Error>;
    /// Everything the target has logged so far. The conformance suite searches this for leaked secrets.
    fn diagnostics(&self) -> String;
}
