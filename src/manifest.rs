//! What a consumer declares about its app (see `docs/CONSUMER-CONTRACT.md`).

use std::collections::BTreeMap;
use std::fmt;

/// The app, as the consumer describes it. Names only for secrets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppManifest {
    pub(crate) id: String,
    pub(crate) health_path: String,
    pub(crate) settings: Vec<String>,
    pub(crate) secrets: Vec<String>,
}

impl AppManifest {
    /// Start a manifest for the app with this fixed id.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            health_path: "/health".to_string(),
            settings: Vec::new(),
            secrets: Vec::new(),
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

    /// The app id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The health path.
    pub fn health_path(&self) -> &str {
        &self.health_path
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
