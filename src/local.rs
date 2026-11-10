//! Local serving with WebServAnt (feature `local`, ROADMAP row C8): this first slice reads a running site's certificate back as a [`TlsStatus`].
//!
//! What the answer rests on: [`read_tls`] makes a real TLS handshake with the site's own listener (`RunningSite::served_certificate`), so the
//! expiry and issuer are what a visitor is served, not what a store file says. The certificate is NOT validated by that handshake (a self-signed or
//! expired one is reported as it is), so the result is for reporting and must never decide trust. When the site has automatic certificates,
//! `Certificates::status` adds the covered names and how the last renewal ended; a site with a certificate from files has neither, and says
//! `Renewal::Unknown`. Nothing here reads, keeps or prints a private key.

use webservant::{CertificateStatus, RunningSite, ServedCert};

use crate::{Error, Renewal, TlsStatus};

/// Seconds since the Unix epoch as RFC 3339 text in UTC (`2023-11-14T22:13:20Z`). Years 0000 to 9999 only; anything else is `None`.
pub fn rfc3339(unix_seconds: i64) -> Option<String> {
    let days = unix_seconds.div_euclid(86_400);
    let secs = unix_seconds.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    if !(0..=9_999).contains(&year) {
        return None;
    }
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs / 3_600,
        secs % 3_600 / 60,
        secs % 60
    ))
}

/// The DNS names a DER certificate lists as subject alternative names; empty when it lists none or cannot be read.
fn names_in(der: &[u8]) -> Vec<String> {
    use x509_parser::extensions::GeneralName;
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(der) else {
        return Vec::new();
    };
    let Ok(Some(san)) = cert.subject_alternative_name() else {
        return Vec::new();
    };
    san.value
        .general_names
        .iter()
        .filter_map(|n| match n {
            GeneralName::DNSName(name) => Some((*name).to_string()),
            _ => None,
        })
        .collect()
}

/// Combine what the listener served with what the certificate manager says, if there is one.
pub fn tls_status_of(served: &ServedCert, manager: Option<&CertificateStatus>) -> TlsStatus {
    let (names, last_renewal) = match manager {
        Some(m) => (
            m.names.clone(),
            match &m.last_result {
                None => Renewal::Unknown,
                Some(Ok(())) => Renewal::Succeeded,
                Some(Err(why)) => Renewal::Failed(why.clone()),
            },
        ),
        // No manager: the names are the ones the served certificate itself lists (its DNS subject alternative names), NOT the name asked for. A
        // listener that holds one certificate from files serves it for any name a client sends, so echoing the asked name would claim coverage.
        None => (names_in(&served.leaf_der), Renewal::Unknown),
    };
    TlsStatus {
        names,
        not_after: rfc3339(served.not_after),
        issuer: Some(served.issuer.clone()),
        last_renewal,
    }
}

/// Read the certificate `site` presents for `server_name` over a real handshake. An error (not an invented answer) when the site is not serving
/// HTTPS or the handshake fails.
pub async fn read_tls(site: &RunningSite, server_name: &str) -> Result<TlsStatus, Error> {
    let served = site
        .served_certificate(server_name)
        .await
        .map_err(|e| Error::Target(format!("could not read the served certificate: {e}")))?;
    let manager = site.certificates().map(|c| c.status());
    Ok(tls_status_of(&served, manager.as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use webservant::Site;

    #[test]
    fn unix_seconds_become_rfc3339_including_leap_days_and_before_1970() {
        assert_eq!(rfc3339(0).as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(
            rfc3339(1_700_000_000).as_deref(),
            Some("2023-11-14T22:13:20Z")
        );
        assert_eq!(
            rfc3339(951_782_400).as_deref(),
            Some("2000-02-29T00:00:00Z")
        );
        assert_eq!(rfc3339(-1).as_deref(), Some("1969-12-31T23:59:59Z"));
        assert_eq!(
            rfc3339(253_402_300_799).as_deref(),
            Some("9999-12-31T23:59:59Z")
        );
        assert_eq!(rfc3339(253_402_300_800), None);
        assert_eq!(rfc3339(-62_167_219_201), None);
    }

    /// `ServedCert` cannot be built here (it is `non_exhaustive`), so take a real one from a listener and set the fields a test needs.
    fn served(not_after: i64) -> ServedCert {
        let (dir, cert, key) = self_signed();
        let mut served = runtime().block_on(async {
            let site = Site::new("app.test")
                .files(dir.path())
                .bind("127.0.0.1:0")
                .tls_pem_files(&cert, &key)
                .start()
                .await
                .unwrap();
            let served = site.served_certificate("app.test").await.unwrap();
            site.shutdown().await.unwrap();
            served
        });
        served.not_after = not_after;
        served.issuer = "CN=Test CA".to_string();
        served
    }

    #[test]
    fn the_served_certificate_gives_the_expiry_and_issuer_and_a_manager_gives_names_and_renewal() {
        let alone = tls_status_of(&served(1_700_000_000), None);
        assert_eq!(alone.names, ["app.test", "www.app.test"]);
        assert_eq!(alone.not_after.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(alone.issuer.as_deref(), Some("CN=Test CA"));
        assert_eq!(alone.last_renewal, Renewal::Unknown);

        let managed = |last_result| CertificateStatus {
            names: vec!["managed.test".to_string(), "www.managed.test".to_string()],
            cert: None,
            last_result,
        };
        let ok = tls_status_of(&served(0), Some(&managed(Some(Ok(())))));
        assert_eq!(ok.names, ["managed.test", "www.managed.test"]);
        assert_eq!(ok.last_renewal, Renewal::Succeeded);
        let failed = tls_status_of(&served(0), Some(&managed(Some(Err("port 80".into())))));
        assert_eq!(failed.last_renewal, Renewal::Failed("port 80".to_string()));
        let none = tls_status_of(&served(0), Some(&managed(None)));
        assert_eq!(none.last_renewal, Renewal::Unknown);
    }

    #[test]
    fn a_certificate_without_names_or_unreadable_lists_no_names() {
        assert!(names_in(b"not a certificate").is_empty());
        let key = rcgen::KeyPair::generate().unwrap();
        let bare = rcgen::CertificateParams::new(Vec::<String>::new())
            .unwrap()
            .self_signed(&key)
            .unwrap();
        assert!(names_in(bare.der()).is_empty());
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
    }

    /// A self-signed certificate for `app.test`, valid until 2030-01-01, written as PEM files; returns (directory, cert path, key path).
    fn self_signed() -> (tempfile::TempDir, std::path::PathBuf, std::path::PathBuf) {
        let mut params =
            rcgen::CertificateParams::new(vec!["app.test".to_string(), "www.app.test".to_string()])
                .unwrap();
        params.not_after = rcgen::date_time_ymd(2030, 1, 1);
        let key = rcgen::KeyPair::generate().unwrap();
        let cert = params.self_signed(&key).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let (c, k) = (dir.path().join("cert.pem"), dir.path().join("key.pem"));
        std::fs::write(&c, cert.pem()).unwrap();
        std::fs::write(&k, key.serialize_pem()).unwrap();
        (dir, c, k)
    }

    #[test]
    fn a_site_serving_a_certificate_from_files_is_read_back_over_a_real_handshake() {
        let (dir, cert, key) = self_signed();
        let files = dir.path().join("public");
        std::fs::create_dir(&files).unwrap();
        std::fs::write(files.join("index.html"), "hi").unwrap();
        runtime().block_on(async {
            let site = Site::new("app.test")
                .files(&files)
                .bind("127.0.0.1:0")
                .tls_pem_files(&cert, &key)
                .start()
                .await
                .unwrap();
            let status = read_tls(&site, "app.test").await.unwrap();
            assert_eq!(status.names, ["app.test", "www.app.test"]);
            assert_eq!(status.not_after.as_deref(), Some("2030-01-01T00:00:00Z"));
            assert!(status.issuer.is_some());
            assert_eq!(
                status.last_renewal,
                Renewal::Unknown,
                "a certificate from files has no renewal to report"
            );
            // This listener serves its one certificate for any name asked, so the answer must list what the certificate covers, not the asked name.
            let other = read_tls(&site, "other.test").await.unwrap();
            assert!(!other.names.iter().any(|n| n == "other.test"));
            assert_eq!(other.names, status.names);
            site.shutdown().await.unwrap();
        });
    }

    #[test]
    fn a_site_that_does_not_serve_https_has_no_certificate_to_read() {
        let dir = tempfile::tempdir().unwrap();
        runtime().block_on(async {
            let site = Site::new("app.test")
                .files(dir.path())
                .bind("127.0.0.1:0")
                .start()
                .await
                .unwrap();
            assert!(read_tls(&site, "app.test").await.is_err());
            site.shutdown().await.unwrap();
        });
    }
}
