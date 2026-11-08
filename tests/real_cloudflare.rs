//! Real-target tests for the Cloudflare adapter: what `examples/deploy.rs` did by hand on 2026-11-08, as a test that runs on a self-hosted runner
//! against the D6 test account (CIRESNAVE rule: tests that cannot run on GitHub run on a self-hosted runner, never `#[ignore]`).
//!
//! It always compiles (so `cargo build --all-targets` checks it), and it is a real run only when `WEBHOSTANT_REAL_TARGET=1`. Then the account id and the
//! API token MUST be in `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` (the token from the secrets store, the account id from the environment so CI can
//! substitute it); a missing one fails the run by name, it never skips. Without the flag the real test prints one fixed line and returns: that is the
//! GitHub-hosted case, where there is no account.
//!
//! Safety, in the order a run does it: the account's Workers are counted and fingerprinted first; every Worker made has a throwaway name
//! (`webhostant-ci-...`) and is deleted whatever happens (a guard that runs on a panic too); at the end the account's Workers must have the same
//! fingerprint, so a leftover Worker FAILS the run. Nothing prints a secret value, a token, a Worker name or the account id.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use webhostant::cloudflare::{CloudflareTarget, ProcessRunner};
use webhostant::cloudflare_domains::{HttpRest, Method, Rest, RestCall};
use webhostant::{
    AppManifest, Config, HealthProbe, Host, MapSecrets, Op, Presence, Rollback, SecretValue,
    Target, WaitPolicy,
};

/// The two things a real run needs from the environment.
struct Env {
    account_id: String,
    token: String,
}

/// Read the real-run environment through `lookup`. `Ok(None)` is the GitHub-hosted case (no flag); a flag with a missing variable is an error that
/// names the variable (never a value).
fn real_env(lookup: impl Fn(&str) -> Option<String>) -> Result<Option<Env>, String> {
    if lookup("WEBHOSTANT_REAL_TARGET").as_deref() != Some("1") {
        return Ok(None);
    }
    let need = |name: &str| {
        lookup(name)
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| format!("WEBHOSTANT_REAL_TARGET=1 but {name} is not set"))
    };
    Ok(Some(Env {
        account_id: need("CLOUDFLARE_ACCOUNT_ID")?,
        token: need("CLOUDFLARE_API_TOKEN")?,
    }))
}

/// A throwaway Worker name: `webhostant-ci-<tag>-<pid>-<seconds>`, lowercase letters, digits and hyphens only.
fn throwaway_name(tag: &str) -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    format!("webhostant-ci-{tag}-{}-{secs}", std::process::id())
}

/// The names in one page of Cloudflare's Workers list, and how many pages there are.
fn parse_scripts_page(body: &str) -> Result<(Vec<String>, u64), String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|_| "the list was not JSON")?;
    if v.get("success").and_then(|s| s.as_bool()) != Some(true) {
        return Err("Cloudflare did not report success for the Workers list".to_string());
    }
    let names = v
        .get("result")
        .and_then(|r| r.as_array())
        .ok_or("no result array")?
        .iter()
        .filter_map(|s| s.get("id")?.as_str().map(str::to_string))
        .collect();
    let pages = v
        .get("result_info")
        .and_then(|i| i.get("total_pages")?.as_u64())
        .unwrap_or(1);
    Ok((names, pages))
}

/// How many Workers the account has and a fingerprint of their names (the names themselves are not printed).
fn worker_fingerprint(rest: &dyn Rest, account_id: &str) -> Result<(usize, String), String> {
    let mut names = Vec::new();
    let mut page = 1u64;
    loop {
        let answer = rest
            .send(&RestCall {
                method: Method::Get,
                path: format!("/accounts/{account_id}/workers/scripts"),
                query: vec![
                    ("per_page".to_string(), "100".to_string()),
                    ("page".to_string(), page.to_string()),
                ],
                body: None,
            })
            .map_err(|_| "could not list the account's Workers".to_string())?;
        if answer.status != 200 {
            return Err("the Workers list answered a non-200 status".to_string());
        }
        let (mut got, pages) = parse_scripts_page(&answer.body)?;
        names.append(&mut got);
        if page >= pages {
            break;
        }
        page += 1;
    }
    names.sort();
    let digest = Sha256::digest(names.join("\n").as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Ok((names.len(), hex[..16].to_string()))
}

/// The account's workers.dev subdomain, which a throwaway Worker's health URL needs.
fn workers_dev_subdomain(rest: &dyn Rest, account_id: &str) -> Result<String, String> {
    let answer = rest
        .send(&RestCall {
            method: Method::Get,
            path: format!("/accounts/{account_id}/workers/subdomain"),
            query: Vec::new(),
            body: None,
        })
        .map_err(|_| "could not read the workers.dev subdomain".to_string())?;
    serde_json::from_str::<serde_json::Value>(&answer.body)
        .ok()
        .and_then(|v| {
            v.get("result")?
                .get("subdomain")?
                .as_str()
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "the test account has no workers.dev subdomain; register one once in the dashboard"
                .to_string()
        })
}

/// A throwaway Worker project on disk, deleted from Cloudflare and from disk when this is dropped, even on a panic.
struct Throwaway {
    name: String,
    dir: PathBuf,
}

impl Throwaway {
    fn new(tag: &str) -> Self {
        let name = throwaway_name(tag);
        let dir = std::env::temp_dir().join(&name);
        std::fs::create_dir_all(&dir).expect("could not make the throwaway project directory");
        std::fs::write(
            dir.join("wrangler.toml"),
            format!(
                "name = \"{name}\"\nmain = \"index.js\"\ncompatibility_date = \"2026-11-01\"\nworkers_dev = true\n"
            ),
        )
        .expect("could not write wrangler.toml");
        std::fs::write(
            dir.join("index.js"),
            "export default { async fetch(request) { const u = new URL(request.url); \
             if (u.pathname === \"/health\") { return new Response(JSON.stringify({status:\"ok\"}), \
             {headers:{\"content-type\":\"application/json\"}}); } return new Response(\"throwaway\\n\"); } };\n",
        )
        .expect("could not write index.js");
        Self { name, dir }
    }

    /// `wrangler delete`, best effort (the Worker may not exist). Returns whether wrangler said it succeeded.
    fn delete_worker(&self) -> bool {
        let mut cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "wrangler"]);
            c
        } else {
            Command::new("wrangler")
        };
        cmd.args(["delete", "--name", &self.name, "--cwd"])
            .arg(&self.dir)
            .arg("--force")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        self.delete_worker();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn target_for(w: &Throwaway, health_url: String) -> CloudflareTarget<ProcessRunner> {
    let url = health_url.clone();
    let plain = HealthProbe::new().into_probe(move |_| url.clone());
    let policy = WaitPolicy {
        deadline: Duration::from_secs(180),
        interval: Duration::from_secs(10),
    };
    let waiting = HealthProbe::new().into_waiting_probe(policy, move |_| health_url.clone());
    CloudflareTarget::new(
        ProcessRunner::with_timeout(Duration::from_secs(300)),
        &w.dir,
        plain,
    )
    .with_ready_wait(waiting)
    .trust_operations(&[
        Op::Create,
        Op::SetSetting,
        Op::SetSecret,
        Op::DeleteSecret,
        Op::Restore,
    ])
}

fn config(mode: &str) -> Config {
    BTreeMap::from([("CHECK_MODE".to_string(), mode.to_string())])
}

/// Plan and apply exactly the plan that was planned (through its digest), returning the apply result.
fn plan_apply(
    host: &mut Host<CloudflareTarget<ProcessRunner>>,
    manifest: &AppManifest,
    cfg: &Config,
    secrets: &MapSecrets,
    label: &str,
) -> webhostant::Applied {
    let plan = host
        .plan(manifest, cfg)
        .unwrap_or_else(|_| panic!("{label}: the plan was refused"));
    host.apply_approved(&plan, &plan.digest(), secrets)
        .unwrap_or_else(|_| panic!("{label}: the apply failed"))
}

#[test]
fn real_cloudflare_create_setting_secret_delete_restore_and_cleanup() {
    let env = match real_env(|k| std::env::var(k).ok()) {
        Ok(Some(env)) => env,
        Ok(None) => {
            println!(
                "not a real-target run (WEBHOSTANT_REAL_TARGET is not 1): nothing was touched"
            );
            return;
        }
        Err(message) => panic!("{message}"),
    };
    let rest = HttpRest::new(SecretValue::new(env.token.clone()));
    let before = worker_fingerprint(&rest, &env.account_id).expect("the starting list of Workers");
    let sub = workers_dev_subdomain(&rest, &env.account_id).expect("the workers.dev subdomain");
    println!(
        "account Workers before: {} fingerprint {}",
        before.0, before.1
    );

    // Worker A: create, setting, secret, delete the secret, then restore a later change back.
    {
        let w = Throwaway::new("a");
        let health = format!("https://{}.{sub}.workers.dev/health", w.name);
        let mut host = Host::new(target_for(&w, health));
        let dummy = MapSecrets::new().with("CHECK_SECRET", "dummy-not-a-secret-0000");
        let none = MapSecrets::new();
        let base = AppManifest::new(&w.name).setting("CHECK_MODE");

        plan_apply(
            &mut host,
            &base,
            &config("one"),
            &none,
            "create and setting",
        );
        plan_apply(
            &mut host,
            &base.clone().secret("CHECK_SECRET"),
            &config("one"),
            &dummy,
            "secret",
        );
        let seen = host
            .target()
            .observe(&w.name)
            .expect("observe after the secret");
        assert!(
            !seen.secret_names.is_empty(),
            "the secret was not read back"
        );
        plan_apply(
            &mut host,
            &base.clone().without_secret("CHECK_SECRET"),
            &config("one"),
            &none,
            "delete the secret",
        );
        let seen = host
            .target()
            .observe(&w.name)
            .expect("observe after the delete");
        assert!(
            seen.secret_names.is_empty(),
            "the deleted secret is still there"
        );

        // A later change, then roll it back.
        let later = base.clone().secret("CHECK_SECRET");
        let applied = plan_apply(&mut host, &later, &config("two"), &dummy, "a later change");
        let point = applied
            .rollback_point
            .expect("restore is trusted, so there is a rollback point");
        assert_eq!(
            host.rollback(&point),
            Rollback::Restored,
            "the rollback did not match the snapshot"
        );
        let back = host
            .target()
            .observe(&w.name)
            .expect("observe after the rollback");
        assert_eq!(
            back.settings.get("CHECK_MODE").map(String::as_str),
            Some("one")
        );
        assert!(
            back.secret_names.is_empty(),
            "the secret made after the snapshot is still there"
        );
        assert!(w.delete_worker(), "the cleanup did not succeed");
        assert_eq!(
            host.target()
                .locate(&w.name)
                .expect("locate after the cleanup"),
            Presence::Absent,
            "the Worker is still there after the cleanup"
        );
    }

    // Worker B: roll back a first deploy; the Worker this apply created must be deleted.
    {
        let w = Throwaway::new("b");
        let health = format!("https://{}.{sub}.workers.dev/health", w.name);
        let mut host = Host::new(target_for(&w, health));
        let manifest = AppManifest::new(&w.name).setting("CHECK_MODE");
        let applied = plan_apply(
            &mut host,
            &manifest,
            &config("one"),
            &MapSecrets::new(),
            "first deploy",
        );
        let point = applied
            .rollback_point
            .expect("a rollback point for a first deploy");
        assert_eq!(
            host.rollback(&point),
            Rollback::Restored,
            "rolling back a first deploy"
        );
        assert_eq!(
            host.target()
                .locate(&w.name)
                .expect("locate after the first-deploy rollback"),
            Presence::Absent
        );
    }

    let after = worker_fingerprint(&rest, &env.account_id).expect("the ending list of Workers");
    println!("account Workers after: {} fingerprint {}", after.0, after.1);
    assert_eq!(
        before, after,
        "the account's Workers changed: a leftover Worker fails the run"
    );
}

#[test]
fn without_the_flag_nothing_is_a_real_run() {
    let none = real_env(|_| None).unwrap();
    assert!(none.is_none());
    // Any value but "1" is not a real run either, even with credentials present.
    let env = |k: &str| match k {
        "WEBHOSTANT_REAL_TARGET" => Some("yes".to_string()),
        _ => Some("x".to_string()),
    };
    assert!(real_env(env).unwrap().is_none());
}

#[test]
fn with_the_flag_a_missing_variable_fails_by_name_and_never_skips() {
    let only_flag = |k: &str| (k == "WEBHOSTANT_REAL_TARGET").then(|| "1".to_string());
    let e = real_env(only_flag).err().expect("must fail, not skip");
    assert!(e.contains("CLOUDFLARE_ACCOUNT_ID"), "{e}");
    let no_token = |k: &str| match k {
        "WEBHOSTANT_REAL_TARGET" => Some("1".to_string()),
        "CLOUDFLARE_ACCOUNT_ID" => Some("abc".to_string()),
        _ => None,
    };
    let e = real_env(no_token).err().expect("must fail, not skip");
    assert!(e.contains("CLOUDFLARE_API_TOKEN"), "{e}");
    let blank = |k: &str| match k {
        "WEBHOSTANT_REAL_TARGET" => Some("1".to_string()),
        _ => Some("  ".to_string()),
    };
    assert!(real_env(blank).is_err(), "a blank variable is missing");
    let all = |k: &str| {
        Some(
            if k == "WEBHOSTANT_REAL_TARGET" {
                "1"
            } else {
                "v"
            }
            .to_string(),
        )
    };
    let ok = real_env(all).unwrap().expect("a real run");
    assert_eq!((ok.account_id.as_str(), ok.token.as_str()), ("v", "v"));
}

#[test]
fn throwaway_names_are_valid_unique_per_tag_and_marked() {
    let a = throwaway_name("a");
    let b = throwaway_name("b");
    for n in [&a, &b] {
        assert!(n.starts_with("webhostant-ci-"));
        assert!(n.len() <= 63);
        assert!(
            n.bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
        );
    }
    assert_ne!(a, b);
}

#[test]
fn the_workers_list_is_read_across_pages_and_a_failure_is_not_an_empty_list() {
    use webhostant::cloudflare_domains::RestAnswer;
    /// Serves the given pages, in the given name order.
    struct Pages(&'static [&'static str]);
    impl Rest for Pages {
        fn send(&self, call: &RestCall) -> Result<RestAnswer, webhostant::Error> {
            let page: usize = call
                .query
                .iter()
                .find(|(k, _)| k == "page")
                .and_then(|(_, v)| v.parse().ok())
                .unwrap_or(0);
            let body = self.0.get(page.wrapping_sub(1)).copied().unwrap_or("{}");
            Ok(RestAnswer {
                status: 200,
                body: body.to_string(),
            })
        }
    }
    const P1: &str =
        r#"{"success":true,"result":[{"id":"b"},{"id":"a"}],"result_info":{"total_pages":2}}"#;
    const P2: &str = r#"{"success":true,"result":[{"id":"c"}],"result_info":{"total_pages":2}}"#;
    let (count, fp) = worker_fingerprint(&Pages(&[P1, P2]), "acct").unwrap();
    assert_eq!(count, 3);
    // The same Workers listed in another order (another page split) have the same fingerprint; a different set does not.
    const Q1: &str =
        r#"{"success":true,"result":[{"id":"c"},{"id":"a"}],"result_info":{"total_pages":2}}"#;
    const Q2: &str = r#"{"success":true,"result":[{"id":"b"}],"result_info":{"total_pages":2}}"#;
    let (_, other_order) = worker_fingerprint(&Pages(&[Q1, Q2]), "acct").unwrap();
    assert_eq!(fp, other_order);
    const R2: &str = r#"{"success":true,"result":[{"id":"d"}],"result_info":{"total_pages":2}}"#;
    let (_, other_set) = worker_fingerprint(&Pages(&[P1, R2]), "acct").unwrap();
    assert_ne!(fp, other_set);

    // A refusal is an error even when its body looks like a success: a status other than 200 is never trusted.
    struct Refuses;
    impl Rest for Refuses {
        fn send(&self, _: &RestCall) -> Result<RestAnswer, webhostant::Error> {
            Ok(RestAnswer {
                status: 403,
                body: P2.to_string(),
            })
        }
    }
    assert!(worker_fingerprint(&Refuses, "acct").is_err());
    assert!(parse_scripts_page(r#"{"success":false,"result":[]}"#).is_err());
}
