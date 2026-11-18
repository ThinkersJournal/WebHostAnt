// SPDX-License-Identifier: MIT OR Apache-2.0
//! Secrets from SecretAnt: a [`Secrets`] source over a getter that returns SecretAnt's `SecretValue` (ROADMAP row C7a).
//!
//! Two sources. [`SecretAntSecrets`] takes a getter, a closure from a secret's name to `Option<secretant::SecretValue>` (C7a). [`SecretAntStore`]
//! reads SecretAnt's own encrypted store through its read API (`SecretReader`, C7b, SecretAnt 0.1.5): a secret by name, and the list of names.
//! SecretAnt 0.1.5 has no audit, so neither source offers one.
//!
//! * A value that is not valid UTF-8 is unusable here (our `SecretValue` is text) and gives no value, never a lossy copy.
//! * An empty value gives no value (an empty secret is never sent), as [`crate::EnvSecrets`] does.
//! * The bytes are copied once into our text value; SecretAnt's own value is dropped (and wiped) right after.
//! * `Debug` shows nothing of any value.

use std::fmt;

use crate::{SecretValue, Secrets};

/// Our text value from SecretAnt's bytes: none when empty (an empty secret is never sent) or not UTF-8 (never a lossy copy).
fn text_of(found: &secretant::SecretValue) -> Option<SecretValue> {
    if found.is_empty() {
        return None;
    }
    let text = std::str::from_utf8(found.expose()).ok()?;
    Some(SecretValue::new(text))
}

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
        text_of(&(self.get)(name)?)
    }
}

impl fmt::Debug for SecretAntSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretAntSecrets").finish_non_exhaustive()
    }
}

/// A [`Secrets`] source over SecretAnt's store file, read fresh on every call (SecretAnt keeps no cache, so a rotated value shows at once).
///
/// `open` checks that the store opens with the given unlock. `try_get` tells a missing secret (`Ok(None)`) from a store that cannot be read
/// (`Err`, one of [`secretant::ReadError`]'s classes: it is `#[non_exhaustive]`, so match it with a wildcard arm). [`Secrets::get`] cannot say
/// why, so it gives no value for both. Every call is bounded by the `ReadConfig` timeout. The same-user limit of SecretAnt applies: any process
/// running as the same user can read what this reads.
pub struct SecretAntStore {
    reader: secretant::SecretReader,
}

impl SecretAntStore {
    /// Open the store with `config`, within its timeout.
    pub fn open(config: secretant::ReadConfig) -> Result<Self, secretant::ReadError> {
        Ok(Self {
            reader: secretant::SecretReader::open(config)?,
        })
    }

    /// The value of `name`: `Ok(None)` when the store has no such entry, or its value is empty or not UTF-8; `Err` when the store cannot be read.
    pub fn try_get(&self, name: &str) -> Result<Option<SecretValue>, secretant::ReadError> {
        match self.reader.get(name) {
            Ok(found) => Ok(text_of(&found)),
            Err(secretant::ReadError::NotFound) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// Every name in the store, sorted. Names only: no value is read.
    pub fn names(&self) -> Result<Vec<String>, secretant::ReadError> {
        self.reader.list()
    }
}

impl Secrets for SecretAntStore {
    fn get(&self, name: &str) -> Option<SecretValue> {
        self.try_get(name).ok().flatten()
    }
}

impl fmt::Debug for SecretAntStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecretAntStore").finish_non_exhaustive()
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

    use secretant::{FileStore, KdfParams, ReadConfig, ReadError, ReadUnlock, Store};
    use std::{collections::BTreeMap, time::Duration};

    const PASS: &str = "CANARY-passphrase-1";

    /// A real store in a throwaway folder holding `API_TOKEN` and `OTHER`, with cheap key costs (tests only).
    fn store_in(dir: &tempfile::TempDir) -> (std::path::PathBuf, FileStore) {
        let path = dir.path().join("store.json");
        let cheap = KdfParams {
            m_kib: 64,
            t: 1,
            p: 1,
        };
        let pass = secretant::SecretValue::from_string(PASS.into());
        let mut store = FileStore::create_with_params(&path, &pass, cheap).unwrap();
        for (name, value) in [("API_TOKEN", "CANARY-store-1"), ("OTHER", "CANARY-store-2")] {
            let v = secretant::SecretValue::from_string(value.into());
            store.put(name, v, BTreeMap::new()).unwrap();
        }
        (path, store)
    }

    fn config(path: &std::path::Path, pass: &str) -> ReadConfig {
        ReadConfig {
            path: path.to_path_buf(),
            unlock: ReadUnlock::Passphrase(secretant::SecretValue::from_string(pass.into())),
            timeout: Duration::from_secs(30),
        }
    }

    #[test]
    fn a_secret_is_read_from_a_real_store_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        assert_eq!(s.get("API_TOKEN").unwrap().expose(), "CANARY-store-1");
        assert_eq!(s.get("OTHER").unwrap().expose(), "CANARY-store-2");
        // A name the store does not hold is no value, and the class says it was not found, not refused.
        assert!(s.get("MISSING").is_none());
        assert!(matches!(s.try_get("MISSING"), Ok(None)));
    }

    #[test]
    fn the_names_are_listed_sorted_and_never_a_value() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        let names = s.names().unwrap();
        assert_eq!(names, vec!["API_TOKEN".to_string(), "OTHER".to_string()]);
        assert!(!format!("{names:?}").contains("CANARY"));
    }

    #[test]
    fn a_wrong_passphrase_or_a_missing_store_is_an_error_class_not_a_missing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _store) = store_in(&dir);
        let denied = SecretAntStore::open(config(&path, "CANARY-wrong")).unwrap_err();
        assert_eq!(denied, ReadError::Denied);
        assert!(!format!("{denied}").contains("CANARY"));
        let gone = SecretAntStore::open(config(&dir.path().join("none.json"), PASS)).unwrap_err();
        assert_eq!(gone, ReadError::Unreachable);
    }

    #[test]
    fn a_store_that_goes_away_after_open_is_an_error_not_a_missing_secret() {
        let dir = tempfile::tempdir().unwrap();
        let (path, store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        drop(store);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(s.try_get("API_TOKEN").unwrap_err(), ReadError::Unreachable);
        assert_eq!(s.names().unwrap_err(), ReadError::Unreachable);
        // The `Secrets` view cannot say why, so it gives no value rather than a wrong one.
        assert!(s.get("API_TOKEN").is_none());
    }

    #[test]
    fn a_rotated_value_is_seen_on_the_next_read() {
        let dir = tempfile::tempdir().unwrap();
        let (path, mut store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        assert_eq!(s.get("API_TOKEN").unwrap().expose(), "CANARY-store-1");
        let v = secretant::SecretValue::from_string("CANARY-store-3".into());
        store.put("API_TOKEN", v, BTreeMap::new()).unwrap();
        assert_eq!(s.get("API_TOKEN").unwrap().expose(), "CANARY-store-3");
    }

    #[test]
    fn debug_of_a_store_source_shows_no_value_and_no_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let (path, _store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        let shown = format!("{s:?}");
        assert!(shown.contains("SecretAntStore"));
        assert!(!shown.contains("CANARY"));
    }

    #[test]
    fn a_real_store_feeds_a_plan() {
        use crate::{AppManifest, Host, fake::FakeTarget};
        let dir = tempfile::tempdir().unwrap();
        let (path, _store) = store_in(&dir);
        let s = SecretAntStore::open(config(&path, PASS)).unwrap();
        let manifest = AppManifest::new("app").secret("API_TOKEN");
        let mut host = Host::new(FakeTarget::new());
        let plan = host.plan(&manifest, &crate::Config::new()).unwrap();
        let done = host.apply_approved(&plan, &plan.digest(), &s).unwrap();
        assert!(done.steps_applied >= 1);
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
