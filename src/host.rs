//! Plan, apply, read back: the one path every target goes through.

use std::fmt;

use crate::{AppManifest, Config, Error, Observed, Presence, Secrets, Step, Support, Target};

/// An inert list of changes. Holds secret names, never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    app_id: String,
    steps: Vec<Step>,
}

impl Plan {
    /// Build a plan by hand (tests, tools). `Host::plan` is the normal source.
    pub fn new(app_id: impl Into<String>, steps: Vec<Step>) -> Self {
        Self {
            app_id: app_id.into(),
            steps,
        }
    }

    /// The app this plan is for.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The changes, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// True when the target already matches.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

impl fmt::Display for Plan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.steps.is_empty() {
            return writeln!(f, "{}: nothing to change", self.app_id);
        }
        for step in &self.steps {
            match step {
                Step::Create => writeln!(f, "create {}", self.app_id)?,
                Step::SetSetting { name, value } => writeln!(f, "set {name} = {value}")?,
                Step::SetSecret { name } => writeln!(f, "set secret {name} (value not shown)")?,
            }
        }
        Ok(())
    }
}

/// What the target showed after an apply, by name. Never a secret value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadBack {
    /// Settings confirmed on the target.
    pub settings: Vec<String>,
    /// Secret names confirmed to exist.
    pub secret_names: Vec<String>,
    /// Whether the app answered its health check.
    pub healthy: bool,
}

/// The result of an apply: only returned when the read-back matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// How many steps ran.
    pub steps_applied: usize,
    /// What the target showed afterwards.
    pub read_back: ReadBack,
}

/// A target plus the plan/apply/read-back discipline.
#[derive(Debug)]
pub struct Host<T: Target> {
    target: T,
}

impl<T: Target> Host<T> {
    /// Wrap a target.
    pub fn new(target: T) -> Self {
        Self { target }
    }

    /// The target, read-only.
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Consume the host and return the target.
    pub fn into_target(self) -> T {
        self.target
    }

    fn require(&self, step: &Step) -> Result<(), Error> {
        if self.target.supports(step.op()) == Support::Yes {
            Ok(())
        } else {
            Err(Error::Unsupported {
                target: self.target.name().to_string(),
                op: step.op(),
            })
        }
    }

    /// Work out what would change. Inert: takes `&self` and only reads.
    pub fn plan(&self, manifest: &AppManifest, config: &Config) -> Result<Plan, Error> {
        let presence = self.target.locate(&manifest.id)?;
        let mut steps = Vec::new();
        let observed = match presence {
            Presence::NotOurs => {
                return Err(Error::NotOurs {
                    app: manifest.id.clone(),
                });
            }
            Presence::Absent => {
                steps.push(Step::Create);
                Observed::default()
            }
            Presence::Ours => self.target.observe(&manifest.id)?,
        };
        for name in &manifest.settings {
            let value = config
                .get(name)
                .ok_or_else(|| Error::MissingSetting { name: name.clone() })?;
            if observed.settings.get(name) != Some(value) {
                steps.push(Step::SetSetting {
                    name: name.clone(),
                    value: value.clone(),
                });
            }
        }
        for name in &manifest.secrets {
            if !observed.secret_names.contains(name) {
                steps.push(Step::SetSecret { name: name.clone() });
            }
        }
        for step in &steps {
            self.require(step)?;
        }
        Ok(Plan {
            app_id: manifest.id.clone(),
            steps,
        })
    }

    /// Make the plan so, then read the target's own state back. `Ok` means the read-back matched, not that a command returned.
    ///
    /// Everything that can be checked without acting (ownership, support, secrets present) is checked first, so a refusal changes nothing.
    pub fn apply(&mut self, plan: &Plan, secrets: &dyn Secrets) -> Result<Applied, Error> {
        if self.target.locate(&plan.app_id)? == Presence::NotOurs {
            return Err(Error::NotOurs {
                app: plan.app_id.clone(),
            });
        }
        let mut values = Vec::with_capacity(plan.steps.len());
        for step in &plan.steps {
            self.require(step)?;
            values.push(match step {
                Step::SetSecret { name } => Some(
                    secrets
                        .get(name)
                        .ok_or_else(|| Error::MissingSecret { name: name.clone() })?,
                ),
                _ => None,
            });
        }
        for (step, value) in plan.steps.iter().zip(&values) {
            self.target.apply_step(&plan.app_id, step, value.as_ref())?;
        }
        let read_back = self.verify(plan)?;
        Ok(Applied {
            steps_applied: plan.steps.len(),
            read_back,
        })
    }

    fn verify(&self, plan: &Plan) -> Result<ReadBack, Error> {
        if self.target.locate(&plan.app_id)? != Presence::Ours {
            return Err(Error::ReadBackMismatch {
                details: vec!["app is not present".to_string()],
            });
        }
        let observed = self.target.observe(&plan.app_id)?;
        let mut read_back = ReadBack {
            healthy: observed.healthy,
            ..ReadBack::default()
        };
        let mut bad = Vec::new();
        for step in &plan.steps {
            match step {
                Step::Create => {}
                Step::SetSetting { name, value } => {
                    if observed.settings.get(name) == Some(value) {
                        read_back.settings.push(name.clone());
                    } else {
                        bad.push(format!("setting {name}"));
                    }
                }
                Step::SetSecret { name } => {
                    if observed.secret_names.contains(name) {
                        read_back.secret_names.push(name.clone());
                    } else {
                        bad.push(format!("secret {name}"));
                    }
                }
            }
        }
        if !observed.healthy {
            bad.push("health".to_string());
        }
        if bad.is_empty() {
            Ok(read_back)
        } else {
            Err(Error::ReadBackMismatch { details: bad })
        }
    }
}
