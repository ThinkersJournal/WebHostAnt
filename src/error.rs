//! Errors. Each says what the person should do next, and none carries a secret value.

use std::fmt;
use std::time::Duration;

use crate::{Op, RollbackPoint, StepOutcome};

/// Everything WebHostAnt can refuse or fail at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The target holds something that is not this app. Nothing was changed.
    NotOurs {
        /// The app id that was being deployed.
        app: String,
    },
    /// The target does not declare `Yes` for an operation the plan needs.
    Unsupported {
        /// The target's name.
        target: String,
        /// The operation that is not supported.
        op: Op,
    },
    /// A secret the plan needs was not supplied. Nothing was changed.
    MissingSecret {
        /// The secret's name (never its value).
        name: String,
    },
    /// A setting the manifest declares has no value in the configuration.
    MissingSetting {
        /// The setting's name.
        name: String,
    },
    /// After applying, the target's own state did not match the plan.
    ReadBackMismatch {
        /// What differed, by name.
        details: Vec<String>,
    },
    /// An apply began changing the target and then stopped. Some steps may have run.
    ApplyFailed {
        /// What happened to each step, in plan order.
        steps: Vec<StepOutcome>,
        /// Why it stopped.
        cause: Box<Error>,
        /// Present when the target can be put back: pass it to `Host::rollback`. Absent when the target cannot undo.
        rollback_point: Option<Box<RollbackPoint>>,
    },
    /// A call to the target did not finish in time and was stopped. What it was doing may be half done.
    TimedOut {
        /// The call, by name (arguments only, never a secret value).
        what: String,
        /// The limit it exceeded.
        after: Duration,
    },
    /// A domain the manifest declares is not a plain host name. Nothing was changed.
    InvalidDomain {
        /// The name as declared.
        name: String,
    },
    /// The plan to apply is not the plan that was approved. Nothing was changed.
    PlanChanged,
    /// The target reported a failure of its own.
    Target(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NotOurs { app } => write!(
                f,
                "the target holds something that is not `{app}`; nothing was changed. Pick another name or target, or remove the other app yourself"
            ),
            Error::Unsupported { target, op } => write!(
                f,
                "target `{target}` does not support {op:?}; choose a target that does, or do that step by hand"
            ),
            Error::MissingSecret { name } => write!(
                f,
                "secret `{name}` was not supplied; nothing was changed. Provide it (or paste it into the host by hand) and run again"
            ),
            Error::MissingSetting { name } => write!(
                f,
                "setting `{name}` has no value; add it to the configuration"
            ),
            Error::ReadBackMismatch { details } => write!(
                f,
                "the target does not show what was applied ({}); re-run the plan to see what is left",
                details.join(", ")
            ),
            Error::ApplyFailed {
                steps,
                cause,
                rollback_point,
            } => {
                let n = |o: StepOutcome| steps.iter().filter(|s| **s == o).count();
                write!(
                    f,
                    "the apply stopped ({} applied, {} failed, {} not run): {cause}. {}",
                    n(StepOutcome::Applied),
                    n(StepOutcome::Failed),
                    n(StepOutcome::Skipped),
                    if rollback_point.is_some() {
                        "Roll back with the rollback point, or plan again to finish what is left"
                    } else {
                        "This target cannot undo; plan again to see what is left"
                    }
                )
            }
            Error::TimedOut { what, after } => write!(
                f,
                "`{what}` did not finish within {}s and was stopped; the target may be partly changed, so plan again to see what is left before retrying",
                after.as_secs_f32()
            ),
            Error::InvalidDomain { name } => write!(
                f,
                "`{name}` is not a host name WebHostAnt can bind; use letters, digits, hyphens and dots only, like app.example.org (no scheme, port, path, wildcard or address)"
            ),
            Error::PlanChanged => write!(
                f,
                "this plan is not the one that was approved, so nothing was applied; make a new plan and approve it again"
            ),
            Error::Target(msg) => write!(f, "the target reported: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
