//! Render's REST API: services, variables, secret files, custom domains and deploys.
//!
//! **Shapes come from Render's published OpenAPI file** (`https://api-docs.render.com/openapi/render-public-api-1.json`, read 2026-10-31; see
//! `docs/SECOND-HOST.md`), not from a recorded live answer: no Render account exists for us yet. Tests use a fake that answers in those shapes.
//! Nothing here is wired into a [`crate::Target`] yet; that is the next slice.
//!
//! Render's answers are plain JSON (no `success`/`result` envelope, unlike Cloudflare's); an error is `{id, message, code}` with a 4xx/5xx status.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::Error;
use crate::cloudflare_domains::{Method, Rest, RestCall, plain_id};

/// Plans the schema lists for a web service (read from `POST /services`, `serviceDetails.plan`). Anything else is refused before a call is made.
pub const WEB_SERVICE_PLANS: &[&str] = &[
    "starter",
    "starter_plus",
    "standard",
    "standard_plus",
    "pro",
    "pro_plus",
    "pro_max",
    "pro_ultra",
    "free",
    "custom",
];

/// A page this size is refused as possibly incomplete: the answer does not say whether more follow, so reading only this page could hide some.
const PAGE: usize = 100;

/// A service Render holds. Only the fields this crate uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    /// Render's id (`srv-...`).
    pub id: String,
    /// The service's name.
    pub name: String,
    /// `web_service`, `private_service`, ...
    pub kind: String,
    /// The owning workspace.
    pub owner_id: String,
    /// Its public URL, if Render reports one.
    pub url: Option<String>,
}

/// Where a deploy is. Render's words, plus `Unknown` for a word this crate was not written against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeployStatus {
    /// Accepted, not running yet.
    Pending,
    /// Building, updating or running its pre-deploy command.
    InProgress,
    /// The new version is serving.
    Live,
    /// Replaced by a later deploy.
    Deactivated,
    /// It failed (`build_failed`, `update_failed`, `pre_deploy_failed`) or was cancelled. The word is kept.
    Failed(String),
    /// A status this crate does not know. Never treated as live.
    Unknown(String),
}

impl DeployStatus {
    fn parse(word: &str) -> Self {
        match word {
            "created" | "queued" => Self::Pending,
            "build_in_progress" | "update_in_progress" | "pre_deploy_in_progress" => {
                Self::InProgress
            }
            "live" => Self::Live,
            "deactivated" => Self::Deactivated,
            "build_failed" | "update_failed" | "pre_deploy_failed" | "canceled" => {
                Self::Failed(word.to_string())
            }
            other => Self::Unknown(other.to_string()),
        }
    }

    /// Whether the deploy is finished and serving.
    pub fn is_live(&self) -> bool {
        *self == Self::Live
    }

    /// Whether the deploy has ended without serving.
    pub fn is_failed(&self) -> bool {
        matches!(self, Self::Failed(_))
    }
}

/// A deploy Render reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deploy {
    /// Render's id (`dep-...`).
    pub id: String,
    /// Where it is.
    pub status: DeployStatus,
}

/// A custom domain Render holds for a service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomDomain {
    /// Render's id.
    pub id: String,
    /// The host name.
    pub name: String,
    /// Whether Render has verified the DNS records.
    pub verified: bool,
}

/// Render's API for one workspace (`owner_id`).
pub struct RenderApi<R: Rest> {
    rest: R,
    owner_id: String,
}

impl<R: Rest> RenderApi<R> {
    /// `owner_id` is the workspace id (`tea-...` or `usr-...`); it must be a plain id.
    pub fn new(rest: R, owner_id: impl Into<String>) -> Result<Self, Error> {
        let owner_id = owner_id.into();
        plain_id("workspace id", &owner_id)?;
        Ok(Self { rest, owner_id })
    }

    /// Send; a non-2xx status is an error carrying Render's `message` and `code` and never the request. Returns the parsed body (`Null` if empty).
    fn call(&self, call: RestCall) -> Result<(u16, Value), Error> {
        let answer = self.rest.send(&call)?;
        let body: Value = if answer.body.trim().is_empty() {
            Value::Null
        } else {
            serde_json::from_str(&answer.body).map_err(|_| {
                Error::Target(format!(
                    "Render answered {} with a body that is not JSON",
                    answer.status
                ))
            })?
        };
        if !(200..300).contains(&answer.status) {
            let message = body
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no message");
            let code = body
                .get("code")
                .and_then(Value::as_str)
                .map(|c| format!(" [code: {c}]"))
                .unwrap_or_default();
            return Err(Error::Target(format!(
                "Render refused the call (status {}): {message}{code}",
                answer.status
            )));
        }
        Ok((answer.status, body))
    }

    fn get(&self, path: String, query: Vec<(String, String)>) -> Result<Value, Error> {
        self.call(RestCall {
            method: Method::Get,
            path,
            query,
            body: None,
        })
        .map(|(_, b)| b)
    }

    fn service_of(v: &Value) -> Option<Service> {
        let s = v.get("service").unwrap_or(v);
        let text = |k: &str| s.get(k).and_then(Value::as_str).map(str::to_string);
        Some(Service {
            id: text("id")?,
            name: text("name")?,
            kind: text("type")?,
            owner_id: text("ownerId")?,
            url: s
                .get("serviceDetails")
                .and_then(|d| d.get("url"))
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    fn deploy_of(v: &Value) -> Option<Deploy> {
        let d = v.get("deploy").unwrap_or(v);
        Some(Deploy {
            id: d.get("id")?.as_str()?.to_string(),
            status: DeployStatus::parse(d.get("status")?.as_str()?),
        })
    }

    /// The service with exactly this name in this workspace, if any. The API's `name` filter may match more than the exact name, so the answer is
    /// filtered here; a full page is refused rather than trusted.
    pub fn find_service(&self, name: &str) -> Result<Option<Service>, Error> {
        let body = self.get(
            "/services".to_string(),
            vec![
                ("name".into(), name.into()),
                ("ownerId".into(), self.owner_id.clone()),
                ("limit".into(), PAGE.to_string()),
            ],
        )?;
        let items = body
            .as_array()
            .ok_or_else(|| Error::Target("the service list is not an array".to_string()))?;
        if items.len() >= PAGE {
            return Err(Error::Target(
                "Render returned a full page of services; there may be more than were read"
                    .to_string(),
            ));
        }
        let mut found = None;
        for item in items {
            let s = Self::service_of(item).ok_or_else(|| {
                Error::Target("a service in the list lacks id, name, type or ownerId".to_string())
            })?;
            if s.name == name && s.owner_id == self.owner_id {
                found = Some(s);
            }
        }
        Ok(found)
    }

    /// Create a web service that runs a container image, with plain settings as variables. Refuses a plan the schema does not list. Secrets do not go
    /// here: set them afterwards with [`RenderApi::put_env_var`] or [`RenderApi::put_secret_file`].
    pub fn create_web_service(
        &self,
        name: &str,
        image_path: &str,
        plan: &str,
        health_path: &str,
        settings: &BTreeMap<String, String>,
    ) -> Result<Service, Error> {
        if !WEB_SERVICE_PLANS.contains(&plan) {
            return Err(Error::Target(format!(
                "'{plan}' is not a plan Render lists for a web service"
            )));
        }
        let env_vars: Vec<Value> = settings
            .iter()
            .map(|(k, v)| json!({"key": k, "value": v}))
            .collect();
        let (_, body) = self.call(RestCall {
            method: Method::Post,
            path: "/services".to_string(),
            query: Vec::new(),
            body: Some(
                json!({
                    "type": "web_service",
                    "name": name,
                    "ownerId": self.owner_id,
                    "image": {"imagePath": image_path, "ownerId": self.owner_id},
                    "envVars": env_vars,
                    "serviceDetails": {"runtime": "image", "plan": plan, "healthCheckPath": health_path},
                })
                .to_string(),
            ),
        })?;
        let created = Self::service_of(&body)
            .ok_or_else(|| Error::Target("the create answer has no service".to_string()))?;
        if created.name != name || created.owner_id != self.owner_id {
            return Err(Error::Target(
                "Render created something other than what was asked".to_string(),
            ));
        }
        Ok(created)
    }

    /// The service's variables. A full page is refused.
    pub fn env_vars(&self, service_id: &str) -> Result<BTreeMap<String, String>, Error> {
        plain_id("service id", service_id)?;
        let body = self.get(
            format!("/services/{service_id}/env-vars"),
            vec![("limit".into(), PAGE.to_string())],
        )?;
        let items = body
            .as_array()
            .ok_or_else(|| Error::Target("the variable list is not an array".to_string()))?;
        if items.len() >= PAGE {
            return Err(Error::Target(
                "Render returned a full page of variables; there may be more than were read"
                    .to_string(),
            ));
        }
        items
            .iter()
            .map(|i| {
                let v = i.get("envVar").unwrap_or(i);
                match (
                    v.get("key").and_then(Value::as_str),
                    v.get("value").and_then(Value::as_str),
                ) {
                    (Some(k), Some(val)) => Ok((k.to_string(), val.to_string())),
                    _ => Err(Error::Target("a variable lacks key or value".to_string())),
                }
            })
            .collect()
    }

    /// Set one variable. The value travels in the request body only; no error or `Debug` here carries it.
    pub fn put_env_var(&self, service_id: &str, key: &str, value: &str) -> Result<(), Error> {
        plain_id("service id", service_id)?;
        plain_id("variable name", key)?;
        self.call(RestCall {
            method: Method::Put,
            path: format!("/services/{service_id}/env-vars/{key}"),
            query: Vec::new(),
            body: Some(json!({"value": value}).to_string()),
        })
        .map(|_| ())
    }

    /// Set one secret file (Render's `content`). Same rule as [`RenderApi::put_env_var`].
    pub fn put_secret_file(
        &self,
        service_id: &str,
        name: &str,
        content: &str,
    ) -> Result<(), Error> {
        plain_id("service id", service_id)?;
        plain_id("secret file name", name)?;
        self.call(RestCall {
            method: Method::Put,
            path: format!("/services/{service_id}/secret-files/{name}"),
            query: Vec::new(),
            body: Some(json!({"content": content}).to_string()),
        })
        .map(|_| ())
    }

    /// The service's custom domains. A full page is refused.
    pub fn custom_domains(&self, service_id: &str) -> Result<Vec<CustomDomain>, Error> {
        plain_id("service id", service_id)?;
        let body = self.get(
            format!("/services/{service_id}/custom-domains"),
            vec![("limit".into(), PAGE.to_string())],
        )?;
        let items = body
            .as_array()
            .ok_or_else(|| Error::Target("the domain list is not an array".to_string()))?;
        if items.len() >= PAGE {
            return Err(Error::Target(
                "Render returned a full page of domains; there may be more than were read"
                    .to_string(),
            ));
        }
        items.iter().map(Self::domain_of).collect()
    }

    fn domain_of(i: &Value) -> Result<CustomDomain, Error> {
        let v = i.get("customDomain").unwrap_or(i);
        let text = |k: &str| v.get(k).and_then(Value::as_str);
        match (text("id"), text("name"), text("verificationStatus")) {
            (Some(id), Some(name), Some(status)) => Ok(CustomDomain {
                id: id.to_string(),
                name: name.to_string(),
                verified: status == "verified",
            }),
            _ => Err(Error::Target(
                "a domain lacks id, name or verificationStatus".to_string(),
            )),
        }
    }

    /// Attach a host name to the service. Does nothing if this service already has it. A name another service holds is refused by Render with a
    /// 4xx whose code is **UNVERIFIED** (the schema lists 409 for create); the refusal is passed on, never retried.
    pub fn add_custom_domain(&self, service_id: &str, name: &str) -> Result<CustomDomain, Error> {
        if let Some(have) = self
            .custom_domains(service_id)?
            .into_iter()
            .find(|d| d.name == name)
        {
            return Ok(have);
        }
        let (_, body) = self.call(RestCall {
            method: Method::Post,
            path: format!("/services/{service_id}/custom-domains"),
            query: Vec::new(),
            body: Some(json!({"name": name}).to_string()),
        })?;
        // The create answer is a list in the schema's list call and a single object here; accept either, require the name to match.
        let first = body.as_array().and_then(|a| a.first()).unwrap_or(&body);
        let got = Self::domain_of(first)?;
        if got.name != name {
            return Err(Error::Target(
                "Render attached something other than what was asked".to_string(),
            ));
        }
        Ok(got)
    }

    /// Start a deploy. With `image_url` it deploys that image. The answer must carry a deploy id: an accepted request with no id is an error, since
    /// there would be nothing to read back.
    pub fn trigger_deploy(
        &self,
        service_id: &str,
        image_url: Option<&str>,
    ) -> Result<Deploy, Error> {
        plain_id("service id", service_id)?;
        let body = image_url.map(|u| json!({"imageUrl": u}).to_string());
        let (_, answer) = self.call(RestCall {
            method: Method::Post,
            path: format!("/services/{service_id}/deploys"),
            query: Vec::new(),
            body,
        })?;
        Self::deploy_of(&answer)
            .ok_or_else(|| Error::Target("the deploy answer has no deploy id".to_string()))
    }

    /// Read one deploy.
    pub fn deploy(&self, service_id: &str, deploy_id: &str) -> Result<Deploy, Error> {
        plain_id("service id", service_id)?;
        plain_id("deploy id", deploy_id)?;
        let body = self.get(
            format!("/services/{service_id}/deploys/{deploy_id}"),
            Vec::new(),
        )?;
        let got = Self::deploy_of(&body)
            .ok_or_else(|| Error::Target("the deploy answer has no id or status".to_string()))?;
        if got.id != deploy_id {
            return Err(Error::Target(
                "Render answered for a different deploy".to_string(),
            ));
        }
        Ok(got)
    }

    /// Roll back to an earlier deploy (`POST /services/{id}/rollback`, body `deployId`). Returns the new deploy; whether the rollback also restores
    /// variables is **UNVERIFIED**.
    pub fn rollback(&self, service_id: &str, deploy_id: &str) -> Result<Deploy, Error> {
        plain_id("service id", service_id)?;
        plain_id("deploy id", deploy_id)?;
        let (_, answer) = self.call(RestCall {
            method: Method::Post,
            path: format!("/services/{service_id}/rollback"),
            query: Vec::new(),
            body: Some(json!({"deployId": deploy_id}).to_string()),
        })?;
        Self::deploy_of(&answer)
            .ok_or_else(|| Error::Target("the rollback answer has no deploy id".to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cloudflare_domains::RestAnswer;
    use std::cell::RefCell;

    const OWNER: &str = "tea-owner1";

    /// A pretend Render that keeps services, variables, domains and deploys and answers in the schema's shapes.
    #[derive(Default)]
    struct FakeRender {
        services: RefCell<Vec<(String, String, String)>>, // id, name, owner
        vars: RefCell<BTreeMap<String, String>>,
        domains: RefCell<Vec<(String, String)>>, // id, name
        deploy_status: RefCell<String>,
        calls: RefCell<Vec<RestCall>>,
        force: RefCell<Option<RestAnswer>>,
        pad_services: usize,
    }

    fn json_answer(status: u16, v: Value) -> Result<RestAnswer, Error> {
        Ok(RestAnswer {
            status,
            body: v.to_string(),
        })
    }

    fn svc(id: &str, name: &str, owner: &str) -> Value {
        json!({"id": id, "name": name, "type": "web_service", "ownerId": owner,
               "serviceDetails": {"url": format!("https://{name}.onrender.com")}})
    }

    impl Rest for FakeRender {
        fn send(&self, call: &RestCall) -> Result<RestAnswer, Error> {
            self.calls.borrow_mut().push(call.clone());
            if let Some(a) = self.force.borrow().clone() {
                return Ok(a);
            }
            let body: Value = call
                .body
                .as_ref()
                .map(|b| serde_json::from_str(b).unwrap())
                .unwrap_or(Value::Null);
            let p = call.path.as_str();
            match (call.method, p) {
                (Method::Get, "/services") => {
                    let want = call
                        .query
                        .iter()
                        .find(|(k, _)| k == "name")
                        .unwrap()
                        .1
                        .clone();
                    let mut rows: Vec<Value> = self
                        .services
                        .borrow()
                        .iter()
                        // Like a substring search: "my-app" also finds "my-app-staging".
                        .filter(|(_, n, _)| n.contains(&want))
                        .map(|(i, n, o)| json!({"service": svc(i, n, o), "cursor": "c"}))
                        .collect();
                    for k in 0..self.pad_services {
                        rows.push(json!({"service": svc(&format!("srv-pad{k}"), "pad", OWNER), "cursor": "c"}));
                    }
                    json_answer(200, Value::Array(rows))
                }
                (Method::Post, "/services") => {
                    let (name, owner) = (
                        body["name"].as_str().unwrap().to_string(),
                        body["ownerId"].as_str().unwrap().to_string(),
                    );
                    let id = format!("srv-{}", self.services.borrow().len() + 1);
                    self.services
                        .borrow_mut()
                        .push((id.clone(), name.clone(), owner.clone()));
                    for e in body["envVars"].as_array().unwrap() {
                        self.vars.borrow_mut().insert(
                            e["key"].as_str().unwrap().to_string(),
                            e["value"].as_str().unwrap().to_string(),
                        );
                    }
                    json_answer(
                        201,
                        json!({"service": svc(&id, &name, &owner), "deployId": "dep-1"}),
                    )
                }
                (Method::Get, x) if x.ends_with("/env-vars") => {
                    let rows: Vec<Value> = self
                        .vars
                        .borrow()
                        .iter()
                        .map(|(k, v)| json!({"envVar": {"key": k, "value": v}, "cursor": "c"}))
                        .collect();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Put, x) if x.contains("/env-vars/") => {
                    let key = x.rsplit('/').next().unwrap().to_string();
                    self.vars
                        .borrow_mut()
                        .insert(key, body["value"].as_str().unwrap().to_string());
                    json_answer(200, json!({"key": "k", "value": "v"}))
                }
                (Method::Put, x) if x.contains("/secret-files/") => {
                    json_answer(200, json!({"name": "f", "content": "c"}))
                }
                (Method::Get, x) if x.ends_with("/custom-domains") => {
                    let rows: Vec<Value> = self
                        .domains
                        .borrow()
                        .iter()
                        .map(|(i, n)| json!({"customDomain": {"id": i, "name": n, "domainType": "subdomain", "verificationStatus": "unverified"}, "cursor": "c"}))
                        .collect();
                    json_answer(200, Value::Array(rows))
                }
                (Method::Post, x) if x.ends_with("/custom-domains") => {
                    let n = body["name"].as_str().unwrap().to_string();
                    let id = format!("cdm-{}", self.domains.borrow().len() + 1);
                    self.domains.borrow_mut().push((id.clone(), n.clone()));
                    json_answer(
                        201,
                        json!({"id": id, "name": n, "domainType": "subdomain", "verificationStatus": "unverified"}),
                    )
                }
                (Method::Post, x) if x.ends_with("/deploys") || x.ends_with("/rollback") => {
                    *self.deploy_status.borrow_mut() = "created".into();
                    json_answer(
                        201,
                        json!({"id": "dep-2", "status": "created", "trigger": "api"}),
                    )
                }
                (Method::Get, x) if x.contains("/deploys/") => {
                    let id = x.rsplit('/').next().unwrap();
                    let status = self.deploy_status.borrow().clone();
                    json_answer(200, json!({"id": id, "status": status}))
                }
                other => panic!("unexpected call {other:?}"),
            }
        }
    }

    fn api(f: FakeRender) -> RenderApi<FakeRender> {
        RenderApi::new(f, OWNER).unwrap()
    }

    fn settings() -> BTreeMap<String, String> {
        BTreeMap::from([(
            "APP_BASE_URL".to_string(),
            "https://app.example.org".to_string(),
        )])
    }

    const REFUSED: &str = r#"{"id":"x","message":"name already in use","code":"conflict"}"#;

    #[test]
    fn create_then_find_then_read_back_the_variables() {
        let a = api(FakeRender::default());
        let made = a
            .create_web_service(
                "my-app",
                "docker.io/org/app:1",
                "starter",
                "/health",
                &settings(),
            )
            .unwrap();
        assert_eq!(made.name, "my-app");
        assert_eq!(a.find_service("my-app").unwrap().unwrap().id, made.id);
        assert_eq!(
            a.env_vars(&made.id).unwrap()["APP_BASE_URL"],
            "https://app.example.org"
        );
        assert!(a.find_service("nobody").unwrap().is_none()); // control: the filter filters
    }

    #[test]
    fn the_create_call_asks_for_an_image_service_with_the_documented_fields() {
        let a = api(FakeRender::default());
        a.create_web_service(
            "my-app",
            "docker.io/org/app:1",
            "starter",
            "/health",
            &settings(),
        )
        .unwrap();
        let calls = a.rest.calls.borrow();
        let body: Value = serde_json::from_str(calls[0].body.as_ref().unwrap()).unwrap();
        assert_eq!(body["type"], "web_service");
        assert_eq!(body["image"]["imagePath"], "docker.io/org/app:1");
        assert_eq!(body["image"]["ownerId"], OWNER);
        assert_eq!(body["serviceDetails"]["runtime"], "image");
        assert_eq!(body["serviceDetails"]["healthCheckPath"], "/health");
    }

    #[test]
    fn a_name_that_only_contains_the_wanted_name_is_not_the_service() {
        let f = FakeRender::default();
        f.services
            .borrow_mut()
            .push(("srv-9".into(), "my-app-staging".into(), OWNER.into()));
        assert!(api(f).find_service("my-app").unwrap().is_none());
    }

    #[test]
    fn a_service_of_another_workspace_with_the_same_name_is_not_ours() {
        let f = FakeRender::default();
        f.services
            .borrow_mut()
            .push(("srv-9".into(), "my-app".into(), "tea-other".into()));
        assert!(api(f).find_service("my-app").unwrap().is_none());
    }

    #[test]
    fn a_full_page_is_refused_not_trusted() {
        let f = FakeRender {
            pad_services: 100,
            ..FakeRender::default()
        };
        assert!(api(f).find_service("my-app").is_err());
    }

    #[test]
    fn a_plan_the_schema_does_not_list_is_refused_before_any_call() {
        let a = api(FakeRender::default());
        assert!(
            a.create_web_service("my-app", "i", "starterr", "/h", &settings())
                .is_err()
        );
        assert!(a.rest.calls.borrow().is_empty());
    }

    #[test]
    fn create_answers_that_name_something_else_or_have_no_service_are_refused() {
        for answer in [
            json!({"service": svc("srv-1", "other-name", OWNER)}),
            json!({"deployId": "dep-1"}),
        ] {
            let f = FakeRender::default();
            *f.force.borrow_mut() = Some(RestAnswer {
                status: 201,
                body: answer.to_string(),
            });
            assert!(
                api(f)
                    .create_web_service("my-app", "i", "starter", "/h", &settings())
                    .is_err()
            );
        }
    }

    #[test]
    fn a_refusal_carries_render_s_message_and_code_but_never_the_request() {
        let f = FakeRender::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 409,
            body: REFUSED.into(),
        });
        let e = api(f)
            .put_env_var("srv-1", "DB_PASSWORD", "canary-secret-77")
            .unwrap_err();
        let shown = format!("{e} {e:?}");
        assert!(
            shown.contains("name already in use")
                && shown.contains("conflict")
                && shown.contains("409")
        );
        assert!(!shown.contains("canary-secret-77"));
    }

    #[test]
    fn a_non_json_body_and_a_bad_status_are_errors() {
        let f = FakeRender::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 200,
            body: "<html>".into(),
        });
        assert!(api(f).find_service("x").is_err());
        let f = FakeRender::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 503,
            body: "{}".into(),
        });
        assert!(api(f).find_service("x").is_err());
    }

    #[test]
    fn variables_and_secret_files_go_to_their_documented_paths_with_the_value_in_the_body() {
        let a = api(FakeRender::default());
        a.put_env_var("srv-1", "TOKEN", "v1").unwrap();
        a.put_secret_file("srv-1", "key", "pem-text").unwrap();
        let calls = a.rest.calls.borrow();
        assert_eq!(
            (calls[0].method, calls[0].path.as_str()),
            (Method::Put, "/services/srv-1/env-vars/TOKEN")
        );
        assert_eq!(calls[0].body.as_deref(), Some(r#"{"value":"v1"}"#));
        assert_eq!(calls[1].path, "/services/srv-1/secret-files/key");
        assert_eq!(calls[1].body.as_deref(), Some(r#"{"content":"pem-text"}"#));
        assert!(
            calls.iter().all(|c| c.query.is_empty()),
            "a value never travels in a URL"
        );
    }

    #[test]
    fn a_domain_is_added_once_and_read_back_with_its_verification_status() {
        let a = api(FakeRender::default());
        let d = a.add_custom_domain("srv-1", "app.example.org").unwrap();
        assert_eq!(d.name, "app.example.org");
        assert!(!d.verified);
        a.add_custom_domain("srv-1", "app.example.org").unwrap();
        let posts = a
            .rest
            .calls
            .borrow()
            .iter()
            .filter(|c| c.method == Method::Post)
            .count();
        assert_eq!(posts, 1, "the second add finds it and writes nothing");
        assert_eq!(a.custom_domains("srv-1").unwrap().len(), 1);
    }

    #[test]
    fn a_domain_answer_for_another_name_is_not_accepted() {
        struct Liar(FakeRender);
        impl Rest for Liar {
            fn send(&self, c: &RestCall) -> Result<RestAnswer, Error> {
                if c.method == Method::Post {
                    return Ok(RestAnswer { status: 201, body: json!({"id":"cdm-1","name":"evil.example.org","verificationStatus":"verified"}).to_string() });
                }
                self.0.send(c)
            }
        }
        let a = RenderApi::new(Liar(FakeRender::default()), OWNER).unwrap();
        assert!(a.add_custom_domain("srv-1", "app.example.org").is_err());
    }

    #[test]
    fn a_deploy_is_triggered_read_back_and_its_status_words_are_classified() {
        let a = api(FakeRender::default());
        let started = a
            .trigger_deploy("srv-1", Some("docker.io/org/app:2"))
            .unwrap();
        assert_eq!(started.status, DeployStatus::Pending);
        for (word, live, failed) in [
            ("live", true, false),
            ("build_failed", false, true),
            ("update_failed", false, true),
            ("pre_deploy_failed", false, true),
            ("canceled", false, true),
            ("build_in_progress", false, false),
            ("deactivated", false, false),
            ("some_future_status", false, false),
        ] {
            *a.rest.deploy_status.borrow_mut() = word.to_string();
            let d = a.deploy("srv-1", &started.id).unwrap();
            assert_eq!(
                (d.status.is_live(), d.status.is_failed()),
                (live, failed),
                "{word}"
            );
        }
    }

    #[test]
    fn a_deploy_answer_for_another_deploy_or_without_an_id_is_refused() {
        let f = FakeRender::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 200,
            body: json!({"id": "dep-OTHER", "status": "live"}).to_string(),
        });
        assert!(api(f).deploy("srv-1", "dep-2").is_err());
        let f = FakeRender::default();
        *f.force.borrow_mut() = Some(RestAnswer {
            status: 202,
            body: String::new(),
        });
        assert!(api(f).trigger_deploy("srv-1", None).is_err());
    }

    #[test]
    fn rollback_posts_the_deploy_id_and_returns_the_new_deploy() {
        let a = api(FakeRender::default());
        let d = a.rollback("srv-1", "dep-1").unwrap();
        assert_eq!(d.status, DeployStatus::Pending);
        let calls = a.rest.calls.borrow();
        assert_eq!(calls[0].path, "/services/srv-1/rollback");
        assert_eq!(calls[0].body.as_deref(), Some(r#"{"deployId":"dep-1"}"#));
    }

    #[test]
    fn ids_that_could_add_path_segments_are_refused() {
        let a = api(FakeRender::default());
        assert!(a.env_vars("srv-1/../x").is_err());
        assert!(a.put_env_var("srv-1", "A/B", "v").is_err());
        assert!(a.deploy("srv-1", "").is_err());
        assert!(RenderApi::new(FakeRender::default(), "a/b").is_err());
        assert!(a.rest.calls.borrow().is_empty());
    }
}
