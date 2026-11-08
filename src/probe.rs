//! A real health probe: one `GET` of the deployed app's health path, through HTTPAnt's client.
//!
//! Redirects are never followed (an app that answers its health path with a redirect to a login page is not healthy; the `3xx` is reported), and
//! the private-address guard is on unless the operator opts out (CONSUMER-CONTRACT section 3, rule 4). A probe that is not allowed to reach a
//! private address also insists on `https`.

use std::time::Duration;

use httpant_client::Client;
use httpant_client::core::Error as HttpError;

/// What one probe saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    /// The health path answered `200`.
    Healthy,
    /// It answered, but not `200` (a redirect is reported here as its own `3xx`, never followed).
    Status(u16),
    /// Not asked: the address is private, loopback or link-local, or the URL is not `https`, and the operator did not opt in.
    Refused,
    /// No answer: could not connect, TLS failed, or it timed out.
    Unreachable,
}

/// How long to keep asking, and how often. The default is Humboldt's 2026-11-08 demo: a container needs minutes before `/health` answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WaitPolicy {
    /// Stop asking once this much time has passed.
    pub deadline: Duration,
    /// Time between two questions.
    pub interval: Duration,
}

impl Default for WaitPolicy {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(600),
            interval: Duration::from_secs(15),
        }
    }
}

/// What a wait saw: every answer, in order, so a failure says what the app said and not only that it never came up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Waited {
    /// True when the last answer was [`Health::Healthy`].
    pub healthy: bool,
    /// Every answer, first to last (at least one).
    pub answers: Vec<Health>,
}

/// Ask until healthy or out of time. `check` asks once; `sleep` waits; `elapsed` is the time since the wait began. Asks at least once. Stops at
/// once on `Healthy`, and on `Refused` (the operator's guard refused the address, and asking again cannot change that). Never sleeps past the deadline.
pub fn wait_until_healthy(
    policy: WaitPolicy,
    mut check: impl FnMut() -> Health,
    mut sleep: impl FnMut(Duration),
    mut elapsed: impl FnMut() -> Duration,
) -> Waited {
    let mut answers = Vec::new();
    loop {
        let answer = check();
        let stop = matches!(answer, Health::Healthy | Health::Refused);
        answers.push(answer);
        let spent = elapsed();
        if stop || spent >= policy.deadline {
            break;
        }
        sleep(policy.interval.min(policy.deadline - spent));
    }
    Waited {
        healthy: answers.last() == Some(&Health::Healthy),
        answers,
    }
}

/// How to probe. The default refuses private addresses and plain `http`.
#[derive(Debug, Clone)]
pub struct HealthProbe {
    allow_private: bool,
    timeout: Duration,
}

impl Default for HealthProbe {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthProbe {
    /// Refuse private addresses and plain `http`; give up after 10 seconds.
    pub fn new() -> Self {
        Self {
            allow_private: false,
            timeout: Duration::from_secs(10),
        }
    }

    /// Opt in to private, loopback and link-local addresses and plain `http` (local development). Off by default.
    pub fn allow_private(mut self, allow: bool) -> Self {
        self.allow_private = allow;
        self
    }

    /// Give up after this long (connect and response head each).
    pub fn timeout(mut self, limit: Duration) -> Self {
        self.timeout = limit;
        self
    }

    /// Probe `url` once.
    pub fn check(&self, url: &str) -> Health {
        if !self.allow_private
            && !url
                .get(..8)
                .is_some_and(|s| s.eq_ignore_ascii_case("https://"))
        {
            return Health::Refused;
        }
        let Ok(client) = Client::builder()
            .refuse_private_addresses(!self.allow_private)
            .connect_timeout(self.timeout)
            .timeout(self.timeout)
            .build()
        else {
            return Health::Unreachable;
        };
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            return Health::Unreachable;
        };
        match runtime.block_on(client.get(url).send()) {
            Ok(response) => match response.status().as_u16() {
                200 => Health::Healthy,
                other => Health::Status(other),
            },
            Err(HttpError::PrivateAddress { .. }) => Health::Refused,
            Err(_) => Health::Unreachable,
        }
    }

    /// Probe `url` until it is healthy or the policy runs out (see [`wait_until_healthy`]).
    pub fn wait(&self, url: &str, policy: WaitPolicy) -> Waited {
        let began = std::time::Instant::now();
        wait_until_healthy(
            policy,
            || self.check(url),
            std::thread::sleep,
            || began.elapsed(),
        )
    }

    /// Like [`HealthProbe::into_probe`], but waiting: use it where a deploy has just happened, never for planning (a down app would make every plan
    /// take the whole deadline).
    pub fn into_waiting_probe(
        self,
        policy: WaitPolicy,
        url_of: impl Fn(&str) -> String + 'static,
    ) -> impl Fn(&str) -> bool {
        move |app_id| self.wait(&url_of(app_id), policy).healthy
    }

    /// The `probe` closure `cloudflare::CloudflareTarget::new` takes: `url_of` maps an app id to its health URL; the closure answers whether it is healthy.
    pub fn into_probe(self, url_of: impl Fn(&str) -> String + 'static) -> impl Fn(&str) -> bool {
        move |app_id| self.check(&url_of(app_id)) == Health::Healthy
    }
}

#[cfg(test)]
mod wait_tests {
    use super::*;
    use std::cell::Cell;

    const POLICY: WaitPolicy = WaitPolicy {
        deadline: Duration::from_secs(60),
        interval: Duration::from_secs(15),
    };

    /// Runs `wait_until_healthy` on a fake clock that only `sleep` advances; `script` gives each answer; returns the wait and the sleeps taken.
    fn run(policy: WaitPolicy, script: &[Health]) -> (Waited, Vec<Duration>) {
        let clock = Cell::new(Duration::ZERO);
        let asked = Cell::new(0usize);
        let mut sleeps = Vec::new();
        let w = wait_until_healthy(
            policy,
            || {
                let i = asked.get();
                asked.set(i + 1);
                script.get(i).cloned().unwrap_or(Health::Unreachable)
            },
            |d| {
                sleeps.push(d);
                clock.set(clock.get() + d);
            },
            || clock.get(),
        );
        (w, sleeps)
    }

    #[test]
    fn a_container_that_comes_up_late_is_waited_for_and_every_answer_is_kept() {
        let (w, sleeps) = run(
            POLICY,
            &[
                Health::Unreachable,
                Health::Status(503),
                Health::Healthy,
                Health::Unreachable,
            ],
        );
        assert!(w.healthy);
        assert_eq!(
            w.answers,
            [Health::Unreachable, Health::Status(503), Health::Healthy]
        );
        // It stops the moment it is healthy: two sleeps, none after the success.
        assert_eq!(sleeps.len(), 2);
    }

    #[test]
    fn it_gives_up_at_the_deadline_and_never_sleeps_past_it() {
        let (w, sleeps) = run(POLICY, &[]);
        assert!(!w.healthy);
        assert_eq!(w.answers.len(), 5, "at 0, 15, 30, 45 and 60 seconds");
        assert_eq!(sleeps.iter().sum::<Duration>(), POLICY.deadline);
        let odd = WaitPolicy {
            deadline: Duration::from_secs(20),
            interval: Duration::from_secs(15),
        };
        let (_, sleeps) = run(odd, &[]);
        assert_eq!(sleeps.iter().sum::<Duration>(), odd.deadline);
    }

    #[test]
    fn a_refusal_is_not_retried() {
        let (w, sleeps) = run(POLICY, &[Health::Refused, Health::Healthy]);
        assert!(!w.healthy);
        assert_eq!(w.answers, [Health::Refused]);
        assert!(sleeps.is_empty());
    }

    #[test]
    fn it_always_asks_once_even_with_no_time() {
        let none = WaitPolicy {
            deadline: Duration::ZERO,
            interval: Duration::from_secs(15),
        };
        let (w, sleeps) = run(none, &[Health::Healthy]);
        assert!(w.healthy);
        assert_eq!(w.answers.len(), 1);
        assert!(sleeps.is_empty());
        assert_eq!(run(none, &[Health::Status(500)]).0.answers.len(), 1);
    }

    #[test]
    fn the_real_wait_reaches_a_server_and_reports_a_dead_port() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for s in listener.incoming() {
                let Ok(mut s) = s else { return };
                let mut b = [0u8; 512];
                let _ = s.read(&mut b);
                let _ = s.write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                );
            }
        });
        let quick = WaitPolicy {
            deadline: Duration::from_millis(300),
            interval: Duration::from_millis(50),
        };
        let probe = HealthProbe::new()
            .allow_private(true)
            .timeout(Duration::from_secs(2));
        assert!(
            probe
                .wait(&format!("http://127.0.0.1:{port}/health"), quick)
                .healthy
        );
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let w = probe.wait(&format!("http://127.0.0.1:{dead}/health"), quick);
        assert!(!w.healthy);
        // Windows can take seconds to refuse a connection, so only the shape is pinned; the retry count is covered on the fake clock above.
        assert!(
            w.answers.iter().all(|a| *a == Health::Unreachable),
            "{:?}",
            w.answers
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// Serve `reply` to every connection on a loopback port; returns the port.
    fn serve(reply: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { return };
                let mut buf = [0u8; 1024];
                let _ = s.read(&mut buf);
                let _ = s.write_all(reply.as_bytes());
            }
        });
        port
    }

    const OK: &str =
        "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"status\":\"ok\"}";
    const REDIRECT: &str =
        "HTTP/1.1 302 Found\r\nLocation: /login\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
    const BROKEN: &str =
        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    fn local(port: u16) -> String {
        format!("http://127.0.0.1:{port}/health")
    }

    #[test]
    fn a_200_is_healthy() {
        let url = local(serve(OK));
        assert_eq!(
            HealthProbe::new().allow_private(true).check(&url),
            Health::Healthy
        );
    }

    #[test]
    fn a_redirect_is_seen_not_followed() {
        let url = local(serve(REDIRECT));
        assert_eq!(
            HealthProbe::new().allow_private(true).check(&url),
            Health::Status(302)
        );
    }

    #[test]
    fn a_500_is_not_healthy() {
        let url = local(serve(BROKEN));
        assert_eq!(
            HealthProbe::new().allow_private(true).check(&url),
            Health::Status(500)
        );
    }

    #[test]
    fn nothing_listening_is_unreachable() {
        let port = {
            let l = TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let probe = HealthProbe::new()
            .allow_private(true)
            .timeout(Duration::from_secs(2));
        assert_eq!(probe.check(&local(port)), Health::Unreachable);
    }

    #[test]
    fn the_default_refuses_a_loopback_address_that_would_answer() {
        // Positive control: the same server, opted in, is Healthy (a_200_is_healthy). Here the guard must stop it, over https so only the
        // address guard (not the scheme rule) is under test.
        let port = serve(OK);
        let url = format!("https://127.0.0.1:{port}/health");
        assert_eq!(HealthProbe::new().check(&url), Health::Refused);
    }

    #[test]
    fn the_default_refuses_plain_http() {
        let url = local(serve(OK));
        assert_eq!(HealthProbe::new().check(&url), Health::Refused);
    }

    #[test]
    fn the_cloud_metadata_address_is_refused() {
        assert_eq!(
            HealthProbe::new().check("https://169.254.169.254/health"),
            Health::Refused
        );
    }

    #[test]
    fn into_probe_maps_an_app_id_to_its_url() {
        let port = serve(OK);
        let probe = HealthProbe::new()
            .allow_private(true)
            .into_probe(move |app| format!("http://127.0.0.1:{port}/{app}"));
        assert!(probe("my-app"));
        let down = HealthProbe::new()
            .allow_private(true)
            .timeout(Duration::from_secs(2))
            .into_probe(|_| "http://127.0.0.1:1/health".to_string());
        assert!(!down("my-app"));
    }
}
