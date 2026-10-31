//! Settings from ConfigurAnt: render the values an app declared into WebHostAnt's [`Config`], and name where its secrets come from.
//!
//! This is the "projection" of `docs/DESIGN.md` section 6.3 item 1. It needs no new ConfigurAnt feature: it reads a resolved
//! [`configurant::Config`] and its schema. Rules:
//!
//! * A manifest setting is found by the schema value whose `env` is that name, else whose `key` is that name.
//! * A value the schema marks `secret` is **never** projected as a setting, even if the manifest asks (it would land in a plan and a log);
//!   that is an error that names the key and never shows a value.
//! * A secret is only ever **referred to**: [`secret_sources`] returns the `source` reference string from the schema (for example
//!   `vault:TOKEN`), which names the secret and is not secret. It never reads a secret's value.

use std::collections::BTreeMap;

use configurant::{Config as Resolved, Value, ValueDef};

use crate::{AppManifest, Config, Error};

/// The schema entry for a manifest name: by `env` first, then by `key`.
fn find<'a>(resolved: &'a Resolved, name: &str) -> Option<&'a ValueDef> {
    let values = resolved.schema().values();
    values
        .iter()
        .find(|v| v.env.as_deref() == Some(name))
        .or_else(|| values.iter().find(|v| v.key == name))
}

fn text(key: &str, value: &Value) -> Result<String, Error> {
    Ok(match value {
        Value::String(s) | Value::Enum(s) => s.clone(),
        Value::Path(p) => p.display().to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::List(items) => items.join(","),
        _ => {
            return Err(Error::Target(format!(
                "the setting '{key}' is a table or map, which a host's plain settings cannot hold"
            )));
        }
    })
}

/// The values of the manifest's plain settings, from a resolved ConfigurAnt configuration. A setting the schema does not define is an error; one
/// that is defined but not set is left out (so planning reports it as missing, as for any other source).
pub fn project_settings(resolved: &Resolved, manifest: &AppManifest) -> Result<Config, Error> {
    let mut out = Config::new();
    for name in &manifest.settings {
        let def = find(resolved, name).ok_or_else(|| {
            Error::Target(format!(
                "the setting '{name}' is not defined in the ConfigurAnt schema"
            ))
        })?;
        if def.secret {
            return Err(Error::Target(format!(
                "'{}' is a secret in the ConfigurAnt schema, so it cannot be a plain setting; declare it as a secret",
                def.key
            )));
        }
        if let Some(value) = resolved.value(&def.key) {
            out.insert(name.clone(), text(&def.key, value)?);
        }
    }
    Ok(out)
}

/// For each of the manifest's secrets that the schema defines **and** gives a `source`, the source reference. Names the secret; never reads its
/// value. A secret the schema does not define, or defines without a source, is absent from the answer (the consumer supplies it another way).
/// A manifest secret that the schema defines as *not* secret is an error: it would be written to a place where it is shown.
pub fn secret_sources(
    resolved: &Resolved,
    manifest: &AppManifest,
) -> Result<BTreeMap<String, String>, Error> {
    let mut out = BTreeMap::new();
    for name in &manifest.secrets {
        let Some(def) = find(resolved, name) else {
            continue;
        };
        if !def.secret {
            return Err(Error::Target(format!(
                "'{}' is declared as a secret here but is not marked secret in the ConfigurAnt schema",
                def.key
            )));
        }
        if let Some(source) = &def.source {
            out.insert(name.clone(), source.clone());
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeTarget;
    use crate::{Host, MapSecrets};
    use configurant::{Loader, Schema};

    const SCHEMA: &str = r#"
project = { name = "app", version = "1", summary = "demo", schema_version = 1 }
value = [
  { key = "web.base_url", type = "string", env = "APP_BASE_URL", description = "public URL" },
  { key = "web.port", type = "int", default = 8080, env = "APP_PORT", description = "port" },
  { key = "web.debug", type = "bool", default = false, description = "debug" },
  { key = "web.origins", type = "list", default = ["a", "b"], description = "origins" },
  { key = "web.optional", type = "string", description = "not set anywhere" },
  { key = "db.url", type = "string", secret = true, env = "DATABASE_URL", source = "vault:DB_URL", description = "database" },
  { key = "totp.key", type = "string", secret = true, env = "TOTP_KEY", description = "no source" },
]
"#;

    fn resolved(env: &[(&str, &str)]) -> Resolved {
        let schema = Schema::parse(SCHEMA).unwrap();
        let env: BTreeMap<String, String> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Loader::new(&schema).env(env).build().unwrap()
    }

    #[test]
    fn settings_are_found_by_env_name_or_key_and_rendered_as_text() {
        let r = resolved(&[("APP_BASE_URL", "https://app.example.org")]);
        let m = AppManifest::new("my-app")
            .setting("APP_BASE_URL")
            .setting("web.port")
            .setting("web.debug")
            .setting("web.origins");
        let c = project_settings(&r, &m).unwrap();
        assert_eq!(c["APP_BASE_URL"], "https://app.example.org");
        assert_eq!(c["web.port"], "8080");
        assert_eq!(c["web.debug"], "false");
        assert_eq!(c["web.origins"], "a,b");
    }

    #[test]
    fn an_unset_setting_is_left_out_and_planning_reports_it() {
        let r = resolved(&[]);
        let m = AppManifest::new("my-app").setting("web.optional");
        let c = project_settings(&r, &m).unwrap();
        assert!(c.is_empty());
        let host = Host::new(FakeTarget::new());
        assert!(matches!(
            host.plan(&m, &c),
            Err(Error::MissingSetting { .. })
        ));
    }

    #[test]
    fn an_undefined_setting_is_an_error() {
        let r = resolved(&[]);
        let m = AppManifest::new("my-app").setting("nope");
        assert!(project_settings(&r, &m).is_err());
    }

    #[test]
    fn a_secret_is_never_projected_as_a_setting_and_its_value_is_never_shown() {
        let r = resolved(&[("DATABASE_URL", "postgres://canary-password-9")]);
        let m = AppManifest::new("my-app").setting("DATABASE_URL");
        let e = project_settings(&r, &m).unwrap_err();
        let shown = format!("{e} {e:?}");
        assert!(!shown.contains("canary-password-9"));
        assert!(shown.contains("db.url"), "the error names the key");
        // Control: a plain setting in the same schema projects fine.
        let ok = AppManifest::new("my-app").setting("web.port");
        assert!(project_settings(&r, &ok).is_ok());
    }

    #[test]
    fn secret_sources_name_the_reference_and_never_read_the_value() {
        let r = resolved(&[("DATABASE_URL", "postgres://canary-password-9")]);
        let m = AppManifest::new("my-app")
            .secret("DATABASE_URL")
            .secret("TOTP_KEY") // defined, but no source
            .secret("UNDEFINED_ELSEWHERE");
        let s = secret_sources(&r, &m).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s["DATABASE_URL"], "vault:DB_URL");
        assert!(!format!("{s:?}").contains("canary-password-9"));
    }

    #[test]
    fn a_manifest_secret_that_the_schema_does_not_mark_secret_is_an_error() {
        let r = resolved(&[]);
        let m = AppManifest::new("my-app").secret("web.port");
        assert!(secret_sources(&r, &m).is_err());
    }

    #[test]
    fn projected_settings_plan_and_apply_end_to_end() {
        let r = resolved(&[("APP_BASE_URL", "https://app.example.org")]);
        let m = AppManifest::new("my-app")
            .setting("APP_BASE_URL")
            .secret("DATABASE_URL");
        let config = project_settings(&r, &m).unwrap();
        let mut host = Host::new(FakeTarget::new());
        let plan = host.plan(&m, &config).unwrap();
        let applied = host
            .apply(&plan, &MapSecrets::new().with("DATABASE_URL", "x"))
            .unwrap();
        assert!(applied.read_back.healthy);
    }
}
