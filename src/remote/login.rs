use std::io::{BufRead, BufReader};
use std::process::ChildStderr;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hitconn_core::web_agent::LOOPBACK_PORTS;

use crate::paths::StatePaths;
use crate::platform::{browser, temporary_trust};
use crate::protocol::AgentEvent;

use super::ssh::{Ssh, free_loopback_port, port_is_free};

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
    let socks_port = free_loopback_port().context("cannot allocate the SSH browser proxy")?;
    let mut child = ssh.spawn_login(port, socks_port)?;
    let diagnostics = child.stderr.take().map(relay_ssh_diagnostics);
    let result = try_port_with_child(&mut child, paths, port, socks_port, no_open);
    if result.is_err() {
        let _ = child.kill();
        let _ = child.wait();
    }
    if let Some(diagnostics) = diagnostics {
        let _ = diagnostics.join();
    }
    result
}

fn try_port_with_child(
    child: &mut std::process::Child,
    paths: &StatePaths,
    port: u16,
    socks_port: u16,
    no_open: bool,
) -> Result<()> {
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
    let browser = browser::open_through_remote(
        &login_url,
        no_open,
        socks_port,
        &paths.cache.join("browser"),
    )?;
    let completion = read_event(&mut lines).context("remote login ended before completion")?;
    let status = child.wait()?;
    browser.close()?;
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

fn relay_ssh_diagnostics(stderr: ChildStderr) -> JoinHandle<()> {
    thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !is_proxied_connection_refusal(&line) {
                eprintln!("{line}");
            }
        }
    })
}

fn is_proxied_connection_refusal(line: &str) -> bool {
    line.starts_with("channel ")
        && line.ends_with(": open failed: connect failed: Connection refused")
}
