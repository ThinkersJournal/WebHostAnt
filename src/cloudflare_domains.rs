//! Cloudflare Workers custom domains over the REST API: list, attach, detach.
//!
//! **Shapes come from Cloudflare's published OpenAPI schema** (`docs/CLOUDFLARE-FACTS.md`), not from a recorded live answer: no test account exists
//! yet (D6). So the operation is `Unverified` until the caller opts in with `CloudflareTarget::trust_operations`. The schema does not say whether
//! attaching an attached hostname is safe to repeat, so [`Domains::attach`] looks first and never overwrites a hostname another Worker holds.

use std::time::Duration;

use httpant_client::Client;
use httpant_client::core::http::Method as HttpMethod;
use serde_json::{Value, json};

use crate::{Error, Renewal, SecretValue, TlsStatus};

/// The REST verbs this module uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Read.
    Get,
    /// Create or replace.
    Put,
    /// Create.
    Post,
    /// Remove.
    Delete,
}

/// One REST call. `path` is relative to the API root (`/accounts/...`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestCall {
    /// The verb.
    pub method: Method,
    /// Path under the API root.
    pub path: String,
    /// Query pairs, not yet encoded.
    pub query: Vec<(String, String)>,
    /// A JSON body, if any.
    pub body: Option<String>,
}

/// What came back: the HTTP status and the body text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestAnswer {
    /// HTTP status.
    pub status: u16,
    /// Body text.
    pub body: String,
}

/// Something that sends a [`RestCall`]. The real one is [`HttpRest`]; tests use a fake.
pub trait Rest {
    /// Send one call.
    fn send(&self, call: &RestCall) -> Result<RestAnswer, Error>;
}

impl Rest for Box<dyn Rest> {
    fn send(&self, call: &RestCall) -> Result<RestAnswer, Error> {
        (**self).send(call)
    }
}

/// A domain Cloudflare reports. Only the fields this crate uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Domain {
    /// Cloudflare's id for the binding.
    pub id: String,
    /// The host name.
    pub hostname: String,
    /// The Worker it points at.
    pub service: String,
}

/// A certificate pack Cloudflare holds for the zone. Only the fields this crate uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CertPack {
    /// The pack's status word (`active`, `pending_validation`, `validation_timed_out`, ...).
    pub status: String,
    /// Host names the pack covers (may include `*.` wildcards).
    pub hosts: Vec<String>,
    /// The authority selected for the order, if stated.
    pub authority: Option<String>,
    /// `(expires_on, issuer)` for each certificate in the pack.
    pub certificates: Vec<(Option<String>, Option<String>)>,
    /// Whether the authority has reported validation errors.
    pub has_validation_errors: bool,
}

impl CertPack {
    fn covers(&self, name: &str) -> bool {
        let name = name.to_ascii_lowercase();
        self.hosts.iter().any(|h| {
            let h = h.to_ascii_lowercase();
            h == name
                || h.strip_prefix("*.").is_some_and(|suffix| {
                    name.strip_suffix(suffix)
                        .and_then(|front| front.strip_suffix('.'))
                        .is_some_and(|label| !label.is_empty() && !label.contains('.'))
                })
        })
    }

    fn timed_out(&self) -> bool {
        self.status.ends_with("_timed_out")
    }
}

/// What the zone's packs say about `names`: the soonest expiry among the active packs that cover them, who issued, and whether a covering pack is stuck.
/// A name no active pack covers is simply absent from the answer (right after a bind that is normal); the renewal is `Unknown` unless a pack says it failed.
pub fn tls_findings(names: &[String], packs: &[CertPack]) -> TlsStatus {
    let covering: Vec<&CertPack> = packs
        .iter()
        .filter(|p| names.iter().any(|n| p.covers(n)))
        .collect();
    let active: Vec<&&CertPack> = covering.iter().filter(|p| p.status == "active").collect();
    // RFC 3339 in UTC with a trailing Z compares correctly as text; any other form is not compared, so no expiry is claimed.
    let dates: Vec<&str> = active
        .iter()
        .flat_map(|p| p.certificates.iter().filter_map(|c| c.0.as_deref()))
        .collect();
    let comparable = !dates.is_empty()
        && dates
            .iter()
            .all(|d| d.ends_with('Z') && d.len() == dates[0].len());
    let not_after = if comparable {
        dates.iter().min().map(|d| d.to_string())
    } else {
        None
    };
    let mut issuers: Vec<String> = active
        .iter()
        .flat_map(|p| {
            p.certificates
                .iter()
                .filter_map(|c| c.1.clone())
                .chain(p.authority.clone())
        })
        .collect();
    issuers.sort();
    issuers.dedup();
    let last_renewal = match covering
        .iter()
        .find(|p| p.timed_out() || p.has_validation_errors)
    {
        Some(p) => Renewal::Failed(format!("the certificate pack is {}", p.status)),
        None => Renewal::Unknown,
    };
    TlsStatus {
        names: names.to_vec(),
        not_after,
        issuer: if issuers.is_empty() {
            None
        } else {
            Some(issuers.join(", "))
        },
        last_renewal,
    }
}

/// An id that goes into a URL path must be plain, or it could add path segments.
pub(crate) fn plain_id(what: &str, id: &str) -> Result<(), Error> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Ok(())
    } else {
        Err(Error::Target(format!("the {what} is not a plain id")))
    }
}

/// Workers custom domains for one account and one zone.
pub struct Domains<R: Rest> {
    rest: R,
    account_id: String,
    zone_id: String,
}

impl<R: Rest> Domains<R> {
    /// `account_id` and `zone_id` must be plain ids (letters, digits, `-`, `_`).
    pub fn new(
        rest: R,
        account_id: impl Into<String>,
        zone_id: impl Into<String>,
    ) -> Result<Self, Error> {
        let (account_id, zone_id) = (account_id.into(), zone_id.into());
        plain_id("account id", &account_id)?;
        plain_id("zone id", &zone_id)?;
        Ok(Self {
            rest,
            account_id,
            zone_id,
        })
    }

    fn base(&self) -> String {
        format!("/accounts/{}/workers/domains", self.account_id)
    }

    /// Send, then insist on a 2xx status and a body that says `success: true`; return the parsed body.
    fn call(&self, call: RestCall) -> Result<Value, Error> {
        let answer = self.rest.send(&call)?;
        let parsed: Value = serde_json::from_str(&answer.body).map_err(|_| {
            Error::Target(format!(
                "Cloudflare answered {} with a body that is not JSON",
                answer.status
            ))
        })?;
        let ok_status = (200..300).contains(&answer.status);
        if !ok_status || parsed.get("success").and_then(Value::as_bool) != Some(true) {
            let first = parsed
                .get("errors")
                .and_then(|e| e.as_array())
                .and_then(|e| e.first());
            let code = first
                .and_then(|e| e.get("code"))
                .and_then(Value::as_i64)
                .map(|c| format!(" [code: {c}]"))
                .unwrap_or_default();
            let message = first
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("no message");
            return Err(Error::Target(format!(
                "Cloudflare refused the domain call (status {}): {message}{code}",
                answer.status
            )));
        }
        Ok(parsed)
    }

    fn domain_of(v: &Value) -> Option<Domain> {
        Some(Domain {
            id: v.get("id")?.as_str()?.to_string(),
            hostname: v.get("hostname")?.as_str()?.to_string(),
            service: v.get("service")?.as_str()?.to_string(),
        })
    }

    /// Every domain, optionally only those of one Worker or one hostname. Refuses (rather than truncates) an answer that says it has more pages.
    pub fn list(
        &self,
        service: Option<&str>,
        hostname: Option<&str>,
    ) -> Result<Vec<Domain>, Error> {
        let mut query = vec![("zone_id".to_string(), self.zone_id.clone())];
        if let Some(s) = service {
            query.push(("service".to_string(), s.to_string()));
        }
        if let Some(h) = hostname {
            query.push(("hostname".to_string(), h.to_string()));
        }
        let parsed = self.call(RestCall {
            method: Method::Get,
            path: self.base(),
            query,
            body: None,
        })?;
        if parsed
            .get("result_info")
            .and_then(|i| i.get("total_pages"))
            .and_then(Value::as_u64)
            .is_some_and(|p| p > 1)
        {
            return Err(Error::Target(
                "Cloudflare reports more than one page of domains; reading only the first would hide some"
                    .to_string(),
            ));
        }
        let items = parsed
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::Target("the domain list has no result array".to_string()))?;
        items
            .iter()
            .map(|v| {
                Self::domain_of(v).ok_or_else(|| {
                    Error::Target("a domain in the list lacks id, hostname or service".to_string())
                })
            })
            .collect()
    }

    /// Point `hostname` at the Worker `service`. Refuses if another Worker already holds it (rule 3: a site that is not ours is never overwritten);
    /// does nothing if this Worker already does. Afterwards the answer must name the same hostname and Worker.
    pub fn attach(&self, hostname: &str, service: &str) -> Result<Domain, Error> {
        if let Some(held) = self
            .list(None, Some(hostname))?
            .into_iter()
            .find(|d| d.hostname == hostname)
        {
            return if held.service == service {
                Ok(held)
            } else {
                Err(Error::Target(
                    "that hostname is already held by another Worker; nothing was changed"
                        .to_string(),
                ))
            };
        }
        let parsed = self.call(RestCall {
            method: Method::Put,
            path: self.base(),
            query: Vec::new(),
            body: Some(
                json!({"hostname": hostname, "service": service, "zone_id": self.zone_id})
                    .to_string(),
            ),
        })?;
        let got = parsed
            .get("result")
            .and_then(Self::domain_of)
            .ok_or_else(|| Error::Target("the attach answer has no domain".to_string()))?;
        if got.hostname != hostname || got.service != service {
            return Err(Error::Target(
                "Cloudflare attached something other than what was asked".to_string(),
            ));
        }
        Ok(got)
    }

    /// The zone's certificate packs (`GET /zones/{zone_id}/ssl/certificate_packs`). Refuses an answer that says it has more pages.
    pub fn certificate_packs(&self) -> Result<Vec<CertPack>, Error> {
        let parsed = self.call(RestCall {
            method: Method::Get,
            path: format!("/zones/{}/ssl/certificate_packs", self.zone_id),
            query: Vec::new(),
            body: None,
        })?;
        if parsed
            .get("result_info")
            .and_then(|i| i.get("total_pages"))
            .and_then(Value::as_u64)
            .is_some_and(|p| p > 1)
        {
            return Err(Error::Target(
                "Cloudflare reports more than one page of certificate packs; reading only the first would hide some"
                    .to_string(),
            ));
        }
        let items = parsed
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                Error::Target("the certificate pack list has no result array".to_string())
            })?;
        let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
        items
            .iter()
            .map(|p| {
                let status = text(p, "status")
                    .ok_or_else(|| Error::Target("a certificate pack has no status".to_string()))?;
                Ok(CertPack {
                    status,
                    hosts: p
                        .get("hosts")
                        .and_then(Value::as_array)
                        .map(|h| {
                            h.iter()
                                .filter_map(|x| x.as_str().map(str::to_string))
                                .collect()
                        })
                        .unwrap_or_default(),
                    authority: text(p, "certificate_authority"),
                    certificates: p
                        .get("certificates")
                        .and_then(Value::as_array)
                        .map(|c| {
                            c.iter()
                                .map(|x| (text(x, "expires_on"), text(x, "issuer")))
                                .collect()
                        })
                        .unwrap_or_default(),
                    has_validation_errors: p
                        .get("validation_errors")
                        .and_then(Value::as_array)
                        .is_some_and(|e| !e.is_empty()),
                })
            })
            .collect()
    }

    /// Remove a binding by its Cloudflare id.
    pub fn detach(&self, domain_id: &str) -> Result<(), Error> {
        plain_id("domain id", domain_id)?;
        self.call(RestCall {
            method: Method::Delete,
            path: format!("{}/{}", self.base(), domain_id),
            query: Vec::new(),
            body: None,
        })
        .map(|_| ())
    }
}

/// The real transport: HTTPS to Cloudflare through HTTPAnt's client, redirects not followed, the API token in memory only.
pub struct HttpRest {
    base: String,
    token: SecretValue,
    allow_private: bool,
}

impl HttpRest {
    /// Talk to `https://api.cloudflare.com/client/v4` with this API token.
    pub fn new(token: SecretValue) -> Self {
        Self::with_base("https://api.cloudflare.com/client/v4", token)
    }

    /// Talk to another root (a test server). Private addresses and plain `http` stay refused unless [`HttpRest::allow_private`] is set.
    pub fn with_base(base: impl Into<String>, token: SecretValue) -> Self {
        Self {
            base: base.into(),
            token,
            allow_private: false,
        }
    }

    /// Opt in to private addresses and plain `http` (a local test server). Off by default.
    pub fn allow_private(mut self, allow: bool) -> Self {
        self.allow_private = allow;
        self
    }
}

impl std::fmt::Debug for HttpRest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpRest")
            .field("base", &self.base)
            .field("token", &"<redacted>")
            .finish()
    }
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

impl Rest for HttpRest {
    fn send(&self, call: &RestCall) -> Result<RestAnswer, Error> {
        if !self.allow_private && !self.base.to_ascii_lowercase().starts_with("https://") {
            return Err(Error::Target("the API root must be https".to_string()));
        }
        let mut url = format!("{}{}", self.base, call.path);
        for (i, (k, v)) in call.query.iter().enumerate() {
            url.push(if i == 0 { '?' } else { '&' });
            url.push_str(&format!("{}={}", encode(k), encode(v)));
        }
        let unreachable = |what: &str| Error::Target(format!("could not reach Cloudflare: {what}"));
        let timeout = Duration::from_secs(30);
        let client = Client::builder()
            .refuse_private_addresses(!self.allow_private)
            .connect_timeout(timeout)
            .timeout(timeout)
            .build()
            .map_err(|_| unreachable("client"))?;
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| unreachable("runtime"))?;
        let method = match call.method {
            Method::Get => HttpMethod::GET,
            Method::Put => HttpMethod::PUT,
            Method::Post => HttpMethod::POST,
            Method::Delete => HttpMethod::DELETE,
        };
        let mut request = client
            .request(method, &url)
            .bearer(self.token.expose())
            .header("content-type", "application/json");
        if let Some(body) = &call.body {
            request = request.body(body.clone());
        }
        runtime.block_on(async {
            let response = request.send().await.map_err(|_| unreachable("send"))?;
            let status = response.status().as_u16();
            let body = response
                .text_limited(1 << 20)
                .await
                .map_err(|_| unreachable("read"))?;
            Ok(RestAnswer { status, body })
        })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    /// A pretend Cloudflare that keeps domains and answers in the documented shapes.
    #[derive(Default)]
    pub(crate) struct FakeRest {
        pub(crate) domains: RefCell<Vec<(String, String, String)>>, // id, hostname, service
        calls: RefCell<Vec<RestCall>>,
        force: RefCell<Option<RestAnswer>>,
        pages: u64,
    }

    fn envelope(result: Value) -> String {
        json!({"success": true, "errors": [], "messages": [], "result": result}).to_string()
    }

    fn dom(id: &str, h: &str, s: &str) -> Value {
        json!({"id": id, "cert_id": "c", "enabled": true, "hostname": h, "previews_enabled": false,
               "service": s, "zone_id": "z", "zone_name": "example.org"})
    }

    impl Rest for FakeRest {
        fn send(&self, call: &RestCall) -> Result<RestAnswer, Error> {
            self.calls.borrow_mut().push(call.clone());
            if let Some(a) = self.force.borrow().clone() {
                return Ok(a);
            }
            let ok = |body: String| Ok(RestAnswer { status: 200, body });
            if call.path.ends_with("/ssl/certificate_packs") {
                return ok(envelope(json!([{
                    "id": "p1", "status": "active", "type": "universal", "hosts": ["example.org", "*.example.org"],
                    "certificate_authority": "lets_encrypt", "validation_errors": [],
                    "certificates": [{"id": "c1", "expires_on": "2027-02-01T00:00:00Z", "issuer": "Lets Encrypt", "hosts": ["example.org"], "status": "active"}]
                }])));
            }
            match call.method {
                Method::Get => {
                    let want = |k: &str| {
                        call.query
                            .iter()
                            .find(|(n, _)| n == k)
                            .map(|(_, v)| v.clone())
                    };
                    let rows: Vec<Value> = self
                        .domains
                        .borrow()
                        .iter()
                        .filter(|(_, h, s)| {
                            want("hostname").is_none_or(|w| &w == h)
                                && want("service").is_none_or(|w| &w == s)
                        })
                        .map(|(i, h, s)| dom(i, h, s))
                        .collect();
                    let mut v: Value = serde_json::from_str(&envelope(Value::Array(rows))).unwrap();
                    v["result_info"] = json!({"count": 1, "page": 1, "per_page": 20,
                        "total_count": 1, "total_pages": self.pages.max(1)});
                    ok(v.to_string())
                }
                Method::Put => {
                    let b: Value = serde_json::from_str(call.body.as_ref().unwrap()).unwrap();
                    let (h, s) = (
                        b["hostname"].as_str().unwrap().to_string(),
                        b["service"].as_str().unwrap().to_string(),
                    );
                    let id = format!("id{}", self.domains.borrow().len() + 1);
                    self.domains
                        .borrow_mut()
                        .push((id.clone(), h.clone(), s.clone()));
                    ok(envelope(dom(&id, &h, &s)))
                }
                Method::Post => panic!("the fake Cloudflare takes no POST"),
                Method::Delete => {
                    let id = call.path.rsplit('/').next().unwrap().to_string();
                    self.domains.borrow_mut().retain(|(i, _, _)| *i != id);
                    ok(json!({"success": true, "errors": [], "messages": []}).to_string())
                }
            }
        }
    }

    fn domains(f: FakeRest) -> Domains<FakeRest> {
        Domains::new(f, "acct1", "zone1").unwrap()
    }

    const DENIED: &str = r#"{"success":false,"errors":[{"code":10000,"message":"Authentication error"}],"messages":[],"result":null}"#;

    #[test]
    fn attach_then_list_shows_the_domain_and_detach_removes_it() {
        let d = domains(FakeRest::default());
        let got = d.attach("app.example.org", "my-app").unwrap();
        assert_eq!(got.hostname, "app.example.org");
        assert_eq!(d.list(Some("my-app"), None).unwrap().len(), 1);
        assert!(d.list(Some("other"), None).unwrap().is_empty()); // control: the filter filters
        d.detach(&got.id).unwrap();
        assert!(d.list(Some("my-app"), None).unwrap().is_empty());
    }

    #[test]
    fn the_calls_use_the_documented_paths_and_the_zone() {
        let d = domains(FakeRest::default());
        d.attach("app.example.org", "my-app").unwrap();
        let calls = d.rest.calls.borrow();
        assert_eq!(calls[0].path, "/accounts/acct1/workers/domains");
        assert!(calls[0].query.contains(&("zone_id".into(), "zone1".into())));
        assert_eq!(calls[1].method, Method::Put);
        let body: Value = serde_json::from_str(calls[1].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["zone_id"], "zone1");
    }

    #[test]
    fn a_hostname_held_by_another_worker_is_never_overwritten() {
        let f = FakeRest::default();
        f.domains.borrow_mut().push((
            "id9".into(),
            "app.example.org".into(),
            "someone-else".into(),
        ));
        let d = domains(f);
        assert!(d.attach("app.example.org", "my-app").is_err());
        assert!(
            d.rest
                .calls
                .borrow()
                .iter()
                .all(|c| c.method != Method::Put),
            "no write may be sent"
        );
    }

    #[test]
    fn attaching_what_is_already_ours_sends_no_write() {
        let f = FakeRest::default();
        f.domains
            .borrow_mut()
            .push(("id1".into(), "app.example.org".into(), "my-app".into()));
        let d = domains(f);
        d.attach("app.example.org", "my-app").unwrap();
        assert!(
            d.rest
                .calls
                .borrow()
                .iter()
                .all(|c| c.method == Method::Get)
        );
    }

    #[test]
    fn success_false_or_a_bad_status_or_a_non_json_body_is_an_error() {
        for answer in [
            RestAnswer {
                status: 200,
                body: DENIED.into(),
            },
            RestAnswer {
                status: 403,
                body: envelope(json!([])),
            },
            RestAnswer {
                status: 200,
                body: "<html>".into(),
            },
        ] {
            let f = FakeRest::default();
            *f.force.borrow_mut() = Some(answer);
            assert!(domains(f).list(None, None).is_err());
        }
        // Control: the same fake with no forced answer succeeds.
        assert!(domains(FakeRest::default()).list(None, None).is_ok());
    }

    #[test]
    fn the_error_names_the_code_and_message() {
        let f = FakeRest::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 403,
            body: DENIED.into(),
        });
        let e = domains(f).list(None, None).unwrap_err().to_string();
        assert!(e.contains("10000") && e.contains("Authentication error"));
    }

    #[test]
    fn more_than_one_page_is_refused_not_truncated() {
        let f = FakeRest {
            pages: 2,
            ..FakeRest::default()
        };
        assert!(domains(f).list(None, None).is_err());
    }

    #[test]
    fn an_answer_for_a_different_hostname_is_not_accepted() {
        struct Liar(FakeRest);
        impl Rest for Liar {
            fn send(&self, c: &RestCall) -> Result<RestAnswer, Error> {
                if c.method == Method::Put {
                    return Ok(RestAnswer {
                        status: 200,
                        body: envelope(dom("id1", "evil.example.org", "my-app")),
                    });
                }
                self.0.send(c)
            }
        }
        let d = Domains::new(Liar(FakeRest::default()), "acct1", "zone1").unwrap();
        assert!(d.attach("app.example.org", "my-app").is_err());
    }

    fn pack(status: &str, hosts: &[&str], expires: &[&str]) -> CertPack {
        CertPack {
            status: status.into(),
            hosts: hosts.iter().map(|h| h.to_string()).collect(),
            authority: Some("lets_encrypt".into()),
            certificates: expires
                .iter()
                .map(|e| (Some(e.to_string()), Some("Lets Encrypt".into())))
                .collect(),
            has_validation_errors: false,
        }
    }

    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn the_soonest_expiry_of_the_active_covering_packs_is_reported() {
        let packs = [
            pack(
                "active",
                &["example.org", "*.example.org"],
                &["2027-03-01T00:00:00Z", "2027-02-01T00:00:00Z"],
            ),
            // Does not cover the name: must not count.
            pack("active", &["other.net"], &["2026-11-01T00:00:00Z"]),
        ];
        let s = tls_findings(&names(&["app.example.org"]), &packs);
        assert_eq!(s.not_after.as_deref(), Some("2027-02-01T00:00:00Z"));
        assert!(s.issuer.as_deref().unwrap().contains("Lets Encrypt"));
        assert_eq!(s.last_renewal, Renewal::Unknown);
    }

    #[test]
    fn a_wildcard_covers_one_label_only() {
        let packs = [pack(
            "active",
            &["*.example.org"],
            &["2027-01-01T00:00:00Z"],
        )];
        assert!(
            tls_findings(&names(&["a.example.org"]), &packs)
                .not_after
                .is_some()
        );
        assert!(
            tls_findings(&names(&["a.b.example.org"]), &packs)
                .not_after
                .is_none()
        );
        assert!(
            tls_findings(&names(&["example.org"]), &packs)
                .not_after
                .is_none()
        );
        assert!(
            tls_findings(&names(&["evilexample.org"]), &packs)
                .not_after
                .is_none()
        );
    }

    #[test]
    fn a_pending_pack_claims_no_expiry_and_a_timed_out_one_is_a_failed_renewal() {
        let pending = [pack(
            "pending_validation",
            &["app.example.org"],
            &["2027-01-01T00:00:00Z"],
        )];
        let s = tls_findings(&names(&["app.example.org"]), &pending);
        assert_eq!(s.not_after, None);
        assert_eq!(s.last_renewal, Renewal::Unknown);
        let stuck = [pack("validation_timed_out", &["app.example.org"], &[])];
        assert!(matches!(
            tls_findings(&names(&["app.example.org"]), &stuck).last_renewal,
            Renewal::Failed(_)
        ));
    }

    #[test]
    fn dates_in_a_form_that_cannot_be_compared_claim_no_expiry() {
        let packs = [pack(
            "active",
            &["app.example.org"],
            &["2027-01-01T00:00:00+02:00", "2027-01-01T00:00:00Z"],
        )];
        assert_eq!(
            tls_findings(&names(&["app.example.org"]), &packs).not_after,
            None
        );
    }

    #[test]
    fn certificate_packs_are_parsed_from_the_documented_shape() {
        let f = FakeRest::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 200,
            body: envelope(json!([{
                "id": "p1", "status": "active", "type": "universal",
                "hosts": ["example.org", "*.example.org"], "certificate_authority": "lets_encrypt",
                "validation_errors": [],
                "certificates": [{"id": "c1", "expires_on": "2027-02-01T00:00:00Z", "issuer": "Lets Encrypt", "hosts": ["example.org"], "status": "active"}]
            }])),
        });
        let packs = domains(f).certificate_packs().unwrap();
        assert_eq!(packs.len(), 1);
        assert_eq!(packs[0].status, "active");
        assert_eq!(
            packs[0].certificates[0].0.as_deref(),
            Some("2027-02-01T00:00:00Z")
        );
        assert!(!packs[0].has_validation_errors);
    }

    #[test]
    fn certificate_pack_errors_and_extra_pages_are_refused() {
        let f = FakeRest::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 403,
            body: DENIED.into(),
        });
        assert!(domains(f).certificate_packs().is_err());
        let f = FakeRest::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 200,
            body: json!({"success": true, "errors": [], "messages": [], "result": [], "result_info": {"total_pages": 3}}).to_string(),
        });
        assert!(domains(f).certificate_packs().is_err());
    }

    #[test]
    fn ids_that_could_add_path_segments_are_refused() {
        assert!(Domains::new(FakeRest::default(), "a/../b", "z").is_err());
        assert!(Domains::new(FakeRest::default(), "a", "").is_err());
        assert!(domains(FakeRest::default()).detach("x/../y").is_err());
    }

    // ---- the real transport, against a local server ----

    /// Serve one canned reply; returns the port and the request text it saw.
    fn serve_once(reply: String) -> (u16, std::sync::mpsc::Receiver<String>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if let Ok((mut s, _)) = l.accept() {
                let mut buf = [0u8; 4096];
                let n = s.read(&mut buf).unwrap_or(0);
                let _ = tx.send(String::from_utf8_lossy(&buf[..n]).to_string());
                let _ = s.write_all(reply.as_bytes());
            }
        });
        (port, rx)
    }

    fn http_reply(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn get(path: &str, query: Vec<(String, String)>) -> RestCall {
        RestCall {
            method: Method::Get,
            path: path.into(),
            query,
            body: None,
        }
    }

    #[test]
    fn http_rest_sends_a_bearer_token_encodes_the_query_and_returns_the_body() {
        let (port, seen) = serve_once(http_reply(&envelope(json!([]))));
        let rest = HttpRest::with_base(
            format!("http://127.0.0.1:{port}/client/v4"),
            SecretValue::new("tok-canary-123"),
        )
        .allow_private(true);
        let answer = rest
            .send(&get(
                "/accounts/a/workers/domains",
                vec![("hostname".into(), "a b&c".into())],
            ))
            .unwrap();
        assert_eq!(answer.status, 200);
        let request = seen.recv().unwrap().to_lowercase();
        assert!(
            request.starts_with("get /client/v4/accounts/a/workers/domains?hostname=a%20b%26c ")
        );
        assert!(request.contains("authorization: bearer tok-canary-123"));
    }

    #[test]
    fn http_rest_refuses_plain_http_and_private_addresses_by_default_and_hides_its_token() {
        let rest = HttpRest::with_base(
            "http://127.0.0.1:1/client/v4",
            SecretValue::new("tok-canary-123"),
        );
        let e = rest.send(&get("/x", vec![])).unwrap_err();
        assert!(!format!("{e} {e:?} {rest:?}").contains("tok-canary-123"));
        // Over https the scheme rule passes, so only the address guard can refuse a loopback address.
        let private = HttpRest::with_base(
            "https://127.0.0.1:1/client/v4",
            SecretValue::new("tok-canary-123"),
        );
        let e = private.send(&get("/x", vec![])).unwrap_err();
        assert!(!format!("{e:?}").contains("tok-canary-123"));
    }

    #[test]
    fn http_rest_does_not_follow_a_redirect() {
        let (port, _) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_string(),
        );
        let rest = HttpRest::with_base(
            format!("http://127.0.0.1:{port}/client/v4"),
            SecretValue::new("t"),
        )
        .allow_private(true);
        let a = rest.send(&get("/x", vec![]));
        assert!(matches!(a, Ok(RestAnswer { status: 302, .. })));
    }
}
