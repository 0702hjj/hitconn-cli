use std::io::{BufRead, BufReader};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hitconn_core::web_agent::LOOPBACK_PORTS;

use crate::paths::StatePaths;
use crate::platform::{browser, temporary_trust};
use crate::protocol::AgentEvent;

use super::ssh::{Ssh, port_is_free};

pub fn run(ssh: &Ssh, paths: &StatePaths, no_open: bool) -> Result<()> {
    browser::require_supported()?;
    let mut last_error = None;
    for port in LOOPBACK_PORTS {
        if !port_is_free(port)? {
            continue;
        }
        match try_port(ssh, paths, port, no_open) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("no common WebAgent port is available")))
}

fn try_port(ssh: &Ssh, paths: &StatePaths, port: u16, no_open: bool) -> Result<()> {
    let mut child = ssh.spawn_login(port)?;
    let stdout = child
        .stdout
        .take()
        .context("SSH login bridge has no stdout")?;
    let mut lines = BufReader::new(stdout).lines();
    let ready = read_event(&mut lines).context("remote login agent did not become ready")?;
    let AgentEvent::LoginReady {
        protocol,
        login_url,
        port: reported_port,
        certificate_der,
    } = ready
    else {
        let _ = child.kill();
        bail!("remote login agent returned an unexpected event");
    };
    if protocol != crate::protocol::VERSION || reported_port != port {
        let _ = child.kill();
        bail!("remote login agent returned incompatible parameters");
    }
    let certificate = STANDARD
        .decode(certificate_der)
        .context("remote login certificate is not valid base64")?;
    let trust = temporary_trust::install(paths, &certificate)?;
    browser::open(&login_url, no_open)?;
    let completion = read_event(&mut lines).context("remote login ended before completion")?;
    let status = child.wait()?;
    trust.remove()?;
    if !status.success() {
        bail!("remote login agent exited with {status}");
    }
    let AgentEvent::LoginComplete { protocol, username } = completion else {
        bail!("remote login agent returned an unexpected completion event");
    };
    if protocol != crate::protocol::VERSION {
        bail!("remote login completion protocol is incompatible");
    }
    println!("Authenticated remote target as {username}.");
    Ok(())
}

fn read_event(lines: &mut impl Iterator<Item = std::io::Result<String>>) -> Result<AgentEvent> {
    let line = lines
        .next()
        .context("remote agent closed its protocol stream")??;
    serde_json::from_str(&line).context("remote agent emitted invalid protocol JSON")
}
