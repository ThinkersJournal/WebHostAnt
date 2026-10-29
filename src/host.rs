//! Plan, apply, read back: the one path every target goes through.

use std::fmt;

use sha2::{Digest, Sha256};

use crate::{
    AppManifest, Config, Error, Migrations, Observed, Op, Presence, Secrets, Snapshot, Step,
    Support, Target, TlsStatus,
};

/// Whether a release changes the consumer's data schema. Only the consumer knows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Release {
    /// The consumer did not say. A plan then cannot say whether data can follow a rollback.
    Unknown,
    /// This release changes the schema.
    MigratesSchema,
    /// This release leaves the schema as it was.
    NoSchemaChange,
}

/// What a plan says about data if the code is later rolled back. Stated before apply; rollback itself never touches data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataAfterRollback {
    /// The app declared no data to protect.
    NoData,
    /// Older code runs on the data as it will be.
    Follows,
    /// Older code will refuse the new data: only a backup restore will do.
    NeedsBackupRestore,
    /// The consumer did not say enough to tell.
    Unknown,
}

impl DataAfterRollback {
    fn derive(policy: Option<Migrations>, release: Release) -> Self {
        match (policy, release) {
            (None, _) => Self::Unknown,
            (Some(Migrations::NoData), _) => Self::NoData,
            (Some(_), Release::Unknown) => Self::Unknown,
            (Some(_), Release::NoSchemaChange) => Self::Follows,
            (Some(Migrations::ForwardOnlyOlderRefuses), Release::MigratesSchema) => {
                Self::NeedsBackupRestore
            }
            (Some(_), Release::MigratesSchema) => Self::Follows,
        }
    }

    /// The sentence a plan prints.
    pub fn sentence(self) -> &'static str {
        match self {
            Self::NoData => "this app declares no data, so a rollback has none to lose",
            Self::Follows => "after a rollback, older code will run on the data as it will be",
            Self::NeedsBackupRestore => {
                "after a rollback, older code will REFUSE the new data; only a backup restore will do"
            }
            Self::Unknown => {
                "whether data can follow a rollback is NOT KNOWN; the consumer did not say"
            }
        }
    }
}

/// An inert list of changes. Holds secret names, never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    app_id: String,
    steps: Vec<Step>,
    undoable: bool,
    data: DataAfterRollback,
}

impl Plan {
    /// Build a plan by hand (tests, tools). `Host::plan` is the normal source.
    pub fn new(app_id: impl Into<String>, steps: Vec<Step>) -> Self {
        Self {
            app_id: app_id.into(),
            steps,
            undoable: false,
            data: DataAfterRollback::Unknown,
        }
    }

    /// Whether the target this plan was made for can put the app back. Always `false` for a hand-built plan: say "cannot undo" unless known.
    pub fn undoable(&self) -> bool {
        self.undoable
    }

    /// What this plan says about data if the code is rolled back.
    pub fn data_after_rollback(&self) -> DataAfterRollback {
        self.data
    }

    /// The app this plan is for.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The changes, in order.
    pub fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// A fingerprint of this plan: the app, the undo note and every step in order, with setting values and secret *names* (never secret values,
    /// which a plan does not hold). Show it with the plan; pass it to [`Host::apply_approved`] to apply only what was approved.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        let mut field = |tag: u8, text: &str| {
            h.update([tag]);
            h.update((text.len() as u64).to_be_bytes());
            h.update(text.as_bytes());
        };
        field(b'v', "webhostant-plan-v1");
        field(b'a', &self.app_id);
        field(b'u', if self.undoable { "1" } else { "0" });
        field(b'D', self.data.sentence());
        for step in &self.steps {
            match step {
                Step::Create => field(b'c', ""),
                Step::SetSetting { name, value } => {
                    field(b's', name);
                    field(b'=', value);
                }
                Step::SetSecret { name } => field(b'k', name),
                Step::BindDomain { name } => field(b'd', name),
            }
        }
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
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
        writeln!(f, "DATA: {}", self.data.sentence())?;
        if !self.undoable {
            writeln!(f, "note: this target cannot undo these changes")?;
        }
        for step in &self.steps {
            match step {
                Step::Create => writeln!(f, "create {}", self.app_id)?,
                Step::SetSetting { name, value } => writeln!(f, "set {name} = {value}")?,
                Step::SetSecret { name } => writeln!(f, "set secret {name} (value not shown)")?,
                Step::BindDomain { name } => writeln!(f, "bind domain {name}")?,
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
    /// Domain names confirmed bound.
    pub domains: Vec<String>,
    /// Whether the app answered its health check.
    pub healthy: bool,
}

/// The result of an apply: only returned when the read-back matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// How many steps ran.
    pub steps_applied: usize,
    /// What to pass to [`Host::rollback`]. `None` when the target cannot put the app back.
    pub rollback_point: Option<Box<RollbackPoint>>,
    /// What the target showed afterwards.
    pub read_back: ReadBack,
}

/// What happened to one step of an apply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepOutcome {
    /// It ran.
    Applied,
    /// It was not attempted, because an earlier step failed.
    Skipped,
    /// It was attempted and failed; it may have changed the target partway.
    Failed,
}

/// Where the target stood before an apply began. Holds secret names, never values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackPoint {
    app_id: String,
    snapshot: Snapshot,
    attempted: usize,
}

impl RollbackPoint {
    /// The app this point is for.
    pub fn app_id(&self) -> &str {
        &self.app_id
    }

    /// The state to go back to.
    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

/// The result of a rollback. Never `Restored` unless the target's own state was read back and matched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rollback {
    /// The target now matches the snapshot, as read back.
    Restored,
    /// Nothing had been applied, so there was nothing to undo.
    Unchanged,
    /// The state is genuinely not known: the restore failed, the host could not be reached, or the read-back did not match.
    Unknown {
        /// Why it is not known.
        reason: String,
        /// What differed, by name, when the read-back ran and did not match.
        details: Vec<String>,
    },
}

impl Rollback {
    fn unknown(reason: impl Into<String>) -> Self {
        Rollback::Unknown {
            reason: reason.into(),
            details: Vec::new(),
        }
    }
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
        self.plan_release(manifest, config, Release::Unknown)
    }

    /// Like [`Host::plan`], and states whether data can follow a rollback, from the manifest's [`Migrations`] and what the consumer says about this release.
    pub fn plan_release(
        &self,
        manifest: &AppManifest,
        config: &Config,
        release: Release,
    ) -> Result<Plan, Error> {
        if let Some(bad) = manifest
            .domains
            .iter()
            .find(|d| !crate::manifest::valid_domain(d))
        {
            return Err(Error::InvalidDomain { name: bad.clone() });
        }
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
        for name in &manifest.domains {
            if !observed.domains.contains(name) {
                steps.push(Step::BindDomain { name: name.clone() });
            }
        }
        for step in &steps {
            self.require(step)?;
        }
        Ok(Plan {
            app_id: manifest.id.clone(),
            steps,
            undoable: self.can_roll_back(),
            data: DataAfterRollback::derive(manifest.migrations, release),
        })
    }

    /// What the target says about the app's domains and certificate. Read-only. An app that is absent or not ours, an unreachable host and a
    /// target that cannot report this are errors; an empty status is never made up.
    pub fn tls_status(&self, app_id: &str) -> Result<TlsStatus, Error> {
        if self.target.supports(Op::ReadTls) != Support::Yes {
            return Err(Error::Unsupported {
                target: self.target.name().to_string(),
                op: Op::ReadTls,
            });
        }
        match self.target.locate(app_id)? {
            Presence::Ours => self.target.tls_status(app_id),
            Presence::NotOurs => Err(Error::NotOurs {
                app: app_id.to_string(),
            }),
            Presence::Absent => Err(Error::Target("the app is not deployed".to_string())),
        }
    }

    /// Whether this target can put an app back after an apply.
    pub fn can_roll_back(&self) -> bool {
        self.target.supports(Op::Restore) == Support::Yes
    }

    /// Apply `plan` only if its [`Plan::digest`] is the one that was approved; otherwise `Error::PlanChanged` and nothing is touched.
    pub fn apply_approved(
        &mut self,
        plan: &Plan,
        approved_digest: &str,
        secrets: &dyn Secrets,
    ) -> Result<Applied, Error> {
        if plan.digest() != approved_digest {
            return Err(Error::PlanChanged);
        }
        self.apply(plan, secrets)
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
        // Read-only, and before the first change, so a refusal here still changes nothing.
        let snapshot = if self.can_roll_back() {
            Some(self.target.snapshot(&plan.app_id)?)
        } else {
            None
        };
        let point = |attempted: usize| {
            snapshot.as_ref().map(|s| {
                Box::new(RollbackPoint {
                    app_id: plan.app_id.clone(),
                    snapshot: s.clone(),
                    attempted,
                })
            })
        };
        let mut outcomes = vec![StepOutcome::Skipped; plan.steps.len()];
        for (i, (step, value)) in plan.steps.iter().zip(&values).enumerate() {
            if let Err(cause) = self.target.apply_step(&plan.app_id, step, value.as_ref()) {
                outcomes[i] = StepOutcome::Failed;
                return Err(Error::ApplyFailed {
                    steps: outcomes,
                    cause: Box::new(cause),
                    rollback_point: point(i + 1),
                });
            }
            outcomes[i] = StepOutcome::Applied;
        }
        match self.verify(plan) {
            Ok(read_back) => Ok(Applied {
                steps_applied: plan.steps.len(),
                rollback_point: point(plan.steps.len()),
                read_back,
            }),
            Err(cause) => Err(Error::ApplyFailed {
                steps: outcomes,
                cause: Box::new(cause),
                rollback_point: point(plan.steps.len()),
            }),
        }
    }

    /// Put the app back to where it stood before the apply that produced `point`, then read the target back.
    ///
    /// The answer is never a bare `Ok`: `Restored` only when the target's own state matches the snapshot; otherwise `Unknown`. Code, settings and
    /// secret names are restored; a secret overwritten by the apply cannot be read back, so its old value is not restored and shows as a difference.
    pub fn rollback(&mut self, point: &RollbackPoint) -> Rollback {
        if point.attempted == 0 {
            return Rollback::Unchanged;
        }
        if !self.can_roll_back() {
            return Rollback::unknown("this target cannot restore");
        }
        match self.target.locate(&point.app_id) {
            Ok(Presence::NotOurs) => {
                return Rollback::unknown("the target now holds something that is not this app");
            }
            Ok(_) => {}
            Err(e) => return Rollback::unknown(format!("could not reach the target: {e}")),
        }
        if let Err(e) = self.target.restore(&point.app_id, &point.snapshot) {
            return Rollback::unknown(format!("restore failed: {e}"));
        }
        self.check_restored(point)
    }

    fn check_restored(&self, point: &RollbackPoint) -> Rollback {
        let presence = match self.target.locate(&point.app_id) {
            Ok(p) => p,
            Err(e) => return Rollback::unknown(format!("could not read the target back: {e}")),
        };
        let want = &point.snapshot;
        let mut bad = Vec::new();
        match (want.existed, presence) {
            (false, Presence::Absent) => {}
            (false, _) => bad.push("app is still present".to_string()),
            (true, Presence::Ours) => {
                let now = match self.target.observe(&point.app_id) {
                    Ok(o) => o,
                    Err(e) => {
                        return Rollback::unknown(format!("could not read the target back: {e}"));
                    }
                };
                for name in want.observed.settings.keys().chain(now.settings.keys()) {
                    if want.observed.settings.get(name) != now.settings.get(name) {
                        bad.push(format!("setting {name}"));
                    }
                }
                for name in want
                    .observed
                    .secret_names
                    .symmetric_difference(&now.secret_names)
                {
                    bad.push(format!("secret {name}"));
                }
                bad.sort();
                bad.dedup();
            }
            (true, _) => bad.push("app is not present".to_string()),
        }
        if bad.is_empty() {
            Rollback::Restored
        } else {
            Rollback::Unknown {
                reason: "the target does not match the snapshot after restoring".to_string(),
                details: bad,
            }
        }
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
                Step::BindDomain { name } => {
                    if observed.domains.contains(name) {
                        read_back.domains.push(name.clone());
                    } else {
                        bad.push(format!("domain {name}"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeTarget;
    use crate::{Config, MapSecrets};

    fn app() -> AppManifest {
        AppManifest::new("digest-app")
            .setting("BASE_URL")
            .secret("API_TOKEN")
    }

    fn config(url: &str) -> Config {
        Config::from([("BASE_URL".to_string(), url.to_string())])
    }

    #[test]
    fn the_same_plan_has_the_same_digest_and_a_changed_plan_does_not() {
        let host = Host::new(FakeTarget::new());
        let a = host.plan(&app(), &config("https://a.example.org")).unwrap();
        let again = host.plan(&app(), &config("https://a.example.org")).unwrap();
        let b = host.plan(&app(), &config("https://b.example.org")).unwrap();
        assert_eq!(a.digest(), again.digest());
        assert_ne!(a.digest(), b.digest());
        assert_eq!(a.digest().len(), 64);
    }

    #[test]
    fn a_tampered_plan_is_refused_and_the_target_is_untouched() {
        let mut host = Host::new(FakeTarget::new());
        let plan = host.plan(&app(), &config("https://a.example.org")).unwrap();
        let approved = plan.digest();
        let mut tampered = plan.clone();
        tampered.steps.push(Step::SetSetting {
            name: "EVIL".into(),
            value: "1".into(),
        });
        let secrets = MapSecrets::new().with("API_TOKEN", "t");
        assert_eq!(
            host.apply_approved(&tampered, &approved, &secrets),
            Err(Error::PlanChanged)
        );
        assert_eq!(host.target.locate("digest-app").unwrap(), Presence::Absent);
    }

    #[test]
    fn the_approved_plan_applies() {
        let mut host = Host::new(FakeTarget::new());
        let plan = host.plan(&app(), &config("https://a.example.org")).unwrap();
        let secrets = MapSecrets::new().with("API_TOKEN", "t");
        let applied = host
            .apply_approved(&plan, &plan.digest(), &secrets)
            .unwrap();
        assert!(applied.read_back.healthy);
    }

    fn data_plan(policy: Option<Migrations>, release: Release) -> Plan {
        let mut a = AppManifest::new("digest-app").setting("BASE_URL");
        if let Some(p) = policy {
            a = a.migrations(p);
        }
        Host::new(FakeTarget::new())
            .plan_release(&a, &config("https://a.example.org"), release)
            .unwrap()
    }

    #[test]
    fn each_policy_and_release_gives_its_one_statement() {
        use Migrations::*;
        use Release::*;
        let cases = [
            (None, MigratesSchema, DataAfterRollback::Unknown),
            (None, NoSchemaChange, DataAfterRollback::Unknown),
            (Some(NoData), MigratesSchema, DataAfterRollback::NoData),
            (Some(Reversible), Unknown, DataAfterRollback::Unknown),
            (Some(Reversible), MigratesSchema, DataAfterRollback::Follows),
            (Some(Reversible), NoSchemaChange, DataAfterRollback::Follows),
            (
                Some(ForwardOnlyOlderTolerates),
                MigratesSchema,
                DataAfterRollback::Follows,
            ),
            (
                Some(ForwardOnlyOlderRefuses),
                NoSchemaChange,
                DataAfterRollback::Follows,
            ),
            (
                Some(ForwardOnlyOlderRefuses),
                Unknown,
                DataAfterRollback::Unknown,
            ),
            (
                Some(ForwardOnlyOlderRefuses),
                MigratesSchema,
                DataAfterRollback::NeedsBackupRestore,
            ),
        ];
        for (policy, release, want) in cases {
            assert_eq!(
                data_plan(policy, release).data_after_rollback(),
                want,
                "{policy:?} + {release:?}"
            );
        }
    }

    #[test]
    fn plain_plan_says_not_known_and_prints_a_data_line_first() {
        let host = Host::new(FakeTarget::new());
        let plan = host.plan(&app(), &config("https://a.example.org")).unwrap();
        assert_eq!(plan.data_after_rollback(), DataAfterRollback::Unknown);
        assert!(plan.to_string().starts_with("DATA: "));
    }

    #[test]
    fn a_refusing_migration_never_prints_follows() {
        let plan = data_plan(
            Some(Migrations::ForwardOnlyOlderRefuses),
            Release::MigratesSchema,
        );
        let text = plan.to_string();
        assert!(text.contains("REFUSE"), "{text}");
        assert!(!text.contains("will run on the data"), "{text}");
    }

    #[test]
    fn an_approved_plan_with_the_warning_edited_away_is_refused() {
        let mut host = Host::new(FakeTarget::new());
        let plan = data_plan(
            Some(Migrations::ForwardOnlyOlderRefuses),
            Release::MigratesSchema,
        );
        let approved = plan.digest();
        let mut edited = plan.clone();
        edited.data = DataAfterRollback::Follows;
        assert_ne!(edited.digest(), approved);
        let secrets = MapSecrets::new().with("API_TOKEN", "t");
        assert_eq!(
            host.apply_approved(&edited, &approved, &secrets),
            Err(Error::PlanChanged)
        );
    }
}
