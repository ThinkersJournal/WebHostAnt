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
//! });
//! assert!(failures.is_empty(), "{failures:?}");
//! ```
//!
//! Checks here: plan is inert, apply is proven by read-back, apply is idempotent, no secret leaks, a refusal changes nothing, a target
//! that is not ours is never overwritten, an unsupported operation is an error, a host that cannot be reached is never reported as "nothing
//! installed". Not yet covered (later slices): rollback.

use crate::{
    AppManifest, Config, Error, Host, MapSecrets, Presence, Secrets, Step, Support, Target,
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
    fn a_failure_never_contains_the_canary() {
        for fault in [
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
