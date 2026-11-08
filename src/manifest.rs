//! What a consumer declares about its app (see `docs/CONSUMER-CONTRACT.md`).

use std::collections::BTreeMap;
use std::fmt;

/// What a consumer says about its data when code is rolled back (contract section 1, row 6). WebHostAnt carries this; it never checks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Migrations {
    /// The app holds no data worth protecting.
    NoData,
    /// Every migration can be reversed.
    Reversible,
    /// Forward-only, additive-only: older code still runs on the new data.
    ForwardOnlyOlderTolerates,
    /// Forward-only, and older code refuses the new schema.
    ForwardOnlyOlderRefuses,
}

/// The app, as the consumer describes it. Names only for secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub(crate) id: String,
    pub(crate) health_path: String,
    pub(crate) settings: Vec<String>,
    pub(crate) secrets: Vec<String>,
    pub(crate) absent_secrets: Vec<String>,
    pub(crate) domains: Vec<String>,
    pub(crate) migrations: Option<Migrations>,
}

impl AppManifest {
    /// Start a manifest for the app with this fixed id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            health_path: "/health".to_string(),
            settings: Vec::new(),
            secrets: Vec::new(),
            absent_secrets: Vec::new(),
            domains: Vec::new(),
            migrations: None,
        }
    }

    /// The path that answers when the app is up.
    pub fn health(mut self, path: impl Into<String>) -> Self {
        self.health_path = path.into();
        self
    }

    /// Declare a plain (non-secret) setting the host must receive.
    pub fn setting(mut self, name: impl Into<String>) -> Self {
        self.settings.push(name.into());
        self
    }

    /// Declare a secret by name. Its value is supplied at apply time, never here.
    pub fn secret(mut self, name: impl Into<String>) -> Self {
        self.secrets.push(name.into());
        self
    }

    /// Declare a secret the app must NOT have, by name. A plan deletes it when the host holds it (a deploy keeps old secrets, so a secret the
    /// app no longer wants stays readable by the app until someone deletes it). A name also declared with [`AppManifest::secret`] is an error.
    pub fn without_secret(mut self, name: impl Into<String>) -> Self {
        self.absent_secrets.push(name.into());
        self
    }

    /// Declare a host name the app should be served at. The name is checked when a plan is made (`Error::InvalidDomain`).
    pub fn domain(mut self, name: impl Into<String>) -> Self {
        self.domains.push(name.into());
        self
    }

    /// Declare the data policy. Left undeclared, a plan says the answer is unknown, never that data follows.
    pub fn migrations(mut self, policy: Migrations) -> Self {
        self.migrations = Some(policy);
        self
    }

    /// The app id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The health path.
    pub fn health_path(&self) -> &str {
        &self.health_path
    }
}

/// Whether `name` is a plain host name: dot-separated labels of letters, digits and hyphens (no scheme, port, path, wildcard or IP literal).
pub(crate) fn valid_domain(name: &str) -> bool {
    let labels: Vec<&str> = name.split('.').collect();
    let label = |l: &&str| {
        !l.is_empty()
            && l.len() <= 63
            && !l.starts_with('-')
            && !l.ends_with('-')
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    name.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(label)
        // A last label of digits only is an IP literal, not a name.
        && !labels[labels.len() - 1].bytes().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::valid_domain;

    #[test]
    fn host_names_are_accepted_and_everything_else_is_not() {
        for ok in [
            "app.example.org",
            "a-b.c1.example.co.uk",
            "xn--bcher-kva.example",
        ] {
            assert!(valid_domain(ok), "{ok}");
        }
        for bad in [
            "",
            "localhost",
            "https://app.example.org",
            "app.example.org:8443",
            "app.example.org/x",
            "*.example.org",
            "192.168.0.1",
            "-a.example.org",
            "a-.example.org",
            "a..example.org",
            "app.example.org.",
            "ex ample.org",
            "under_score.example.org",
        ] {
            assert!(!valid_domain(bad), "{bad}");
        }
    }
}

/// Non-secret setting values, by name.
pub type Config = BTreeMap<String, String>;

/// A secret value. Its `Debug` shows nothing.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(String);

impl SecretValue {
    /// Wrap a value.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value. Call only at the point of delivery to the target.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretValue(<redacted>)")
    }
}

/// Where secret values come from at deploy time (SecretAnt later; a map or the host's own paste step now).
pub trait Secrets {
    /// The value for this name, if there is one.
    fn get(&self, name: &str) -> Option<SecretValue>;
}

/// A fixed set of secrets, for tests and for callers that already hold them.
#[derive(Debug, Default, Clone)]
pub struct MapSecrets(BTreeMap<String, SecretValue>);

impl MapSecrets {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add one.
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(name.into(), SecretValue::new(value));
        self
    }
}

impl Secrets for MapSecrets {
    fn get(&self, name: &str) -> Option<SecretValue> {
        self.0.get(name).cloned()
    }
}

type Lookup = Box<dyn Fn(&str) -> Option<String>>;

/// Secrets read from environment variables, as a tool such as `with-secret` exports them for one process. A secret name maps to the variable of
/// the same name unless [`EnvSecrets::var`] says otherwise. A variable that is unset, empty or not valid text gives no value (an empty secret is never
/// sent). Its `Debug` shows the names it knows, never a value.
pub struct EnvSecrets {
    vars: BTreeMap<String, String>,
    lookup: Lookup,
}

impl EnvSecrets {
    /// Read from this process's environment.
    pub fn new() -> Self {
        Self::with_lookup(|var| std::env::var(var).ok())
    }

    /// Read through `lookup` instead of the environment (for tests, or a caller that holds its own table).
    pub fn with_lookup(lookup: impl Fn(&str) -> Option<String> + 'static) -> Self {
        Self {
            vars: BTreeMap::new(),
            lookup: Box::new(lookup),
        }
    }

    /// Read the secret `name` from the variable `var` instead of a variable called `name`.
    pub fn var(mut self, name: impl Into<String>, var: impl Into<String>) -> Self {
        self.vars.insert(name.into(), var.into());
        self
    }
}

impl Default for EnvSecrets {
    fn default() -> Self {
        Self::new()
    }
}

impl Secrets for EnvSecrets {
    fn get(&self, name: &str) -> Option<SecretValue> {
        let var = self.vars.get(name).map_or(name, String::as_str);
        (self.lookup)(var)
            .filter(|v| !v.is_empty())
            .map(SecretValue::new)
    }
}

impl fmt::Debug for EnvSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvSecrets")
            .field("mapped", &self.vars.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod env_tests {
    use super::*;

    fn table() -> EnvSecrets {
        EnvSecrets::with_lookup(|var| match var {
            "API_TOKEN" => Some("CANARY-env-1".to_string()),
            "RENAMED_VAR" => Some("CANARY-env-2".to_string()),
            "EMPTY_ONE" => Some(String::new()),
            _ => None,
        })
    }

    #[test]
    fn a_secret_reads_the_variable_of_its_name_or_the_one_it_is_mapped_to() {
        let s = table().var("DB", "RENAMED_VAR");
        assert_eq!(s.get("API_TOKEN").unwrap().expose(), "CANARY-env-1");
        assert_eq!(s.get("DB").unwrap().expose(), "CANARY-env-2");
        // A mapped name does not also read the variable called by its own name.
        let s = table().var("API_TOKEN", "NOT_SET_ANYWHERE");
        assert!(s.get("API_TOKEN").is_none());
    }

    #[test]
    fn unset_and_empty_give_no_value() {
        let s = table();
        assert!(s.get("NOT_THERE").is_none());
        assert!(s.get("EMPTY_ONE").is_none());
    }

    #[test]
    fn debug_shows_no_value() {
        let s = table().var("DB", "RENAMED_VAR");
        let _ = s.get("API_TOKEN");
        assert!(!format!("{s:?}").contains("CANARY-env"));
        assert!(!format!("{:?}", s.get("DB")).contains("CANARY-env"));
    }

    #[test]
    fn the_real_environment_is_read_and_a_missing_variable_is_not_made_up() {
        // PATH is set in every environment these tests run in; the other name is not.
        assert!(EnvSecrets::new().get("PATH").is_some());
        assert!(
            EnvSecrets::new()
                .get("WEBHOSTANT_SURELY_NOT_SET_7F3A")
                .is_none()
        );
    }
}
