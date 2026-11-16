// SPDX-License-Identifier: MIT OR Apache-2.0
//! Ask "will automatic certificates work here?" with one issuance against Let's Encrypt STAGING (never production), then stop.
//!
//! ```text
//! cargo run --features local --example local_staging -- <public-name> <contact-email> <https-bind> <http-bind> [--fake-ca]
//! ```
//!
//! The site serves a temporary folder holding one static file and keeps its certificate store in a throwaway folder; both are deleted on exit.
//! `ACME_STAGING` is left unset, so the authority is staging by default (ROADMAP row C8d). With `--fake-ca` no network is touched: a local
//! stand-in issues a self-signed certificate, which proves the wiring offline. Prints the reachability answer only: no account key, no secret.

use std::process::exit;

use webhostant::local_target::LocalTarget;
use webhostant::{Op, Step, Target};
use webservant::acme::LETS_ENCRYPT_STAGING;
use webservant::{AcmeClient, AcmeError, ChallengeKind, Challenges, Issued};

/// An offline authority: a self-signed certificate for the asked names.
struct FakeCa;

impl AcmeClient for FakeCa {
    fn obtain<'a>(
        &'a self,
        names: &'a [String],
        _challenges: &'a Challenges,
        _kind: ChallengeKind,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Issued, AcmeError>> + Send + 'a>>
    {
        Box::pin(async move {
            let params = rcgen::CertificateParams::new(names.to_vec())
                .map_err(|e| AcmeError(e.to_string()))?;
            let key = rcgen::KeyPair::generate().map_err(|e| AcmeError(e.to_string()))?;
            let cert = params
                .self_signed(&key)
                .map_err(|e| AcmeError(e.to_string()))?;
            Ok(Issued {
                chain_pem: cert.pem(),
                key_pem: key.serialize_pem(),
            })
        })
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let fake = args.iter().any(|a| a == "--fake-ca");
    args.retain(|a| a != "--fake-ca");
    let [name, contact, https_bind, http_bind] = args.as_slice() else {
        eprintln!(
            "usage: local_staging <public-name> <contact-email> <https-bind> <http-bind> [--fake-ca]"
        );
        exit(2);
    };
    let (files, store) = (
        tempfile::tempdir().expect("a temporary folder"),
        tempfile::tempdir().expect("a temporary folder"),
    );
    std::fs::write(
        files.path().join("index.html"),
        "webhostant staging check\n",
    )
    .expect("one static file");

    let mut target = LocalTarget::new()
        .expect("a local target")
        .trust_operations(&[Op::Create, Op::SetSetting]);
    if fake {
        target = target.with_acme_client(FakeCa);
    }
    let settings = [
        ("FILES", files.path().display().to_string()),
        // First: `ACME_CONTACT` is the opt-in to listening beyond loopback, so a `BIND` set before it is refused (`a_wide_bind_with_a_contact_earlier_in_the_same_settings_is_accepted` in
        // `local_target.rs` holds this order).
        ("ACME_CONTACT", contact.clone()),
        ("BIND", https_bind.clone()),
        ("HTTP_BIND", http_bind.clone()),
        ("ACME_STORE", store.path().display().to_string()),
    ];
    let result = target
        .apply_step(name, &Step::Create, None)
        .and_then(|_| {
            settings.iter().try_for_each(|(k, v)| {
                target
                    .apply_step(
                        name,
                        &Step::SetSetting {
                            name: (*k).to_string(),
                            value: v.clone(),
                        },
                        None,
                    )
                    .map(|_| ())
            })
        })
        .and_then(|()| target.check_reachable(name));
    match result {
        Ok(r) if r.is_reachable() => println!(
            "reachable: one issuance succeeded ({})",
            if fake {
                "fake CA, offline".to_string()
            } else {
                format!("Let's Encrypt staging, directory {LETS_ENCRYPT_STAGING}")
            }
        ),
        Ok(r) => {
            println!("not reachable: {}", r.failure.unwrap_or_default());
            exit(1);
        }
        Err(e) => {
            eprintln!("could not run the check: {e}");
            exit(2);
        }
    }
}
