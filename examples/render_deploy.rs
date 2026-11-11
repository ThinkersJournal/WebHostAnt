//! One throwaway Render web service, end to end, for the by-hand real-account check (`docs/RELEASE-CHECKLIST.md`).
//!
//! ```text
//! with-secret RENDER_API_KEY -- cargo run --example render_deploy -- --workspace tea-... --app webhostant-check-1 \
//!     --image docker.io/library/nginx:alpine --health-path / --dummy-secret CHECK_DUMMY
//! ```
//!
//! What one run does, in order: counts the workspace's services; plans and applies (create, one setting); applies a second plan (two settings changed
//! and a dummy secret added: ONE deploy at the end) and reads it back; rolls the second apply back and reads back; then DELETES the service it made
//! (also on any failure, and only when the marker shows it is its own) and counts again. It succeeds only if the count after equals the count before.
//!
//! The API key is read from the environment variable `RENDER_API_KEY` only and is never printed. The dummy secret's value is a fixed word made up here,
//! not a real secret. The plan is `free`: a free service costs nothing, and a name that does not start with `webhostant-check` is refused so that a
//! typo cannot touch a service that is not a throwaway. Output is fixed labels and counts of services, never a name, a value or an id.

use std::process::ExitCode;

use webhostant::cloudflare_domains::HttpRest;
use webhostant::render::RenderApi;
use webhostant::render_target::{Pacing, RenderTarget};
use webhostant::{
    AppManifest, Config, HealthProbe, Host, MapSecrets, Op, Presence, Rollback, SecretValue, Target,
};

const API: &str = "https://api.render.com/v1";

struct Args {
    workspace: String,
    app: String,
    image: String,
    health_path: String,
    dummy_secret: String,
    /// The port the image listens on, set as the `PORT` variable. Render expects 10000 unless told (UNVERIFIED for an image that listens elsewhere).
    port: Option<String>,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        workspace: String::new(),
        app: String::new(),
        image: String::new(),
        health_path: "/".to_string(),
        dummy_secret: String::new(),
        port: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |what: &str| it.next().ok_or(format!("{flag} needs {what}"));
        match flag.as_str() {
            "--workspace" => a.workspace = value("a workspace id")?,
            "--app" => a.app = value("a service name")?,
            "--image" => a.image = value("an image path")?,
            "--health-path" => a.health_path = value("a path")?,
            "--dummy-secret" => a.dummy_secret = value("a variable name")?,
            "--port" => a.port = Some(value("a port number")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    for (name, v) in [
        ("--workspace", &a.workspace),
        ("--app", &a.app),
        ("--image", &a.image),
        ("--dummy-secret", &a.dummy_secret),
    ] {
        if v.is_empty() {
            return Err(format!("{name} is required"));
        }
    }
    if !a.app.starts_with("webhostant-check") {
        return Err("--app must start with `webhostant-check` (throwaway services only)".into());
    }
    Ok(a)
}

fn api(workspace: &str, key: &str) -> Result<RenderApi<HttpRest>, String> {
    RenderApi::new(HttpRest::with_base(API, SecretValue::new(key)), workspace)
        .map_err(|e| e.to_string())
}

/// Delete the service this run made, if it is there and marked as ours. Never deletes anything else.
fn cleanup(workspace: &str, key: &str, app: &str, image: &str, health: &str) -> Result<(), String> {
    let target = RenderTarget::new(api(workspace, key)?, image, "free", health, |_| true);
    match target.locate(app).map_err(|e| e.to_string())? {
        Presence::Absent => Ok(()),
        Presence::NotOurs => {
            Err("a service of that name exists and is not ours: left alone".into())
        }
        Presence::Ours => {
            let render = api(workspace, key)?;
            let service = render
                .find_service(app)
                .map_err(|e| e.to_string())?
                .ok_or("the service vanished before it could be deleted")?;
            render
                .delete_service(&service.id)
                .map_err(|e| e.to_string())
        }
    }
}

fn main() -> ExitCode {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };
    let Ok(key) = std::env::var("RENDER_API_KEY") else {
        eprintln!("RENDER_API_KEY is not set (run under with-secret)");
        return ExitCode::from(2);
    };
    let counter = match api(&a.workspace, &key) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };
    let before = match counter.service_count() {
        Ok(n) => n,
        Err(e) => {
            eprintln!("could not count services: {e}");
            return ExitCode::from(1);
        }
    };
    println!("services before: {before}");

    let outcome = run(&a, &key);
    // Always: remove what this run made, then count again.
    let cleaned = cleanup(&a.workspace, &key, &a.app, &a.image, &a.health_path);
    match &cleaned {
        Ok(()) => println!("cleanup: done"),
        Err(e) => eprintln!("cleanup failed: {e}"),
    }
    let after = counter.service_count();
    match &after {
        Ok(n) => println!("services after: {n}"),
        Err(e) => eprintln!("could not count services: {e}"),
    }
    let same = matches!(after, Ok(n) if n == before);
    println!("account unchanged: {same}");
    match (outcome, cleaned.is_ok() && same) {
        (Ok(()), true) => {
            println!("PASS");
            ExitCode::SUCCESS
        }
        (Err(e), _) => {
            eprintln!("run failed: {e}");
            ExitCode::from(1)
        }
        (Ok(()), false) => ExitCode::from(1),
    }
}

fn run(a: &Args, key: &str) -> Result<(), String> {
    let url_key = key.to_string();
    let (ws, app) = (a.workspace.clone(), a.app.clone());
    // The health probe asks the service's own public address, looked up when it is asked.
    let probe = HealthProbe::new().into_probe(move |_| {
        api(&ws, &url_key)
            .ok()
            .and_then(|r| r.find_service(&app).ok().flatten())
            .and_then(|s| s.url)
            .unwrap_or_default()
    });
    let target = RenderTarget::new(
        api(&a.workspace, key)?,
        &a.image,
        "free",
        &a.health_path,
        probe,
    )
    .with_pacing(Pacing::default())
    .trust_operations(&[Op::Create, Op::SetSetting, Op::SetSecret, Op::Restore]);
    let mut host = Host::new(target);
    let result = steps(&mut host, a);
    if result.is_err() {
        // What the adapter logged: step names and each deploy status Render reported as it changed. Never a value or a key.
        eprintln!(
            "adapter log:
{}",
            host.target().diagnostics()
        );
    }
    result
}

fn steps(host: &mut Host<RenderTarget<HttpRest>>, a: &Args) -> Result<(), String> {
    // 1. Create, with one setting.
    let mut first_manifest = AppManifest::new(&a.app)
        .health(&a.health_path)
        .setting("CHECK_MODE");
    let mut config = Config::new();
    if let Some(port) = &a.port {
        first_manifest = first_manifest.setting("PORT");
        config.insert("PORT".into(), port.clone());
    }
    config.insert("CHECK_MODE".into(), "one".into());
    let plan = host
        .plan(&first_manifest, &config)
        .map_err(|e| e.to_string())?;
    let secrets = MapSecrets::new().with(&a.dummy_secret, "dummy-not-a-real-secret");
    host.apply_approved(&plan, &plan.digest(), &secrets)
        .map_err(|e| format!("first apply: {e}"))?;
    println!("first apply: ok, read back");

    // 2. Two settings changed and a secret added: one deploy at the end.
    let mut second_manifest = AppManifest::new(&a.app)
        .health(&a.health_path)
        .setting("CHECK_MODE")
        .setting("CHECK_EXTRA")
        .secret(&a.dummy_secret);
    if a.port.is_some() {
        second_manifest = second_manifest.setting("PORT");
    }
    config.insert("CHECK_MODE".into(), "two".into());
    config.insert("CHECK_EXTRA".into(), "x".into());
    let plan = host
        .plan(&second_manifest, &config)
        .map_err(|e| e.to_string())?;
    let done = host
        .apply_approved(&plan, &plan.digest(), &secrets)
        .map_err(|e| format!("second apply: {e}"))?;
    println!(
        "second apply: ok, {} step(s), read back, healthy {}",
        done.steps_applied, done.read_back.healthy
    );

    // 3. Roll the second apply back.
    let point = done.rollback_point.ok_or("no rollback point")?;
    match host.rollback(&point) {
        Rollback::Restored => println!("rollback: the target matches the snapshot"),
        Rollback::Unchanged => println!("rollback: nothing had been applied"),
        Rollback::Unknown { reason, .. } => {
            return Err(format!("rollback: state not known: {reason}"));
        }
    }
    Ok(())
}
