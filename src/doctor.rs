use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::Result;
use serde::Serialize;

use crate::paths::StatePaths;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DoctorReport {
    platform: String,
    architecture: String,
    browser: bool,
    user_trust: bool,
    openssh: bool,
    systemd: bool,
    tun: bool,
    iproute2: bool,
    local_tunnel_supported: bool,
    remote_orchestrator_supported: bool,
    saved_session: bool,
}

pub fn print(paths: &StatePaths, json: bool) -> Result<()> {
    let report = inspect(paths);
    if json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        println!("Platform: {} ({})", report.platform, report.architecture);
        line("Default browser", report.browser);
        line("Current-user WebAgent trust", report.user_trust);
        line("OpenSSH client", report.openssh);
        line("Remote orchestration", report.remote_orchestrator_supported);
        line("systemd", report.systemd);
        line("/dev/net/tun", report.tun);
        line("iproute2", report.iproute2);
        line("Local Linux tunnel", report.local_tunnel_supported);
        line("Saved authentication", report.saved_session);
    }
    Ok(())
}

fn inspect(paths: &StatePaths) -> DoctorReport {
    let linux = cfg!(target_os = "linux");
    let browser = browser_available();
    let user_trust = trust_available();
    let openssh = command("ssh", &["-V"]);
    let systemd = linux && command("systemctl", &["--version"]);
    let tun = linux && Path::new("/dev/net/tun").exists();
    let iproute2 = linux && command("ip", &["-Version"]);
    DoctorReport {
        platform: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        browser,
        user_trust,
        openssh,
        systemd,
        tun,
        iproute2,
        local_tunnel_supported: linux && systemd && tun && iproute2,
        remote_orchestrator_supported: browser && user_trust && openssh,
        saved_session: paths.session.is_file(),
    }
}

fn browser_available() -> bool {
    if cfg!(target_os = "macos") {
        Path::new("/usr/bin/open").is_file()
    } else if cfg!(target_os = "linux") {
        command("xdg-open", &["--help"]) || command("gio", &["help", "open"])
    } else if cfg!(target_os = "windows") {
        command("cmd", &["/C", "ver"])
    } else {
        false
    }
}

fn trust_available() -> bool {
    if cfg!(target_os = "macos") {
        Path::new("/usr/bin/security").is_file()
    } else if cfg!(target_os = "linux") {
        command("certutil", &["-H"])
    } else if cfg!(target_os = "windows") {
        command("certutil", &["-?"])
    } else {
        false
    }
}

fn command(program: &str, arguments: &[&str]) -> bool {
    Command::new(program)
        .args(arguments)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn line(label: &str, available: bool) {
    println!(
        "{label}: {}",
        if available { "available" } else { "missing" }
    );
}
