//! The conformance suite: run it against any [`Target`] to find out whether it keeps the contract.
//!
//! ```
//! use webhostant::conformance::{self, Scenario};
//! use webhostant::fake::FakeTarget;
//!
//! let failures = conformance::run(|scenario| match scenario {
//!     Scenario::Empty => FakeTarget::new(),
//!     Scenario::HoldsOtherApp => FakeTarget::new().holding_other_app(),
//!     Scenario::SecretsUnsupported => FakeTarget::new().with_secrets_support(webhostant::Support::No),
//!     Scenario::Unreachable => FakeTarget::new().unreachable(),
//!     Scenario::FailsLastStepOnce => FakeTarget::new().failing_step_once(2),
//!     Scenario::DropsAfterApply => FakeTarget::new().going_down_after(3),
//! });
//! assert!(failures.is_empty(), "{failures:?}");
//! ```
//!
//! Checks here: plan is inert, apply is proven by read-back, apply is idempotent, no secret leaks, a refusal changes nothing, a target
//! that is not ours is never overwritten, an unsupported operation is an error, a host that cannot be reached is never reported as "nothing
//! installed", a failed apply says which steps ran and a re-run finishes it, a target that can restore puts the app back and proves it by
//! read-back (and says `Unknown`, never `Restored`, when it cannot tell), and a target that cannot restore says so before apply, and domain/certificate status is read from the target, never made up, and a declared domain is bound, proven by read-back and not bound twice.

use crate::{
    AppManifest, Config, Error, Host, MapSecrets, Presence, Rollback, Secrets, Step, StepOutcome,
    Support, Target,
};

const CANARY: &str = "CANARY-7f3a-not-a-real-secret";

/// The starting state a target must be built in for a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Nothing deployed, every operation supported.
    Empty,
    /// Something else lives under every app id.
    HoldsOtherApp,
    /// Nothing deployed; `SetSecret` is not `Yes`.
    SecretsUnsupported,
    /// Every call to the host fails, as if the network were down or the credential refused.
    Unreachable,
    /// Nothing deployed. Of the three steps of the suite's plan (create, one setting, one secret), the third fails once; a second try works.
    FailsLastStepOnce,
    /// Nothing deployed. Every call to the host fails once all three steps of the suite's plan have been applied.
    DropsAfterApply,
}

/// One failed check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// Which check.
    pub check: &'static str,
    /// What went wrong. Never contains the canary.
    pub detail: String,
}

fn manifest() -> AppManifest {
    AppManifest::new("conformance-app")
        .setting("BASE_URL")
        .secret("API_TOKEN")
}

fn config() -> Config {
    Config::from([(
        "BASE_URL".to_string(),
        "https://example.invalid".to_string(),
    )])
}

fn secrets() -> MapSecrets {
    MapSecrets::new().with("API_TOKEN", CANARY)
}

fn fail(out: &mut Vec<Failure>, check: &'static str, detail: impl Into<String>) {
    out.push(Failure {
        check,
        detail: detail.into(),
    });
}

/// Run every check. `make` builds a fresh target for a scenario. An empty result means the target keeps the contract.
pub fn run<T: Target>(make: impl Fn(Scenario) -> T) -> Vec<Failure> {
    let mut out = Vec::new();
    plan_is_inert(&make, &mut out);
    apply_is_proven_and_idempotent(&make, &mut out);
    no_secret_leaks(&make, &mut out);
    missing_secret_acts_nothing(&make, &mut out);
    not_ours_is_refused(&make, &mut out);
    unsupported_is_an_error(&make, &mut out);
    unreachable_is_never_absent(&make, &mut out);
    undo_is_stated_up_front(&make, &mut out);
    failed_apply_is_visible_and_resumable(&make, &mut out);
    rollback_restores_the_snapshot(&make, &mut out);
    failed_apply_can_be_rolled_back(&make, &mut out);
    rollback_never_claims_more_than_it_knows(&make, &mut out);
    tls_status_is_read_never_made_up(&make, &mut out);
    domains_are_bound_and_read_back(&make, &mut out);
    out
}

fn plan_is_inert<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "plan is inert";
    let host = Host::new(make(Scenario::Empty));
    let before = (
        host.target().locate("conformance-app"),
        host.target().observe("conformance-app"),
        host.target().diagnostics(),
    );
    match host.plan(&manifest(), &config()) {
        Ok(plan) if plan.is_empty() => fail(out, C, "an empty target produced an empty plan"),
        Ok(_) => {}
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    }
    let after = (
        host.target().locate("conformance-app"),
        host.target().observe("conformance-app"),
        host.target().diagnostics(),
    );
    if before != after {
        fail(out, C, "the target's state changed during plan");
    }
    if before.0 != Ok(Presence::Absent) {
        fail(out, C, "the Empty scenario did not start Absent");
    }
}

fn apply_is_proven_and_idempotent<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "apply is proven by read-back, and idempotent";
    let mut host = Host::new(make(Scenario::Empty));
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    if let Err(e) = host.apply(&plan, &secrets()) {
        return fail(out, C, format!("apply failed: {e}"));
    }
    // Independent of the host's own verification: ask the target directly.
    match host.target().observe("conformance-app") {
        Ok(o) => {
            if o.settings.get("BASE_URL").map(String::as_str) != Some("https://example.invalid")
                || !o.secret_names.contains("API_TOKEN")
                || !o.healthy
            {
                fail(
                    out,
                    C,
                    "apply returned Ok but the target does not show the result",
                );
            }
        }
        Err(e) => return fail(out, C, format!("observe failed: {e}")),
    }
    let settled = host.target().observe("conformance-app");
    match host.plan(&manifest(), &config()) {
        Ok(p) if !p.is_empty() => fail(
            out,
            C,
            "a second plan still has steps after a successful apply",
        ),
        Ok(_) => {}
        Err(e) => return fail(out, C, format!("second plan failed: {e}")),
    }
    if let Err(e) = host.apply(&plan, &secrets()) {
        return fail(out, C, format!("re-applying the same plan failed: {e}"));
    }
    if host.target().observe("conformance-app") != settled {
        fail(out, C, "re-applying the same plan changed the target");
    }
}

fn no_secret_leaks<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "no secret leaks";
    let mut host = Host::new(make(Scenario::Empty));
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let applied = host.apply(&plan, &secrets());
    let surfaces = [
        ("plan display", plan.to_string()),
        ("plan debug", format!("{plan:?}")),
        ("apply result", format!("{applied:?}")),
        ("target diagnostics", host.target().diagnostics()),
        (
            "observed state",
            format!("{:?}", host.target().observe("conformance-app")),
        ),
        ("secret debug", format!("{:?}", secrets().get("API_TOKEN"))),
    ];
    for (name, text) in surfaces {
        if text.contains(CANARY) {
            fail(out, C, format!("the secret value appears in: {name}"));
        }
    }
}

fn missing_secret_acts_nothing<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "a missing secret changes nothing";
    let mut host = Host::new(make(Scenario::Empty));
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let before = (
        host.target().locate("conformance-app"),
        host.target().diagnostics(),
    );
    match host.apply(&plan, &MapSecrets::new()) {
        Err(Error::MissingSecret { name }) if name == "API_TOKEN" => {}
        other => fail(out, C, format!("expected MissingSecret, got {other:?}")),
    }
    let after = (
        host.target().locate("conformance-app"),
        host.target().diagnostics(),
    );
    if before != after {
        fail(out, C, "the target changed although the apply was refused");
    }
}

fn not_ours_is_refused<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "a target that is not ours is never overwritten";
    let mut host = Host::new(make(Scenario::HoldsOtherApp));
    match host.plan(&manifest(), &config()) {
        Err(Error::NotOurs { .. }) => {}
        other => fail(out, C, format!("plan: expected NotOurs, got {other:?}")),
    }
    let by_hand = crate::Plan::new("conformance-app", vec![Step::Create]);
    let before = host.target().diagnostics();
    match host.apply(&by_hand, &secrets()) {
        Err(Error::NotOurs { .. }) => {}
        other => fail(out, C, format!("apply: expected NotOurs, got {other:?}")),
    }
    if host.target().diagnostics() != before {
        fail(out, C, "the target changed although the apply was refused");
    }
}

fn unsupported_is_an_error<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "an unsupported operation is an error";
    let mut host = Host::new(make(Scenario::SecretsUnsupported));
    if host.target().supports(crate::Op::SetSecret) == Support::Yes {
        return fail(out, C, "the SecretsUnsupported scenario still says Yes");
    }
    match host.plan(&manifest(), &config()) {
        Err(Error::Unsupported {
            op: crate::Op::SetSecret,
            ..
        }) => {}
        other => fail(out, C, format!("expected Unsupported, got {other:?}")),
    }
    let by_hand = crate::Plan::new(
        "conformance-app",
        vec![Step::SetSecret {
            name: "API_TOKEN".to_string(),
        }],
    );
    let before = host.target().diagnostics();
    match host.apply(&by_hand, &secrets()) {
        Err(Error::Unsupported { .. }) => {}
        other => fail(
            out,
            C,
            format!("apply: expected Unsupported, got {other:?}"),
        ),
    }
    if host.target().diagnostics() != before {
        fail(out, C, "the target changed although the apply was refused");
    }
}

fn unreachable_is_never_absent<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "an unreachable host is never reported as not installed";
    let mut host = Host::new(make(Scenario::Unreachable));
    match host.target().locate("conformance-app") {
        Err(_) => {}
        other => fail(out, C, format!("locate: expected an error, got {other:?}")),
    }
    match host.plan(&manifest(), &config()) {
        Err(_) => {}
        Ok(plan) => fail(
            out,
            C,
            format!(
                "plan succeeded ({} steps) without reaching the host",
                plan.steps().len()
            ),
        ),
    }
    let by_hand = crate::Plan::new("conformance-app", vec![Step::Create]);
    if host.apply(&by_hand, &secrets()).is_ok() {
        fail(out, C, "apply reported success without reaching the host");
    }
}

fn undo_is_stated_up_front<T: Target>(make: &impl Fn(Scenario) -> T, out: &mut Vec<Failure>) {
    const C: &str = "whether an apply can be undone is stated before it runs";
    let mut host = Host::new(make(Scenario::Empty));
    let can = host.target().supports(crate::Op::Restore) == Support::Yes;
    if host.can_roll_back() != can {
        fail(out, C, "Host::can_roll_back disagrees with the target");
    }
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    if plan.undoable() != can {
        fail(out, C, "the plan's undoable flag disagrees with the target");
    }
    if !can && !plan.to_string().contains("cannot undo") {
        fail(out, C, "the plan does not say that it cannot be undone");
    }
    match host.apply(&plan, &secrets()) {
        Ok(applied) if applied.rollback_point.is_some() != can => fail(
            out,
            C,
            "the rollback point is present only when the target can restore",
        ),
        Ok(_) => {}
        Err(e) => fail(out, C, format!("apply failed: {e}")),
    }
}

fn failed_apply_is_visible_and_resumable<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "a failed apply says which steps ran, and a re-run finishes it";
    let mut host = Host::new(make(Scenario::FailsLastStepOnce));
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let total = plan.steps().len();
    match host.apply(&plan, &secrets()) {
        Err(Error::ApplyFailed { steps, .. }) => {
            let mut expected = vec![StepOutcome::Applied; total];
            expected[total - 1] = StepOutcome::Failed;
            if steps != expected {
                fail(out, C, format!("expected {expected:?}, got {steps:?}"));
            }
        }
        other => return fail(out, C, format!("expected ApplyFailed, got {other:?}")),
    }
    match host.plan(&manifest(), &config()) {
        Ok(again) if again.steps().len() >= total => fail(
            out,
            C,
            "a re-plan after a partial apply repeats finished steps",
        ),
        Ok(again) => {
            if let Err(e) = host.apply(&again, &secrets()) {
                fail(out, C, format!("the re-run did not finish: {e}"));
            }
        }
        Err(e) => fail(out, C, format!("re-planning failed: {e}")),
    }
}

fn gone<T: Target>(host: &Host<T>) -> bool {
    host.target().locate("conformance-app") == Ok(Presence::Absent)
}

fn rollback_restores_the_snapshot<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "rollback puts the app back, proven by read-back";
    let mut host = Host::new(make(Scenario::Empty));
    if !host.can_roll_back() {
        return;
    }
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let Some(point) = host
        .apply(&plan, &secrets())
        .ok()
        .and_then(|a| a.rollback_point)
    else {
        return fail(out, C, "apply gave no rollback point");
    };
    match host.rollback(&point) {
        Rollback::Restored => {
            // Independent of the host's own verification.
            if !gone(&host) {
                fail(out, C, "reported Restored but the app is still there");
            }
        }
        other => fail(out, C, format!("expected Restored, got {other:?}")),
    }
}

fn failed_apply_can_be_rolled_back<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "a half-finished apply can be rolled back";
    let mut host = Host::new(make(Scenario::FailsLastStepOnce));
    if !host.can_roll_back() {
        return;
    }
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let Err(Error::ApplyFailed {
        rollback_point: Some(point),
        ..
    }) = host.apply(&plan, &secrets())
    else {
        return fail(out, C, "the failed apply gave no rollback point");
    };
    match host.rollback(&point) {
        Rollback::Restored if gone(&host) => {}
        other => fail(
            out,
            C,
            format!("expected Restored with the app gone, got {other:?}"),
        ),
    }
}

fn rollback_never_claims_more_than_it_knows<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "a rollback on an unreachable host is Unknown, never Restored";
    let mut host = Host::new(make(Scenario::DropsAfterApply));
    if !host.can_roll_back() {
        return;
    }
    let plan = match host.plan(&manifest(), &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    let Err(Error::ApplyFailed {
        rollback_point: Some(point),
        ..
    }) = host.apply(&plan, &secrets())
    else {
        return fail(
            out,
            C,
            "an apply that lost the host did not fail with a rollback point",
        );
    };
    match host.rollback(&point) {
        Rollback::Unknown { .. } => {}
        other => fail(out, C, format!("expected Unknown, got {other:?}")),
    }
}

fn tls_status_is_read_never_made_up<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "domain and certificate status is read from the target, never made up";
    let mut host = Host::new(make(Scenario::Empty));
    if host.target().supports(crate::Op::ReadTls) != Support::Yes {
        match host.tls_status("conformance-app") {
            Err(Error::Unsupported { .. }) => {}
            other => fail(
                out,
                C,
                format!("unsupported: expected Unsupported, got {other:?}"),
            ),
        }
        return;
    }
    if let Ok(s) = host.tls_status("conformance-app") {
        fail(
            out,
            C,
            format!("an app that is not deployed returned {s:?}"),
        );
    }
    match host.plan(&manifest(), &config()) {
        Ok(plan) => {
            if let Err(e) = host.apply(&plan, &secrets()) {
                return fail(out, C, format!("apply failed: {e}"));
            }
        }
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    }
    if let Err(e) = host.tls_status("conformance-app") {
        fail(out, C, format!("a deployed app has no status: {e}"));
    }
    let other = Host::new(make(Scenario::HoldsOtherApp));
    match other.tls_status("conformance-app") {
        Err(Error::NotOurs { .. }) => {}
        got => fail(out, C, format!("not ours: expected NotOurs, got {got:?}")),
    }
    let down = Host::new(make(Scenario::Unreachable));
    if let Ok(s) = down.tls_status("conformance-app") {
        fail(out, C, format!("an unreachable host returned {s:?}"));
    }
    // The adapter itself, without the host's guard: it must not answer for a host it cannot reach.
    if let Ok(s) = down.target().tls_status("conformance-app") {
        fail(
            out,
            C,
            format!("the adapter answered for an unreachable host: {s:?}"),
        );
    }
}

fn domains_are_bound_and_read_back<T: Target>(
    make: &impl Fn(Scenario) -> T,
    out: &mut Vec<Failure>,
) {
    const C: &str = "a declared domain is bound, proven by read-back, and not bound twice";
    let app = manifest().domain("app.example.org");
    let mut host = Host::new(make(Scenario::Empty));
    if host.target().supports(crate::Op::BindDomain) != Support::Yes {
        match host.plan(&app, &config()) {
            Err(Error::Unsupported { .. }) => {}
            other => fail(
                out,
                C,
                format!(
                    "unsupported: expected Unsupported, got {:?}",
                    other.map(|p| p.steps().len())
                ),
            ),
        }
        return;
    }
    match host.plan(&manifest().domain("https://app.example.org/x"), &config()) {
        Err(Error::InvalidDomain { .. }) => {}
        other => fail(
            out,
            C,
            format!(
                "an invalid name was not refused: {:?}",
                other.map(|p| p.steps().len())
            ),
        ),
    }
    let plan = match host.plan(&app, &config()) {
        Ok(p) => p,
        Err(e) => return fail(out, C, format!("planning failed: {e}")),
    };
    if !plan
        .steps()
        .iter()
        .any(|s| matches!(s, Step::BindDomain { .. }))
    {
        return fail(out, C, "the plan has no bind step for a declared domain");
    }
    match host.apply(&plan, &secrets()) {
        Ok(a) if a.read_back.domains == ["app.example.org"] => {}
        Ok(a) => fail(
            out,
            C,
            format!(
                "read-back did not name the domain: {:?}",
                a.read_back.domains
            ),
        ),
        Err(e) => return fail(out, C, format!("apply failed: {e}")),
    }
    match host.target().observe("conformance-app") {
        Ok(o) if o.domains.contains("app.example.org") => {}
        other => fail(
            out,
            C,
            format!(
                "the target does not show the domain bound: {:?}",
                other.map(|o| o.domains)
            ),
        ),
    }
    match host.plan(&app, &config()) {
        Ok(p) if p.is_empty() => {}
        other => fail(
            out,
            C,
            format!(
                "a second plan is not empty: {:?}",
                other.map(|p| p.steps().len())
            ),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{FakeTarget, Fault};

    fn build(fault: Fault) -> impl Fn(Scenario) -> FakeTarget {
        move |scenario| {
            let t = FakeTarget::new().with_fault(fault);
            match scenario {
                Scenario::Empty => t,
                Scenario::HoldsOtherApp => t.holding_other_app(),
                Scenario::SecretsUnsupported => t.with_secrets_support(Support::No),
                Scenario::Unreachable => t.unreachable(),
                Scenario::FailsLastStepOnce => t.failing_step_once(2),
                Scenario::DropsAfterApply => t.going_down_after(3),
            }
        }
    }

    fn checks(fault: Fault) -> Vec<&'static str> {
        run(build(fault)).into_iter().map(|f| f.check).collect()
    }

    #[test]
    fn a_correct_target_passes_every_check() {
        assert_eq!(run(build(Fault::None)), vec![]);
    }

    #[test]
    fn a_silent_noop_is_caught() {
        assert!(
            checks(Fault::SilentNoop).contains(&"apply is proven by read-back, and idempotent")
        );
    }

    #[test]
    fn a_secret_in_the_log_is_caught() {
        assert!(checks(Fault::LeaksSecret).contains(&"no secret leaks"));
    }

    #[test]
    fn acting_during_plan_is_caught() {
        assert!(checks(Fault::ActsDuringPlan).contains(&"plan is inert"));
    }

    #[test]
    fn unreachable_reported_as_absent_is_caught() {
        assert!(
            checks(Fault::UnreachableIsAbsent)
                .contains(&"an unreachable host is never reported as not installed")
        );
    }

    #[test]
    fn an_empty_tls_status_instead_of_an_error_is_caught() {
        assert!(
            checks(Fault::TlsEmptyInsteadOfError)
                .contains(&"domain and certificate status is read from the target, never made up")
        );
    }

    #[test]
    fn a_bind_that_binds_nothing_is_caught() {
        assert!(
            checks(Fault::DomainNotBound)
                .contains(&"a declared domain is bound, proven by read-back, and not bound twice")
        );
    }

    #[test]
    fn a_restore_that_does_nothing_is_caught() {
        assert!(
            checks(Fault::RestoreDoesNothing)
                .contains(&"rollback puts the app back, proven by read-back")
        );
    }

    #[test]
    fn a_restore_that_does_nothing_reads_back_as_unknown() {
        let mut host = Host::new(FakeTarget::new().with_fault(Fault::RestoreDoesNothing));
        let plan = host.plan(&manifest(), &config()).unwrap();
        let point = host
            .apply(&plan, &secrets())
            .unwrap()
            .rollback_point
            .unwrap();
        match host.rollback(&point) {
            Rollback::Unknown { details, .. } => assert!(!details.is_empty()),
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[test]
    fn nothing_applied_means_nothing_to_undo() {
        let mut host = Host::new(FakeTarget::new());
        let plan = crate::Plan::new("conformance-app", vec![]);
        let point = host
            .apply(&plan, &secrets())
            .err()
            .and_then(|e| match e {
                Error::ApplyFailed { rollback_point, .. } => rollback_point,
                _ => None,
            })
            .expect("an empty plan on an empty target fails read-back with a point");
        assert_eq!(host.rollback(&point), Rollback::Unchanged);
    }

    #[test]
    fn a_failure_never_contains_the_canary() {
        for fault in [
            Fault::TlsEmptyInsteadOfError,
            Fault::RestoreDoesNothing,
            Fault::SilentNoop,
            Fault::LeaksSecret,
            Fault::ActsDuringPlan,
            Fault::UnreachableIsAbsent,
        ] {
            for f in run(build(fault)) {
                assert!(!f.detail.contains(CANARY), "{f:?}");
            }
        }
    }
}
