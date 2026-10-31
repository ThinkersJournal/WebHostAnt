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

    /// The `probe` closure `cloudflare::CloudflareTarget::new` takes: `url_of` maps an app id to its health URL; the closure answers whether it is healthy.
    pub fn into_probe(self, url_of: impl Fn(&str) -> String + 'static) -> impl Fn(&str) -> bool {
        move |app_id| self.check(&url_of(app_id)) == Health::Healthy
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
