use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::artifact;
use crate::cli::UpdateAction;
use crate::paths::StatePaths;

pub async fn execute(action: UpdateAction, paths: &StatePaths) -> Result<()> {
    let target = artifact::host_target()?;
    let resolved = artifact::resolve(paths, target, None).await?;
    match action {
        UpdateAction::Check => {
            println!("Current version: {}", env!("CARGO_PKG_VERSION"));
            println!("Available version: {}", resolved.version);
            println!("Target: {}", resolved.target);
            println!("Verified SHA-256: {}", resolved.sha256);
            Ok(())
        }
        UpdateAction::Apply => apply(&resolved.path, &resolved.version),
    }
}

fn apply(artifact: &Path, version: &str) -> Result<()> {
    if cfg!(target_os = "windows") {
        bail!("in-place update is not yet supported on Windows");
    }
    let executable = std::env::current_exe().context("cannot locate the current executable")?;
    if writable_parent(&executable) {
        let temporary = executable.with_extension("hitconn-update");
        fs::copy(artifact, &temporary)?;
        make_executable(&temporary)?;
        fs::rename(&temporary, &executable)?;
    } else {
        let status = Command::new("sudo")
            .arg("--")
            .arg("install")
            .args(["-m", "0755"])
            .arg(artifact)
            .arg(&executable)
            .status()
            .context("cannot install the update with sudo")?;
        if !status.success() {
            bail!("sudo install exited with {status}");
        }
    }
    println!("Updated hitconn to {version}.");
    Ok(())
}

fn writable_parent(executable: &Path) -> bool {
    executable
        .parent()
        .and_then(|parent| fs::metadata(parent).ok())
        .is_some_and(|metadata| !metadata.permissions().readonly())
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}
