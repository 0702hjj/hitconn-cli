use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::artifact::{self, ResolvedArtifact};
use crate::paths::StatePaths;

use super::ssh::{Ssh, executable_exists};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetInfo {
    pub os: String,
    pub architecture: String,
    pub target: String,
    pub systemd: bool,
    pub tun: bool,
    pub deployed: bool,
}

pub fn inspect(ssh: &Ssh) -> Result<TargetInfo> {
    let os = ssh.output_text(&["uname", "-s"])?;
    let architecture = ssh.output_text(&["uname", "-m"])?;
    if !os.eq_ignore_ascii_case("linux") {
        bail!("remote tunnel runtime requires Linux, found {os}");
    }
    let target = match architecture.as_str() {
        "x86_64" | "amd64" => "x86_64-unknown-linux-gnu",
        "aarch64" | "arm64" => "aarch64-unknown-linux-gnu",
        _ => bail!("unsupported remote Linux architecture {architecture}"),
    };
    Ok(TargetInfo {
        os,
        architecture,
        target: target.to_owned(),
        systemd: ssh.status(&["systemctl", "--version"]).unwrap_or(false),
        tun: ssh.status(&["test", "-c", "/dev/net/tun"]).unwrap_or(false),
        deployed: executable_exists(ssh)?,
    })
}

pub async fn ensure(
    ssh: &Ssh,
    paths: &StatePaths,
    local_override: Option<&Path>,
    force: bool,
) -> Result<Option<ResolvedArtifact>> {
    if !force && local_override.is_none() && executable_exists(ssh)? {
        verify_handshake(ssh)?;
        return Ok(None);
    }
    let info = inspect(ssh)?;
    if !info.systemd || !info.tun {
        bail!("target requires systemd and /dev/net/tun");
    }
    let artifact = artifact::resolve(paths, &info.target, local_override).await?;
    stage(ssh, &artifact)?;
    Ok(Some(artifact))
}

pub fn stage(ssh: &Ssh, artifact: &ResolvedArtifact) -> Result<()> {
    let link_target = format!("bin/{}/{}", artifact.version, artifact.target);
    let directory = format!(".cache/hitconn/{link_target}");
    let destination = format!("{directory}/hitconn");
    let temporary = format!("{destination}.tmp");
    ssh.output(&["mkdir", "-p", &directory])?;
    ssh.copy(&artifact.path, &temporary)?;
    ssh.output(&["chmod", "0700", &temporary])?;
    let remote_digest = ssh.output_text(&["sha256sum", &temporary])?;
    let remote_digest = remote_digest
        .split_whitespace()
        .next()
        .context("remote sha256sum returned no digest")?;
    if !remote_digest.eq_ignore_ascii_case(&artifact.sha256) {
        bail!("uploaded artifact SHA-256 does not match the verified local artifact");
    }
    ssh.output(&["mv", "-f", &temporary, &destination])?;
    ssh.output(&["ln", "-sfn", &link_target, ".cache/hitconn/current"])?;
    verify_handshake(ssh)?;
    println!(
        "Deployed hitconn {} for {}.",
        artifact.version, artifact.target
    );
    Ok(())
}

pub fn verify_handshake(ssh: &Ssh) -> Result<()> {
    let output = ssh.current_output(&["agent", "handshake"])?;
    let event: crate::protocol::AgentEvent =
        serde_json::from_slice(&output.stdout).context("remote agent handshake is invalid")?;
    match event {
        crate::protocol::AgentEvent::Hello {
            protocol_min,
            protocol_max,
            ..
        } if (protocol_min..=protocol_max).contains(&crate::protocol::VERSION) => Ok(()),
        _ => bail!("remote agent protocol is incompatible"),
    }
}

pub fn previous_link(ssh: &Ssh) -> Option<String> {
    ssh.output_text(&["readlink", ".cache/hitconn/current"])
        .ok()
        .filter(|value| value.starts_with("bin/") && !value.split('/').any(|part| part == ".."))
}

pub fn restore_link(ssh: &Ssh, previous: &str) -> Result<()> {
    ssh.output(&["ln", "-sfn", previous, ".cache/hitconn/current"])?;
    verify_handshake(ssh)
}

pub fn require_deployed(ssh: &Ssh) -> Result<()> {
    if executable_exists(ssh)? {
        verify_handshake(ssh)
    } else {
        bail!("hitconn is not deployed on this target; run `hitconn remote <target> deploy`")
    }
}
