use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

use anyhow::{Context, Result, bail};
use serde::Serialize;

use crate::cli::ServiceAction;
use crate::paths::StatePaths;
use crate::status;

const SERVICE_NAME: &str = "hitconn.service";
const UNIT_PATH: &str = "/etc/systemd/system/hitconn.service";
const INSTALLED_BINARY: &str = "/usr/local/bin/hitconn";
const SERVICE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

pub fn execute(action: ServiceAction, paths: &StatePaths) -> Result<()> {
    require_linux()?;
    match action {
        ServiceAction::Install => install(paths),
        ServiceAction::Uninstall => uninstall(paths),
        ServiceAction::Start => start(paths),
        ServiceAction::Restart => restart(paths),
        ServiceAction::Stop => stop(),
    }
}

pub fn logs(follow: bool, lines: u32) -> Result<()> {
    require_linux()?;
    let mut command = Command::new("journalctl");
    command.args([
        "--unit",
        SERVICE_NAME,
        "--no-pager",
        "--output",
        "short-iso-precise",
        "--lines",
        &lines.to_string(),
    ]);
    if follow {
        command.arg("--follow");
    }
    successful(
        command.status().context("cannot run journalctl")?,
        "journalctl",
    )
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceState {
    pub active: String,
    pub enabled: String,
    pub mode: String,
}

pub fn state() -> Result<ServiceState> {
    require_linux()?;
    let installed = Path::new(UNIT_PATH).exists();
    let active = systemctl_query("is-active").unwrap_or_else(|| "inactive".to_owned());
    let enabled = systemctl_query("is-enabled").unwrap_or_else(|| "not-enabled".to_owned());
    let mode = if installed {
        "installed"
    } else if active == "active" {
        "transient"
    } else {
        "not installed"
    };
    Ok(ServiceState {
        active,
        enabled,
        mode: mode.to_owned(),
    })
}

pub fn stop_if_running() -> Result<()> {
    if !cfg!(target_os = "linux") || !is_active() {
        return Ok(());
    }
    stop()
}

fn install(paths: &StatePaths) -> Result<()> {
    paths.ensure_private()?;
    if !is_root() {
        return reexecute_with_sudo("install", paths);
    }
    require_systemd()?;
    install_binary()?;
    let unit = systemd_unit(&paths.root, &paths.home)?;
    let temporary = format!("{UNIT_PATH}.tmp");
    fs::write(&temporary, unit).context("cannot write the temporary systemd unit")?;
    fs::rename(&temporary, UNIT_PATH).context("cannot install the systemd unit")?;
    systemctl_root(&["daemon-reload"])?;
    systemctl_root(&["enable", "--now", SERVICE_NAME])?;
    println!("Installed and enabled {SERVICE_NAME}.");
    Ok(())
}

fn uninstall(paths: &StatePaths) -> Result<()> {
    if !is_root() {
        return reexecute_with_sudo("uninstall", paths);
    }
    require_systemd()?;
    let _ = systemctl_root(&["disable", "--now", SERVICE_NAME]);
    cleanup_tagged_routes();
    remove_if_present(Path::new(UNIT_PATH))?;
    remove_if_present(Path::new("/etc/systemd/system/hitconn.service.tmp"))?;
    remove_if_present(Path::new(INSTALLED_BINARY))?;
    status::remove();
    systemctl_root(&["daemon-reload"])?;
    let _ = systemctl_root(&["reset-failed", SERVICE_NAME]);
    println!("Uninstalled {SERVICE_NAME}; saved authentication was preserved.");
    Ok(())
}

fn start(paths: &StatePaths) -> Result<()> {
    require_systemd()?;
    if Path::new(UNIT_PATH).exists() {
        systemctl_privileged(&["start", SERVICE_NAME])?;
    } else if is_active() {
        println!("{SERVICE_NAME} is already active.");
        return Ok(());
    } else {
        paths.ensure_private()?;
        start_transient(paths)?;
    }
    println!("Started {SERVICE_NAME}.");
    Ok(())
}

fn restart(paths: &StatePaths) -> Result<()> {
    require_systemd()?;
    if Path::new(UNIT_PATH).exists() {
        systemctl_privileged(&["restart", SERVICE_NAME])?;
        println!("Restarted {SERVICE_NAME}.");
        Ok(())
    } else if is_active() {
        systemctl_privileged(&["stop", SERVICE_NAME])?;
        paths.ensure_private()?;
        start_transient(paths)?;
        println!("Restarted {SERVICE_NAME}.");
        Ok(())
    } else {
        start(paths)
    }
}

fn stop() -> Result<()> {
    require_systemd()?;
    if !is_active() {
        println!("{SERVICE_NAME} is not active.");
        return Ok(());
    }
    systemctl_privileged(&["stop", SERVICE_NAME])?;
    println!("Stopped {SERVICE_NAME}.");
    Ok(())
}

fn start_transient(paths: &StatePaths) -> Result<()> {
    let executable = std::env::current_exe().context("cannot locate the hitconn executable")?;
    let home = paths
        .home
        .to_str()
        .context("user home directory is not valid UTF-8")?;
    let arguments = vec![
        "systemd-run".to_owned(),
        "--unit=hitconn.service".to_owned(),
        "--collect".to_owned(),
        "--service-type=simple".to_owned(),
        "--property=Restart=on-failure".to_owned(),
        "--property=RestartSec=5s".to_owned(),
        "--property=KillSignal=SIGINT".to_owned(),
        "--property=TimeoutStopSec=20s".to_owned(),
        "--property=RuntimeDirectory=hitconn".to_owned(),
        "--property=UMask=0077".to_owned(),
        format!("--setenv=HOME={home}"),
        format!("--setenv=PATH={SERVICE_PATH}"),
        "--description=HITSZ Connect headless tunnel".to_owned(),
        executable.display().to_string(),
        "--state-dir".to_owned(),
        paths.root.display().to_string(),
        "daemon".to_owned(),
        "run".to_owned(),
    ];
    privileged_command(&arguments)
}

fn install_binary() -> Result<()> {
    let source = std::env::current_exe().context("cannot locate the hitconn executable")?;
    let temporary = PathBuf::from(format!("{INSTALLED_BINARY}.tmp"));
    fs::copy(&source, &temporary).with_context(|| {
        format!(
            "cannot copy {} to {}",
            source.display(),
            temporary.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o755))?;
    }
    fs::rename(&temporary, INSTALLED_BINARY).context("cannot install the hitconn binary")?;
    Ok(())
}

fn systemd_unit(state_dir: &Path, home: &Path) -> Result<String> {
    let state_dir = quote_systemd(state_dir)?;
    let home = home
        .to_str()
        .context("user home directory is not valid UTF-8")?;
    let home = quote_systemd_value(&format!("HOME={home}"))?;
    Ok(format!(
        "[Unit]\nDescription=HITSZ Connect headless tunnel\nDocumentation=https://github.com/YinMo19/hitconn-cli\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=simple\nExecStart={INSTALLED_BINARY} --state-dir {state_dir} daemon run\nRestart=on-failure\nRestartSec=5s\nKillSignal=SIGINT\nTimeoutStopSec=20s\nUMask=0077\nRuntimeDirectory=hitconn\nRuntimeDirectoryMode=0755\nEnvironment=PATH={SERVICE_PATH}\nEnvironment={home}\nNoNewPrivileges=true\nCapabilityBoundingSet=CAP_NET_ADMIN CAP_DAC_READ_SEARCH\nAmbientCapabilities=CAP_NET_ADMIN CAP_DAC_READ_SEARCH\nProtectSystem=strict\nProtectHome=read-only\nReadWritePaths=/run/hitconn\nPrivateTmp=true\n\n[Install]\nWantedBy=multi-user.target\n"
    ))
}

fn quote_systemd(path: &Path) -> Result<String> {
    let value = path
        .to_str()
        .context("state directory is not valid UTF-8")?;
    quote_systemd_value(value)
}

fn quote_systemd_value(value: &str) -> Result<String> {
    if value.chars().any(char::is_control) {
        bail!("systemd value contains control characters");
    }
    Ok(format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "$$")
            .replace('%', "%%")
    ))
}

fn reexecute_with_sudo(action: &str, paths: &StatePaths) -> Result<()> {
    let executable = std::env::current_exe().context("cannot locate the hitconn executable")?;
    let arguments = vec![
        executable.display().to_string(),
        "--state-dir".to_owned(),
        paths.root.display().to_string(),
        "service".to_owned(),
        action.to_owned(),
    ];
    privileged_command(&arguments)
}

fn systemctl_privileged(arguments: &[&str]) -> Result<()> {
    let mut command = vec!["systemctl".to_owned()];
    command.extend(arguments.iter().map(|value| (*value).to_owned()));
    privileged_command(&command)
}

fn privileged_command(arguments: &[String]) -> Result<()> {
    let (program, args) = arguments
        .split_first()
        .context("empty privileged command")?;
    let status = if is_root() {
        Command::new(program).args(args).status()
    } else {
        Command::new("sudo")
            .arg("--")
            .arg(program)
            .args(args)
            .status()
    }
    .with_context(|| format!("cannot run {program}"))?;
    successful(status, program)
}

fn systemctl_root(arguments: &[&str]) -> Result<()> {
    let status = Command::new("systemctl").args(arguments).status()?;
    successful(status, "systemctl")
}

fn systemctl_query(action: &str) -> Option<String> {
    let output = Command::new("systemctl")
        .args([action, SERVICE_NAME])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn is_active() -> bool {
    Command::new("systemctl")
        .args(["is-active", "--quiet", SERVICE_NAME])
        .status()
        .is_ok_and(|status| status.success())
}

fn require_systemd() -> Result<()> {
    if Command::new("systemctl")
        .arg("--version")
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
    {
        Ok(())
    } else {
        bail!("systemd is required for service management")
    }
}

fn require_linux() -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("service management is currently implemented only on Linux")
    }
}

fn is_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn cleanup_tagged_routes() {
    let _ = Command::new("ip")
        .args(["-4", "route", "flush", "proto", "static", "metric", "42760"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn successful(status: ExitStatus, program: &str) -> Result<()> {
    if status.success() {
        Ok(())
    } else {
        bail!("{program} exited with {status}")
    }
}
