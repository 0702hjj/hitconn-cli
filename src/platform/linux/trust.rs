use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

const NICKNAME: &str = "HITSZ Connect Local WebAgent";

pub fn ensure_trusted(certificate: &Path) -> Result<()> {
    require_certutil()?;
    let databases = browser_databases(true)?;
    let mut imported = 0_usize;
    for database in databases {
        match import_certificate(&database, certificate) {
            Ok(()) => imported += 1,
            Err(error) => eprintln!(
                "warning: could not update browser trust at {}: {error:#}",
                database.display()
            ),
        }
    }
    if imported == 0 {
        bail!("could not trust the loopback WebAgent certificate in any browser store");
    }
    Ok(())
}

pub fn remove_trust() -> Result<()> {
    if !certutil_available() {
        return Ok(());
    }
    for database in browser_databases(false)? {
        let _ = Command::new("certutil")
            .args(["-D", "-n", NICKNAME, "-d"])
            .arg(format!("sql:{}", database.display()))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    Ok(())
}

fn browser_databases(initialize_shared: bool) -> Result<Vec<PathBuf>> {
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?);
    let shared = home.join(".pki/nssdb");
    if initialize_shared && !shared.join("cert9.db").exists() {
        fs::create_dir_all(&shared)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&shared, fs::Permissions::from_mode(0o700))?;
        }
        initialize_database(&shared)?;
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
                .filter(|path| path.is_dir() && path.join("cert9.db").exists()),
        );
    }
    databases.sort();
    databases.dedup();
    Ok(databases)
}

fn initialize_database(path: &Path) -> Result<()> {
    let status = Command::new("certutil")
        .args(["-N", "--empty-password", "-d"])
        .arg(format!("sql:{}", path.display()))
        .status()
        .context("cannot initialize the user NSS certificate database")?;
    if status.success() {
        Ok(())
    } else {
        bail!("certutil could not initialize {}", path.display())
    }
}

fn import_certificate(database: &Path, certificate: &Path) -> Result<()> {
    let database_argument = format!("sql:{}", database.display());
    let _ = Command::new("certutil")
        .args(["-D", "-n", NICKNAME, "-d", &database_argument])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let status = Command::new("certutil")
        .args(["-A", "-n", NICKNAME, "-t", "P,,", "-d"])
        .arg(database_argument)
        .arg("-i")
        .arg(certificate)
        .status()
        .context("cannot run certutil")?;
    if status.success() {
        Ok(())
    } else {
        bail!("certutil rejected the local WebAgent leaf certificate")
    }
}

fn require_certutil() -> Result<()> {
    if certutil_available() {
        Ok(())
    } else {
        bail!(
            "`certutil` is required for browser login; install `libnss3-tools` (Debian/Ubuntu) or `nss-tools` (Fedora)"
        )
    }
}

fn certutil_available() -> bool {
    Command::new("certutil")
        .arg("-H")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
