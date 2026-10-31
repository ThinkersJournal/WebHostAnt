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
pub mod render;

pub use error::Error;
pub use host::{
    Applied, DataAfterRollback, Host, Plan, ReadBack, Release, Rollback, RollbackPoint, StepOutcome,
};
pub use manifest::{AppManifest, Config, MapSecrets, Migrations, SecretValue, Secrets};
pub use probe::{Health, HealthProbe};
pub use target::{Observed, Op, Presence, Renewal, Snapshot, Step, Support, Target, TlsStatus};
