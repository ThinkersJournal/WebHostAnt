//! A read-only check, before a Cloudflare deploy, of what the deploy needs on this machine: `wrangler` present and recent enough, a login, the
//! token's scopes, and (for a container) Docker with a running daemon.
//!
//! It changes nothing and prints nothing about the person or the account: no email, no account id or name, no token. It reports scope NAMES that
//! are missing, and nothing else from `wrangler whoami`. It exists because a missing scope (Humboldt's demo needed the Containers scope) otherwise fails late,
//! in the middle of a deploy.
//!
//! Scope names below are the ones `wrangler login` lists in `whoami --json` `tokenPermissions` on 2026-11-08 (4.141.0). That they are the MINIMUM a
//! deploy needs is **UNVERIFIED** (D6); they are what the demo's working login held. An API token makes `wrangler` list no scopes at all: the report
//! then says `Unknown`, never `Ok`.

use std::fmt;

use serde_json::Value;

use crate::cloudflare::{Call, Runner};

/// The oldest `wrangler` this adapter's tests ran against.
pub const MIN_WRANGLER: (u32, u32, u32) = (4, 141, 0);

/// Scopes a Worker deploy needs (UNVERIFIED as a minimum).
pub const WORKER_SCOPES: [&str; 4] = [
    "account:read",
    "user:read",
    "workers:write",
    "workers_scripts:write",
];

/// Extra scopes a container deploy needs (UNVERIFIED as a minimum).
pub const CONTAINER_SCOPES: [&str; 2] = ["containers:write", "cloudchamber:write"];

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    /// Checked and fine.
    Ok,
    /// Checked and not fine: the deploy would fail.
    Problem,
    /// Could not be checked from here; not a block, and not a pass.
    Unknown,
}

/// One line of a report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What was checked.
    pub what: &'static str,
    /// How it came out.
    pub state: State,
    /// What to do, or what was seen. Never an email, account or token.
    pub detail: String,
}

/// The result of [`Preflight::run`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    /// Every check, in the order it ran.
    pub items: Vec<Item>,
}

impl Report {
    /// True when no check found a problem. `Unknown` does not block.
    pub fn ready(&self) -> bool {
        self.items.iter().all(|i| i.state != State::Problem)
    }
}

impl fmt::Display for Report {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for i in &self.items {
            let tag = match i.state {
                State::Ok => "ok     ",
                State::Problem => "PROBLEM",
                State::Unknown => "unknown",
            };
            writeln!(f, "{tag} {}: {}", i.what, i.detail)?;
        }
        Ok(())
    }
}

/// The checks. `wrangler` and `docker` are two runners so a test can answer each.
pub struct Preflight<W: Runner, D: Runner> {
    wrangler: W,
    docker: D,
    containers: bool,
    min_wrangler: (u32, u32, u32),
}

impl<W: Runner, D: Runner> Preflight<W, D> {
    /// Check for a plain Worker deploy.
    pub fn new(wrangler: W, docker: D) -> Self {
        Self {
            wrangler,
            docker,
            containers: false,
            min_wrangler: MIN_WRANGLER,
        }
    }

    /// The app is a container: also check the Containers scopes and Docker.
    pub fn with_containers(mut self) -> Self {
        self.containers = true;
        self
    }

    /// Ask for a different oldest `wrangler`.
    pub fn min_wrangler(mut self, version: (u32, u32, u32)) -> Self {
        self.min_wrangler = version;
        self
    }

    /// Run every check. Only reads.
    pub fn run(&self) -> Report {
        let mut items = vec![self.check_wrangler()];
        items.extend(self.check_login());
        if self.containers {
            items.extend(self.check_docker());
        }
        Report { items }
    }

    fn ask(runner: &impl Runner, args: &[&str]) -> Option<crate::cloudflare::Output> {
        let call = Call {
            args: args.iter().map(|a| a.to_string()).collect(),
            stdin: None,
        };
        runner.run(&call).ok()
    }

    fn check_wrangler(&self) -> Item {
        let what = "wrangler";
        let Some(out) = Self::ask(&self.wrangler, &["--version"]).filter(|o| o.success) else {
            return Item {
                what,
                state: State::Problem,
                detail: "not found or not runnable; install Node and wrangler".to_string(),
            };
        };
        match parse_version(&out.stdout) {
            Some(v) if v >= self.min_wrangler => Item {
                what,
                state: State::Ok,
                detail: format!("{}.{}.{}", v.0, v.1, v.2),
            },
            Some(v) => Item {
                what,
                state: State::Problem,
                detail: format!(
                    "{}.{}.{} is older than {}.{}.{}; update wrangler",
                    v.0, v.1, v.2, self.min_wrangler.0, self.min_wrangler.1, self.min_wrangler.2
                ),
            },
            None => Item {
                what,
                state: State::Unknown,
                detail: "ran, but its version could not be read".to_string(),
            },
        }
    }

    fn check_login(&self) -> Vec<Item> {
        let login = "cloudflare login";
        let Some(out) = Self::ask(&self.wrangler, &["whoami", "--json"]) else {
            return vec![Item {
                what: login,
                state: State::Unknown,
                detail: "wrangler could not be run, so nothing was checked".to_string(),
            }];
        };
        let parsed = serde_json::from_str::<Value>(&out.stdout).ok();
        let logged_in = out.success
            && parsed
                .as_ref()
                .and_then(|v| v.get("loggedIn")?.as_bool())
                .unwrap_or(false);
        if !logged_in {
            return vec![Item {
                what: login,
                state: State::Problem,
                detail: "not logged in; run `wrangler login`, or set CLOUDFLARE_API_TOKEN"
                    .to_string(),
            }];
        }
        let mut items = vec![Item {
            what: login,
            state: State::Ok,
            detail: "logged in".to_string(),
        }];
        let granted: Option<Vec<&str>> = parsed
            .as_ref()
            .and_then(|v| v.get("tokenPermissions")?.as_array())
            .map(|a| a.iter().filter_map(Value::as_str).collect());
        let mut need: Vec<&str> = WORKER_SCOPES.to_vec();
        if self.containers {
            need.extend(CONTAINER_SCOPES);
        }
        items.push(match granted {
            None => Item {
                what: "token scopes",
                state: State::Unknown,
                detail: "wrangler lists no scopes for this kind of login (an API token); a deploy will show if one is missing".to_string(),
            },
            Some(g) => {
                let missing: Vec<&str> = need.iter().copied().filter(|n| !g.contains(n)).collect();
                if missing.is_empty() {
                    Item {
                        what: "token scopes",
                        state: State::Ok,
                        detail: format!("all {} needed scopes present", need.len()),
                    }
                } else {
                    Item {
                        what: "token scopes",
                        state: State::Problem,
                        detail: format!("missing: {}; log in again with them or use a token that has them", missing.join(", ")),
                    }
                }
            }
        });
        items
    }

    fn check_docker(&self) -> Vec<Item> {
        let installed = Self::ask(&self.docker, &["--version"]).is_some_and(|o| o.success);
        if !installed {
            return vec![Item {
                what: "docker",
                state: State::Problem,
                detail: "not found; a container deploy builds an image with Docker on this machine"
                    .to_string(),
            }];
        }
        let running = Self::ask(&self.docker, &["info", "--format", "{{.ServerVersion}}"])
            .is_some_and(|o| o.success && !o.stdout.trim().is_empty());
        vec![Item {
            what: "docker",
            state: if running { State::Ok } else { State::Problem },
            detail: if running {
                "installed, daemon answering".to_string()
            } else {
                "installed, but the daemon is not answering; start Docker".to_string()
            },
        }]
    }
}

/// The first `major.minor.patch` in `text`, numerically (so 4.9.0 is older than 4.141.0).
fn parse_version(text: &str) -> Option<(u32, u32, u32)> {
    text.split(|c: char| !(c.is_ascii_digit() || c == '.'))
        .filter_map(|tok| {
            let mut p = tok.split('.');
            let v = (
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
                p.next()?.parse().ok()?,
            );
            p.next().is_none().then_some(v)
        })
        .next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::cloudflare::Output;

    type Answer = Box<dyn Fn(&[&str]) -> Result<Output, Error>>;
    struct Fake(Answer);
    impl Runner for Fake {
        fn run(&self, call: &Call) -> Result<Output, Error> {
            let a: Vec<&str> = call.args.iter().map(String::as_str).collect();
            (self.0)(&a)
        }
    }

    fn out(ok: bool, stdout: &str) -> Result<Output, Error> {
        Ok(Output {
            success: ok,
            stdout: stdout.to_string(),
            stderr: String::new(),
        })
    }

    const EMAIL: &str = "canary-person@example.invalid";
    const ACCOUNT: &str = "CANARY-ACCOUNT-NAME";

    fn whoami(scopes: &[&str]) -> String {
        serde_json::json!({
            "loggedIn": true, "authType": "OAuth Token", "email": EMAIL,
            "accounts": [{"id": "abc123", "name": ACCOUNT, "type": "standard"}],
            "tokenPermissions": scopes,
        })
        .to_string()
    }

    fn wrangler(version: &'static str, who: String) -> Fake {
        Fake(Box::new(move |a| match a {
            ["--version"] => out(true, version),
            ["whoami", "--json"] => out(true, &who),
            _ => out(false, ""),
        }))
    }

    fn docker(installed: bool, daemon: bool) -> Fake {
        Fake(Box::new(move |a| match a {
            ["--version"] if installed => out(true, "Docker version 29.5.3"),
            ["info", ..] if installed && daemon => out(true, "29.5.3\n"),
            _ if installed => out(false, ""),
            _ => Err(Error::Target("not found".to_string())),
        }))
    }

    fn all_scopes() -> Vec<&'static str> {
        WORKER_SCOPES
            .iter()
            .chain(CONTAINER_SCOPES.iter())
            .copied()
            .collect()
    }

    fn states(r: &Report) -> Vec<(&'static str, State)> {
        r.items.iter().map(|i| (i.what, i.state)).collect()
    }

    #[test]
    fn a_ready_machine_passes_every_check() {
        let p = Preflight::new(
            wrangler("4.141.0", whoami(&all_scopes())),
            docker(true, true),
        )
        .with_containers();
        let r = p.run();
        assert!(r.ready(), "{r}");
        assert!(r.items.iter().all(|i| i.state == State::Ok), "{r}");
        assert_eq!(r.items.len(), 4);
    }

    #[test]
    fn a_missing_scope_is_named_and_blocks() {
        let some: Vec<&str> = all_scopes()
            .into_iter()
            .filter(|s| *s != "containers:write")
            .collect();
        let r = Preflight::new(wrangler("4.141.0", whoami(&some)), docker(true, true))
            .with_containers()
            .run();
        assert!(!r.ready());
        let scopes = r.items.iter().find(|i| i.what == "token scopes").unwrap();
        assert_eq!(scopes.state, State::Problem);
        assert!(scopes.detail.contains("containers:write"), "{r}");
        assert!(
            !scopes.detail.contains("workers:write"),
            "only what is missing is named: {r}"
        );
    }

    #[test]
    fn container_scopes_are_asked_only_for_a_container() {
        let worker_only: Vec<&str> = WORKER_SCOPES.to_vec();
        let plain = Preflight::new(
            wrangler("4.141.0", whoami(&worker_only)),
            docker(false, false),
        )
        .run();
        assert!(plain.ready(), "{plain}");
        assert!(
            plain.items.iter().all(|i| i.what != "docker"),
            "docker is not checked for a plain Worker"
        );
        let container = Preflight::new(
            wrangler("4.141.0", whoami(&worker_only)),
            docker(true, true),
        )
        .with_containers()
        .run();
        assert!(!container.ready(), "{container}");
    }

    #[test]
    fn versions_compare_as_numbers_not_text() {
        // As text "4.9.0" > "4.141.0"; as numbers it is older.
        let old =
            Preflight::new(wrangler("4.9.0", whoami(&all_scopes())), docker(true, true)).run();
        assert_eq!(states(&old)[0], ("wrangler", State::Problem), "{old}");
        let banner = Preflight::new(
            wrangler(" ⛅️ wrangler 4.200.1\n", whoami(&all_scopes())),
            docker(true, true),
        )
        .run();
        assert_eq!(states(&banner)[0], ("wrangler", State::Ok), "{banner}");
        let junk =
            Preflight::new(wrangler("hello", whoami(&all_scopes())), docker(true, true)).run();
        assert_eq!(states(&junk)[0], ("wrangler", State::Unknown), "{junk}");
    }

    #[test]
    fn not_logged_in_and_no_wrangler_block() {
        let not_in = Fake(Box::new(|a| match a {
            ["--version"] => out(true, "4.141.0"),
            _ => out(false, ""),
        }));
        let r = Preflight::new(not_in, docker(true, true)).run();
        assert!(!r.ready());
        assert!(
            r.items
                .iter()
                .any(|i| i.what == "cloudflare login" && i.state == State::Problem)
        );
        // A JSON answer that says loggedIn:false blocks even with exit 0.
        let says_no = Fake(Box::new(|a| match a {
            ["--version"] => out(true, "4.141.0"),
            _ => out(true, "{\"loggedIn\":false}"),
        }));
        assert!(!Preflight::new(says_no, docker(true, true)).run().ready());
        // An answer that does not say it is logged in (no field, or not JSON) is not a login.
        for answer in ["{}", "not json"] {
            let vague = Fake(Box::new(move |a| match a {
                ["--version"] => out(true, "4.141.0"),
                _ => out(true, answer),
            }));
            assert!(
                !Preflight::new(vague, docker(true, true)).run().ready(),
                "{answer}"
            );
        }
        let none = Fake(Box::new(|_| Err(Error::Target("not found".to_string()))));
        let r = Preflight::new(none, docker(true, true)).run();
        assert!(!r.ready());
        assert_eq!(r.items[0].state, State::Problem);
    }

    #[test]
    fn an_api_token_that_lists_no_scopes_is_unknown_not_ok() {
        let api = serde_json::json!({"loggedIn": true, "authType": "API Token"}).to_string();
        let r = Preflight::new(wrangler("4.141.0", api), docker(true, true)).run();
        assert!(r.ready(), "unknown does not block: {r}");
        let scopes = r.items.iter().find(|i| i.what == "token scopes").unwrap();
        assert_eq!(scopes.state, State::Unknown);
    }

    #[test]
    fn docker_missing_or_stopped_blocks_a_container() {
        for (installed, daemon) in [(false, false), (true, false)] {
            let r = Preflight::new(
                wrangler("4.141.0", whoami(&all_scopes())),
                docker(installed, daemon),
            )
            .with_containers()
            .run();
            assert!(!r.ready(), "installed={installed} daemon={daemon}: {r}");
        }
    }

    #[test]
    fn the_report_never_carries_the_person_or_the_account() {
        let some: Vec<&str> = WORKER_SCOPES.to_vec();
        for containers in [false, true] {
            let mut p = Preflight::new(wrangler("4.141.0", whoami(&some)), docker(true, true));
            if containers {
                p = p.with_containers();
            }
            let text = format!("{}{:?}", p.run(), p.run());
            for secret in [EMAIL, ACCOUNT, "abc123"] {
                assert!(!text.contains(secret), "the report leaked a canary");
            }
        }
    }
}
