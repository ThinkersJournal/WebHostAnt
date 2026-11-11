//! Secrets from SecretAnt: a [`Secrets`] source over a getter that returns SecretAnt's `SecretValue` (ROADMAP row C7a).
//!
//! SecretAnt 0.0.2 ships only the value type (bytes, wiped on drop, no `Display`, no `Clone`); the store comes later (C7b). So this takes a
//! getter, a closure from a secret's name to `Option<secretant::SecretValue>`, and works with whatever SecretAnt gives later.
//!
//! * A value that is not valid UTF-8 is unusable here (our `SecretValue` is text) and gives no value, never a lossy copy.
//! * An empty value gives no value (an empty secret is never sent), as [`crate::EnvSecrets`] does.
//! * The bytes are copied once into our text value; SecretAnt's own value is dropped (and wiped) right after.
//! * `Debug` shows nothing of any value.

use std::fmt;

use crate::{SecretValue, Secrets};

type Getter = Box<dyn Fn(&str) -> Option<secretant::SecretValue>>;

/// A [`Secrets`] source that asks a SecretAnt getter for each name.
pub struct SecretAntSecrets {
    get: Getter,
}

impl SecretAntSecrets {
    /// Ask `get` for each secret by name.
    pub fn new(get: impl Fn(&str) -> Option<secretant::SecretValue> + 'static) -> Self {
        Self { get: Box::new(get) }
    }
}

impl Secrets for SecretAntSecrets {
    fn get(&self, name: &str) -> Option<SecretValue> {
        let found = (self.get)(name)?;
        if found.is_empty() {
            return None;
        }
        let text = std::str::from_utf8(found.expose()).ok()?;
        Some(SecretValue::new(text))
    }
}

impl fmt::Debug for SecretAntSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretAntSecrets").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> SecretAntSecrets {
        SecretAntSecrets::new(|name| match name {
            "API_TOKEN" => Some(secretant::SecretValue::from_string("CANARY-sa-1".into())),
            "BINARY" => Some(secretant::SecretValue::new(vec![0xff, 0xfe, 0x41])),
            "EMPTY" => Some(secretant::SecretValue::new(Vec::new())),
            _ => None,
        })
    }

    #[test]
    fn a_named_secret_is_read_through_the_getter() {
        let s = source();
        assert_eq!(s.get("API_TOKEN").unwrap().expose(), "CANARY-sa-1");
        // The getter is asked for the name given: another name is a different answer.
        assert!(s.get("OTHER").is_none());
    }

    #[test]
    fn a_value_that_is_not_text_gives_no_value() {
        // 0xff 0xfe 0x41 is not UTF-8; a lossy copy would send "\u{fffd}\u{fffd}A".
        assert!(source().get("BINARY").is_none());
    }

    #[test]
    fn an_empty_value_gives_no_value() {
        assert!(source().get("EMPTY").is_none());
    }

    #[test]
    fn debug_shows_no_value() {
        let shown = format!("{:?}", source());
        assert!(!shown.contains("CANARY"));
        assert!(shown.contains("SecretAntSecrets"));
    }

    #[test]
    fn it_feeds_a_plan_like_any_other_source() {
        use crate::{AppManifest, Host, fake::FakeTarget};
        let manifest = AppManifest::new("app").secret("API_TOKEN");
        let mut host = Host::new(FakeTarget::new());
        let plan = host.plan(&manifest, &crate::Config::new()).unwrap();
        let done = host
            .apply_approved(&plan, &plan.digest(), &source())
            .unwrap();
        assert!(done.steps_applied >= 1);
    }
}
