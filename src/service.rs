//! The on-boot path of local mode (ROADMAP row C8e): a service definition for the operating system, generated and shown, and never installed without
//! the digest of exactly that text.
//!
//! SupervisorAnt does not exist yet (docs/DESIGN.md section 7.4), so this crate generates the definition itself and says so. What it does:
//!
//! * [`ServiceSpec::systemd_unit`] and [`ServiceSpec::windows_service`] turn a spec into a [`ServiceDefinition`]: the text, a file name, and a
//!   [`ServiceDefinition::digest`] over all of it. Every value is checked first, and every value is quoted for the target syntax, so a name, path or
//!   argument cannot add a line or a second command.
//! * [`install_unit_file`] writes a systemd unit file into a folder the caller names, only when handed the digest of that definition, and refuses to
//!   overwrite a file. It does **not** run `systemctl`, enable or start anything: the person reads the text, then does that step.
//! * A Windows service is generated and shown only; there is no Windows installer here ([`install_unit_file`] refuses it).
//!
//! No secret goes into a definition: a spec has no environment and no secret field. **UNVERIFIED**: neither text has been given to a real
//! `systemd-analyze verify` or a real Windows host; the tests check the text we generate, not what the operating system makes of it.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::Error;

/// What to run as a service. Nothing here is a secret.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    /// The service's name: lower-case letters, digits and `-`, starting with a letter, at most 41 characters.
    pub name: String,
    /// One line saying what it is.
    pub description: String,
    /// The program, as an absolute path.
    pub exec: String,
    /// Its arguments, one entry each.
    pub args: Vec<String>,
    /// The folder to run in, as an absolute path.
    pub working_dir: Option<String>,
    /// The account to run as (systemd only). `None` runs as the service manager's default, which for a system unit is root: set one.
    pub run_as: Option<String>,
    /// Whether the program must bind ports below 1024 (80 and 443) without running as root (systemd only: adds `CAP_NET_BIND_SERVICE`).
    pub privileged_ports: bool,
}

/// Which operating system's service definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// A systemd unit file (`<name>.service`).
    SystemdUnit,
    /// A PowerShell script that creates a Windows service (`<name>.ps1`). Generated and shown only.
    WindowsService,
}

/// A generated definition. Its fields are private, so the text cannot be changed after its digest was shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDefinition {
    kind: ServiceKind,
    file_name: String,
    text: String,
}

impl ServiceDefinition {
    /// Which operating system this is for.
    pub fn kind(&self) -> ServiceKind {
        self.kind
    }

    /// The file name it would be written as.
    pub fn file_name(&self) -> &str {
        &self.file_name
    }

    /// The exact text.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// A fingerprint of the kind, the file name and the text. Show it with the text; pass it to [`install_unit_file`] to install only what was shown.
    pub fn digest(&self) -> String {
        let mut h = Sha256::new();
        let mut field = |tag: u8, text: &str| {
            h.update([tag]);
            h.update((text.len() as u64).to_be_bytes());
            h.update(text.as_bytes());
        };
        field(b'v', "webhostant-service-v1");
        field(
            b'k',
            match self.kind {
                ServiceKind::SystemdUnit => "systemd",
                ServiceKind::WindowsService => "windows",
            },
        );
        field(b'f', &self.file_name);
        field(b't', &self.text);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }

    /// What to show a person before asking for approval: the digest, the file name, the text, and what is not known.
    pub fn show(&self) -> String {
        format!(
            "service definition {} ({:?})\ndigest {}\nUNVERIFIED: this text has not been checked by a real service manager.\n----\n{}",
            self.file_name,
            self.kind,
            self.digest(),
            self.text
        )
    }
}

fn bad(what: &str, why: &str) -> Error {
    Error::Target(format!("service definition: {what} {why}"))
}

/// No control characters at all (a newline would add a line to the file).
fn plain(what: &str, value: &str) -> Result<(), Error> {
    if value.chars().any(char::is_control) {
        return Err(bad(what, "holds a control character"));
    }
    Ok(())
}

fn absolute(what: &str, value: &str, kind: ServiceKind) -> Result<(), Error> {
    plain(what, value)?;
    let ok = match kind {
        ServiceKind::SystemdUnit => value.starts_with('/'),
        ServiceKind::WindowsService => {
            let b = value.as_bytes();
            (b.len() > 2
                && b[0].is_ascii_alphabetic()
                && b[1] == b':'
                && (b[2] == b'\\' || b[2] == b'/'))
                || value.starts_with("\\\\")
        }
    };
    if ok {
        Ok(())
    } else {
        Err(bad(what, "is not an absolute path"))
    }
}

impl ServiceSpec {
    fn validate(&self, kind: ServiceKind) -> Result<(), Error> {
        let n = &self.name;
        let name_ok = !n.is_empty()
            && n.len() <= 41
            && n.starts_with(|c: char| c.is_ascii_lowercase())
            && n.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !name_ok {
            return Err(bad(
                "the name",
                "must be lower-case letters, digits and `-`, starting with a letter (at most 41)",
            ));
        }
        plain("the description", &self.description)?;
        if self.description.trim().is_empty() {
            return Err(bad("the description", "is empty"));
        }
        absolute("the program", &self.exec, kind)?;
        for arg in &self.args {
            plain("an argument", arg)?;
        }
        if let Some(dir) = &self.working_dir {
            absolute("the working folder", dir, kind)?;
        }
        if let Some(user) = &self.run_as {
            let ok = user.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
                && user
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
            if !ok {
                return Err(bad("the account", "is not a plain account name"));
            }
        }
        Ok(())
    }

    /// A systemd unit. Restarts on failure, wants the network, and runs with `NoNewPrivileges`; `WantedBy=multi-user.target` makes it start on boot
    /// once someone runs `systemctl enable` (this crate does not).
    pub fn systemd_unit(&self) -> Result<ServiceDefinition, Error> {
        self.validate(ServiceKind::SystemdUnit)?;
        // systemd expands `%` specifiers and `$` variables in command lines, so both are doubled; words are double-quoted with `\` and `"` escaped.
        let word = |w: &str| {
            let escaped = w
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('%', "%%")
                .replace('$', "$$");
            format!("\"{escaped}\"")
        };
        let mut exec_start = word(&self.exec);
        for a in &self.args {
            exec_start.push(' ');
            exec_start.push_str(&word(a));
        }
        let mut text = String::new();
        text.push_str("[Unit]\n");
        text.push_str(&format!(
            "Description={}\n",
            self.description.replace('%', "%%")
        ));
        text.push_str("After=network-online.target\nWants=network-online.target\n\n");
        text.push_str("[Service]\n");
        text.push_str(&format!("ExecStart={exec_start}\n"));
        if let Some(dir) = &self.working_dir {
            text.push_str(&format!("WorkingDirectory={}\n", dir.replace('%', "%%")));
        }
        if let Some(user) = &self.run_as {
            text.push_str(&format!("User={user}\n"));
        }
        text.push_str("Restart=on-failure\nRestartSec=5\nNoNewPrivileges=true\n");
        if self.privileged_ports {
            text.push_str("AmbientCapabilities=CAP_NET_BIND_SERVICE\n");
        }
        text.push_str("\n[Install]\nWantedBy=multi-user.target\n");
        Ok(ServiceDefinition {
            kind: ServiceKind::SystemdUnit,
            file_name: format!("{}.service", self.name),
            text,
        })
    }

    /// A PowerShell script that creates an automatic-start Windows service with `New-Service`. Generated and shown only: nothing here runs it.
    /// Every value is a single-quoted PowerShell string (`'` doubled), so no value can end the string or add a command.
    pub fn windows_service(&self) -> Result<ServiceDefinition, Error> {
        self.validate(ServiceKind::WindowsService)?;
        let quote = |v: &str| format!("'{}'", v.replace('\'', "''"));
        // The service's command line is one string: the program, then each argument, each in double quotes (a double quote inside is refused).
        let mut parts = vec![format!("\"{}\"", self.exec)];
        for a in &self.args {
            if a.contains('"') {
                return Err(bad(
                    "an argument",
                    "holds a double quote, which a Windows service command line cannot carry here",
                ));
            }
            parts.push(format!("\"{a}\""));
        }
        if self.exec.contains('"') {
            return Err(bad("the program", "holds a double quote"));
        }
        let mut text = String::new();
        text.push_str(&format!(
            "New-Service -Name {} -DisplayName {} -BinaryPathName {} -StartupType Automatic\n",
            quote(&self.name),
            quote(&self.description),
            quote(&parts.join(" "))
        ));
        Ok(ServiceDefinition {
            kind: ServiceKind::WindowsService,
            file_name: format!("{}.ps1", self.name),
            text,
        })
    }
}

/// Write a systemd unit file into `dir`, only when `approved_digest` is the digest of `definition`. It refuses a different or empty digest, a Windows
/// definition, a folder that is not there, and a file that already exists (nothing is overwritten); in every refusal nothing is written. It does not
/// enable or start the service. Returns the path written.
pub fn install_unit_file(
    definition: &ServiceDefinition,
    approved_digest: &str,
    dir: &Path,
) -> Result<PathBuf, Error> {
    use std::io::Write;
    if definition.kind != ServiceKind::SystemdUnit {
        return Err(bad(
            "a Windows service",
            "is shown only; this crate does not install one",
        ));
    }
    if approved_digest != definition.digest() {
        return Err(bad(
            "the approved digest",
            "is not the digest of this definition; nothing was installed",
        ));
    }
    let path = dir.join(&definition.file_name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| Error::Target(format!("could not create the unit file: {e}")))?;
    file.write_all(definition.text.as_bytes())
        .map_err(|e| Error::Target(format!("could not write the unit file: {e}")))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ServiceSpec {
        ServiceSpec {
            name: "school-app".into(),
            description: "School app".into(),
            exec: "/opt/school/bin/app".into(),
            args: vec!["--port".into(), "8080".into()],
            working_dir: Some("/opt/school".into()),
            run_as: Some("school".into()),
            privileged_ports: false,
        }
    }

    #[test]
    fn a_systemd_unit_is_the_exact_text_we_mean() {
        let d = spec().systemd_unit().unwrap();
        assert_eq!(d.file_name(), "school-app.service");
        assert_eq!(
            d.text(),
            "[Unit]\nDescription=School app\nAfter=network-online.target\nWants=network-online.target\n\n[Service]\n\
             ExecStart=\"/opt/school/bin/app\" \"--port\" \"8080\"\nWorkingDirectory=/opt/school\nUser=school\n\
             Restart=on-failure\nRestartSec=5\nNoNewPrivileges=true\n\n[Install]\nWantedBy=multi-user.target\n"
        );
        let mut wide = spec();
        wide.privileged_ports = true;
        assert!(
            wide.systemd_unit()
                .unwrap()
                .text()
                .contains("AmbientCapabilities=CAP_NET_BIND_SERVICE\n")
        );
        assert!(!d.text().contains("AmbientCapabilities"));
    }

    #[test]
    fn a_value_cannot_add_a_line_or_a_command() {
        for field in 0..5 {
            let mut s = spec();
            let evil = "x\nExecStartPre=/bin/evil".to_string();
            match field {
                0 => s.description = evil,
                1 => s.exec = format!("/bin/{evil}"),
                2 => s.args = vec![evil],
                3 => s.working_dir = Some(format!("/{evil}")),
                _ => s.run_as = Some(evil),
            }
            assert!(s.systemd_unit().is_err(), "field {field}");
            assert!(s.windows_service().is_err(), "field {field}");
        }
    }

    #[test]
    fn quotes_percent_and_dollar_in_an_argument_are_escaped_for_systemd() {
        let mut s = spec();
        s.args = vec!["a \"b\" %h $HOME \\".into()];
        let d = s.systemd_unit().unwrap();
        assert!(
            d.text()
                .contains(r#"ExecStart="/opt/school/bin/app" "a \"b\" %%h $$HOME \\""#),
            "{}",
            d.text()
        );
    }

    #[test]
    fn names_and_paths_are_checked() {
        for name in ["", "School", "1app", "a b", "a_b", &"a".repeat(42)] {
            let mut s = spec();
            s.name = name.to_string();
            assert!(s.systemd_unit().is_err(), "{name:?}");
        }
        let mut s = spec();
        s.exec = "app".into();
        assert!(s.systemd_unit().is_err());
        let mut s = spec();
        s.working_dir = Some("relative".into());
        assert!(s.systemd_unit().is_err());
        let mut s = spec();
        s.run_as = Some("Root".into());
        assert!(s.systemd_unit().is_err());
        let mut s = spec();
        s.description = "  ".into();
        assert!(s.systemd_unit().is_err());
    }

    #[test]
    fn the_digest_changes_with_the_text_the_name_and_the_kind() {
        let base = spec().systemd_unit().unwrap().digest();
        assert_eq!(base, spec().systemd_unit().unwrap().digest());
        let mut a = spec();
        a.args.push("--x".into());
        assert_ne!(base, a.systemd_unit().unwrap().digest());
        let mut b = spec();
        b.name = "school-ap2".into();
        assert_ne!(base, b.systemd_unit().unwrap().digest());
        let mut c = spec();
        c.privileged_ports = true;
        assert_ne!(base, c.systemd_unit().unwrap().digest());
        let mut win = spec();
        win.exec = "C:\\school\\app.exe".into();
        win.working_dir = None;
        assert_ne!(base, win.windows_service().unwrap().digest());
        assert_eq!(base.len(), 64);
    }

    #[test]
    fn a_windows_service_is_single_quoted_and_needs_absolute_windows_paths() {
        let mut s = spec();
        s.exec = "C:\\school\\app.exe".into();
        s.working_dir = None;
        s.description = "It's the school app".into();
        let d = s.windows_service().unwrap();
        assert_eq!(d.file_name(), "school-app.ps1");
        assert_eq!(
            d.text(),
            "New-Service -Name 'school-app' -DisplayName 'It''s the school app' -BinaryPathName '\"C:\\school\\app.exe\" \"--port\" \"8080\"' -StartupType Automatic\n"
        );
        // A Unix path is not a Windows path and the other way round.
        assert!(spec().windows_service().is_err());
        let mut unix_for_win = spec();
        unix_for_win.exec = "C:\\x\\a.exe".into();
        assert!(unix_for_win.systemd_unit().is_err());
        let mut quoted = s.clone();
        quoted.args = vec!["a\"b".into()];
        assert!(quoted.windows_service().is_err());
    }

    #[test]
    fn nothing_is_installed_without_the_digest_of_exactly_that_definition() {
        let dir = tempfile::tempdir().unwrap();
        let d = spec().systemd_unit().unwrap();
        let other = {
            let mut s = spec();
            s.args.push("--different".into());
            s.systemd_unit().unwrap()
        };
        for wrong in ["", "x", &other.digest(), &d.digest().to_uppercase()] {
            assert!(
                install_unit_file(&d, wrong, dir.path()).is_err(),
                "{wrong:?}"
            );
        }
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        let path = install_unit_file(&d, &d.digest(), dir.path()).unwrap();
        assert_eq!(path, dir.path().join("school-app.service"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), d.text());
    }

    #[test]
    fn an_existing_file_is_never_overwritten_and_a_windows_service_is_never_installed() {
        let dir = tempfile::tempdir().unwrap();
        let d = spec().systemd_unit().unwrap();
        std::fs::write(dir.path().join("school-app.service"), "mine").unwrap();
        assert!(install_unit_file(&d, &d.digest(), dir.path()).is_err());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("school-app.service")).unwrap(),
            "mine"
        );
        let mut s = spec();
        s.exec = "C:\\school\\app.exe".into();
        s.working_dir = None;
        let w = s.windows_service().unwrap();
        assert!(install_unit_file(&w, &w.digest(), dir.path()).is_err());
        assert!(!dir.path().join("school-app.ps1").exists());
        assert!(install_unit_file(&d, &d.digest(), &dir.path().join("missing")).is_err());
    }

    #[test]
    fn show_carries_the_digest_the_text_and_the_unverified_note() {
        let d = spec().systemd_unit().unwrap();
        let shown = d.show();
        assert!(shown.contains(&d.digest()));
        assert!(shown.contains(d.text()));
        assert!(shown.contains("UNVERIFIED"));
        assert!(!shown.to_lowercase().contains("secret"));
    }
}
