//! Plan, approve, apply and read back one Cloudflare Worker, from the command line.
//!
//! Two runs, on purpose. The first prints the plan and its digest and changes NOTHING:
//!
//! ```text
//! cargo run --example deploy -- --project ./worker --app my-app --health-url https://my-app.example.workers.dev/health \
//!     --setting APP_BASE_URL=https://my-app.example.workers.dev --secret DATABASE_URL --without-secret OLD_KEY --containers
//! ```
//!
//! The second applies exactly that plan, and only if its digest is the one given:
//!
//! ```text
//! ... the same arguments ... --approve <digest> --trust create,set-setting,set-secret,delete-secret
//! ```
//!
//! Secret VALUES are read from environment variables of the secret's own name (for example exported by `with-secret`), never from arguments. Nothing
//! here prints a value, an email or an account. `--trust` claims an operation was proven by a real run: use it only as `docs/RELEASE-CHECKLIST.md`
//! says, one operation at a time, on a throwaway Worker.

use std::process::ExitCode;
use std::time::Duration;

use webhostant::cloudflare::{CloudflareTarget, ProcessRunner};
use webhostant::preflight::Preflight;
use webhostant::{AppManifest, Config, EnvSecrets, HealthProbe, Host, Op, WaitPolicy};

struct Args {
    project: String,
    app: String,
    health_url: String,
    health_path: String,
    settings: Vec<(String, String)>,
    secrets: Vec<String>,
    without: Vec<String>,
    containers: bool,
    approve: Option<String>,
    trust: Vec<Op>,
}

fn parse() -> Result<Args, String> {
    let mut a = Args {
        project: String::new(),
        app: String::new(),
        health_url: String::new(),
        health_path: "/health".to_string(),
        settings: Vec::new(),
        secrets: Vec::new(),
        without: Vec::new(),
        containers: false,
        approve: None,
        trust: Vec::new(),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = |what: &str| it.next().ok_or(format!("{flag} needs {what}"));
        match flag.as_str() {
            "--project" => a.project = value("a directory")?,
            "--app" => a.app = value("an app id")?,
            "--health-url" => a.health_url = value("a URL")?,
            "--health-path" => a.health_path = value("a path")?,
            "--setting" => {
                let kv = value("NAME=VALUE")?;
                let (k, v) = kv.split_once('=').ok_or("--setting needs NAME=VALUE")?;
                a.settings.push((k.to_string(), v.to_string()));
            }
            "--secret" => a.secrets.push(value("a name")?),
            "--without-secret" => a.without.push(value("a name")?),
            "--containers" => a.containers = true,
            "--approve" => a.approve = Some(value("a digest")?),
            "--trust" => {
                for name in value("a list")?.split(',') {
                    a.trust.push(match name {
                        "create" => Op::Create,
                        "set-setting" => Op::SetSetting,
                        "set-secret" => Op::SetSecret,
                        "delete-secret" => Op::DeleteSecret,
                        other => return Err(format!("--trust: unknown operation `{other}`")),
                    });
                }
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    for (name, v) in [
        ("--project", &a.project),
        ("--app", &a.app),
        ("--health-url", &a.health_url),
    ] {
        if v.is_empty() {
            return Err(format!("{name} is required"));
        }
    }
    Ok(a)
}

fn main() -> ExitCode {
    let a = match parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(2);
        }
    };

    // 1. Preflight: stop before anything is planned if the machine cannot deploy.
    let wrangler = ProcessRunner::with_timeout(Duration::from_secs(60));
    let docker = ProcessRunner::program("docker", Duration::from_secs(60));
    let mut check = Preflight::new(wrangler, docker);
    if a.containers {
        check = check.with_containers();
    }
    let report = check.run();
    print!("{report}");
    if !report.ready() {
        eprintln!("not ready: fix the PROBLEM lines above first");
        return ExitCode::from(1);
    }

    // 2. Plan (read-only).
    let url = a.health_url.clone();
    let plain = HealthProbe::new().into_probe({
        let url = url.clone();
        move |_| url.clone()
    });
    let waiting =
        HealthProbe::new().into_waiting_probe(WaitPolicy::default(), move |_| url.clone());
    let target = CloudflareTarget::new(ProcessRunner::default(), &a.project, plain)
        .with_ready_wait(waiting)
        .trust_operations(&a.trust);
    let mut host = Host::new(target);
    let mut manifest = AppManifest::new(&a.app).health(&a.health_path);
    let mut config = Config::new();
    for (k, v) in &a.settings {
        manifest = manifest.setting(k);
        config.insert(k.clone(), v.clone());
    }
    for s in &a.secrets {
        manifest = manifest.secret(s);
    }
    for s in &a.without {
        manifest = manifest.without_secret(s);
    }
    let plan = match host.plan(&manifest, &config) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("plan refused: {e}");
            return ExitCode::from(1);
        }
    };
    print!("{plan}");
    println!("digest: {}", plan.digest());
    let Some(approved) = a.approve else {
        println!(
            "nothing was changed. To apply exactly this plan, run again with --approve <that digest>."
        );
        return ExitCode::SUCCESS;
    };

    // 3. Apply only the approved plan, then read the host back.
    match host.apply_approved(&plan, &approved, &EnvSecrets::new()) {
        Ok(done) => {
            println!(
                "applied {} step(s). read back: settings {:?}, secrets {:?}, healthy {}",
                done.steps_applied,
                done.read_back.settings,
                done.read_back.secret_names,
                done.read_back.healthy
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("apply failed: {e}");
            ExitCode::from(1)
        }
    }
}
