use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x509_parser::extensions::GeneralName;
use x509_parser::parse_x509_certificate;

use crate::paths::{StatePaths, write_private};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CleanupRecord {
    fingerprint: String,
    nickname: String,
    installed_at_unix_seconds: u64,
}

pub struct TrustGuard {
    paths: StatePaths,
    record: Option<CleanupRecord>,
}

impl TrustGuard {
    pub fn remove(mut self) -> Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<()> {
        let Some(record) = self.record.take() else {
            return Ok(());
        };
        remove_platform_trust(&record)?;
        remove_if_present(&self.paths.trust_cleanup)?;
        Ok(())
    }
}

impl Drop for TrustGuard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

pub fn install(paths: &StatePaths, certificate: &[u8]) -> Result<TrustGuard> {
    validate_leaf(certificate)?;
    paths.ensure_private()?;
    let fingerprint = hex::encode_upper(Sha256::digest(certificate));
    let nickname = format!("HITSZ Connect Remote {fingerprint}");
    let record = CleanupRecord {
        fingerprint,
        nickname,
        installed_at_unix_seconds: now(),
    };
    write_private(&paths.trust_cleanup, &serde_json::to_vec(&record)?)?;
    let certificate_path = paths.root.join("remote-web-agent.der");
    write_private(&certificate_path, certificate)?;
    let install_result = install_platform_trust(&certificate_path, &record.nickname);
    let _ = remove_if_present(&certificate_path);
    if let Err(error) = install_result {
        let _ = remove_platform_trust(&record);
        let _ = remove_if_present(&paths.trust_cleanup);
        return Err(error);
    }
    Ok(TrustGuard {
        paths: paths.clone(),
        record: Some(record),
    })
}

pub fn cleanup_stale(paths: &StatePaths) -> Result<bool> {
    let bytes = match fs::read(&paths.trust_cleanup) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("cannot read temporary trust cleanup record"),
    };
    let record: CleanupRecord =
        serde_json::from_slice(&bytes).context("temporary trust cleanup record is invalid")?;
    remove_platform_trust(&record)?;
    remove_if_present(&paths.trust_cleanup)?;
    Ok(true)
}

fn validate_leaf(certificate: &[u8]) -> Result<()> {
    let (remaining, parsed) = parse_x509_certificate(certificate)
        .map_err(|_| anyhow::anyhow!("invalid WebAgent certificate"))?;
    if !remaining.is_empty() || parsed.is_ca() || !parsed.validity().is_valid() {
        bail!("remote WebAgent certificate is not a current non-CA leaf");
    }
    let lifetime =
        parsed.validity().not_after.timestamp() - parsed.validity().not_before.timestamp();
    if !(1..=3_600).contains(&lifetime) {
        bail!("remote WebAgent certificate lifetime exceeds one hour");
    }
    let san = parsed
        .subject_alternative_name()?
        .context("remote WebAgent certificate has no subjectAltName")?;
    if san.value.general_names.as_slice() != [GeneralName::IPAddress([127, 0, 0, 1].as_ref())] {
        bail!("remote WebAgent certificate is not constrained to 127.0.0.1");
    }
    let usage = parsed
        .extended_key_usage()?
        .context("remote WebAgent certificate has no extended key usage")?;
    if !usage.value.server_auth
        || usage.value.any
        || usage.value.client_auth
        || usage.value.code_signing
        || usage.value.email_protection
        || usage.value.time_stamping
        || usage.value.ocsp_signing
        || !usage.value.other.is_empty()
    {
        bail!("remote WebAgent certificate is not server-auth-only");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn install_platform_trust(certificate: &Path, _nickname: &str) -> Result<()> {
    let keychain = login_keychain()?;
    successful(
        Command::new("security")
            .args([
                "add-trusted-cert",
                "-r",
                "trustAsRoot",
                "-p",
                "ssl",
                "-s",
                "127.0.0.1",
                "-k",
            ])
            .arg(&keychain)
            .arg(certificate)
            .status()
            .context("cannot run macOS security tool")?,
        "security add-trusted-cert",
    )
}

#[cfg(target_os = "macos")]
fn remove_platform_trust(record: &CleanupRecord) -> Result<()> {
    let keychain = login_keychain()?;
    let deleted = Command::new("security")
        .args(["delete-certificate", "-Z", &record.fingerprint, "-t"])
        .arg(&keychain)
        .stdout(Stdio::null())
        .status()
        .context("cannot remove temporary macOS trust")?;
    if deleted.success() {
        return Ok(());
    }
    let still_present = Command::new("security")
        .args(["find-certificate", "-Z", &record.fingerprint])
        .arg(keychain)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if still_present {
        bail!("security delete-certificate exited with {deleted}");
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn login_keychain() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join("Library/Keychains/login.keychain-db"))
}

#[cfg(target_os = "linux")]
fn install_platform_trust(certificate: &Path, nickname: &str) -> Result<()> {
    require_certutil()?;
    let databases = linux_databases(true)?;
    if databases.is_empty() {
        bail!("no NSS browser certificate database is available");
    }
    for database in databases {
        let database = format!("sql:{}", database.display());
        let status = Command::new("certutil")
            .args(["-A", "-n", nickname, "-t", "P,,", "-d", &database, "-i"])
            .arg(certificate)
            .status()?;
        if !status.success() {
            bail!("certutil rejected the temporary WebAgent leaf");
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn remove_platform_trust(record: &CleanupRecord) -> Result<()> {
    if !certutil_available() {
        return Ok(());
    }
    for database in linux_databases(false)? {
        let _ = Command::new("certutil")
            .args(["-D", "-n", &record.nickname, "-d"])
            .arg(format!("sql:{}", database.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn linux_databases(initialize_shared: bool) -> Result<Vec<PathBuf>> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let shared = home.join(".pki/nssdb");
    if initialize_shared && !shared.join("cert9.db").exists() {
        fs::create_dir_all(&shared)?;
        successful(
            Command::new("certutil")
                .args(["-N", "--empty-password", "-d"])
                .arg(format!("sql:{}", shared.display()))
                .status()?,
            "certutil -N",
        )?;
    }
    let mut databases = Vec::new();
    if shared.join("cert9.db").exists() {
        databases.push(shared);
    }
    for root in [
        home.join(".mozilla/firefox"),
        home.join("snap/firefox/common/.mozilla/firefox"),
    ] {
        let Ok(entries) = fs::read_dir(root) else {
            continue;
        };
        databases.extend(
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.join("cert9.db").exists()),
        );
    }
    databases.sort();
    databases.dedup();
    Ok(databases)
}

#[cfg(target_os = "linux")]
fn require_certutil() -> Result<()> {
    if certutil_available() {
        Ok(())
    } else {
        bail!("certutil is required for browser trust")
    }
}

#[cfg(target_os = "linux")]
fn certutil_available() -> bool {
    Command::new("certutil")
        .arg("-H")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

#[cfg(target_os = "windows")]
fn install_platform_trust(certificate: &Path, _nickname: &str) -> Result<()> {
    successful(
        Command::new("certutil")
            .args(["-user", "-addstore", "TrustedPeople"])
            .arg(certificate)
            .status()?,
        "certutil -addstore",
    )
}

#[cfg(target_os = "windows")]
fn remove_platform_trust(record: &CleanupRecord) -> Result<()> {
    successful(
        Command::new("certutil")
            .args(["-user", "-delstore", "TrustedPeople", &record.fingerprint])
            .status()?,
        "certutil -delstore",
    )
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn install_platform_trust(_certificate: &Path, _nickname: &str) -> Result<()> {
    bail!("temporary browser trust is unsupported on this platform")
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn remove_platform_trust(_record: &CleanupRecord) -> Result<()> {
    Ok(())
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn successful(status: std::process::ExitStatus, operation: &str) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("{operation} exited with {status}")
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
