#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod host;
mod manifest;
mod target;

pub mod cloudflare;
pub mod conformance;
pub mod fake;

pub use error::Error;
pub use host::{Applied, Host, Plan, ReadBack};
pub use manifest::{AppManifest, Config, MapSecrets, SecretValue, Secrets};
pub use target::{Observed, Op, Presence, Step, Support, Target};
