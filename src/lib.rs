#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod host;
mod manifest;
mod probe;
mod target;

pub mod cloudflare;
pub mod cloudflare_domains;
#[cfg(feature = "configurant")]
pub mod configurant_source;
pub mod conformance;
pub mod fake;
#[cfg(feature = "local")]
pub mod local;
pub mod preflight;
pub mod render;
pub mod render_target;
#[cfg(feature = "secretant")]
pub mod secretant_source;

pub use error::Error;
pub use host::{
    Applied, DataAfterRollback, Host, Plan, ReadBack, Release, Rollback, RollbackPoint, StepOutcome,
};
pub use manifest::{AppManifest, Config, EnvSecrets, MapSecrets, Migrations, SecretValue, Secrets};
pub use probe::{Health, HealthProbe, WaitPolicy, Waited, wait_until_healthy};
pub use target::{Observed, Op, Presence, Renewal, Snapshot, Step, Support, Target, TlsStatus};
