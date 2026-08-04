mod deploy;
mod login;
mod ssh;

use std::io::{self, Write};
use std::path::PathBuf;

use anyhow::{Context, Result, bail};

use crate::cli::{RemoteAction, ServiceAction};
use crate::paths::StatePaths;
use crate::protocol::AgentEvent;

use self::ssh::{Ssh, executable_exists};

pub async fn execute(target: String, action: RemoteAction, paths: &StatePaths) -> Result<()> {
    let ssh = Ssh::new(target)?;
    match action {
        RemoteAction::Connect {
            install,
            upgrade,
            artifact,
            fallback,
            no_open,
        } => connect(&ssh, paths, install, upgrade, artifact, fallback, no_open).await,
        RemoteAction::Disconnect => current(&ssh, &["service", "stop"], true),
        RemoteAction::Login {
            force,
            fallback,
            no_open,
        } => {
            deploy::ensure(&ssh, paths, None, false).await?;
            authenticate(&ssh, paths, force, fallback, no_open)
        }
        RemoteAction::Logout => current(&ssh, &["logout"], true),
        RemoteAction::Status { watch, json } => {
            deploy::require_deployed(&ssh)?;
            let mut arguments = vec!["status".to_owned()];
            flag(&mut arguments, watch, "--watch");
            flag(&mut arguments, json, "--json");
            ssh.current(&arguments, false)
        }
        RemoteAction::Logs { follow, lines } => {
            deploy::require_deployed(&ssh)?;
            let mut arguments = vec!["logs".to_owned(), "--lines".to_owned(), lines.to_string()];
            flag(&mut arguments, follow, "--follow");
            ssh.current(&arguments, false)
        }
        RemoteAction::Resources { search, json } => {
            deploy::require_deployed(&ssh)?;
            let mut arguments = vec!["resources".to_owned()];
            if let Some(search) = search {
                arguments.extend(["--search".to_owned(), search]);
            }
            flag(&mut arguments, json, "--json");
            ssh.current(&arguments, false)
        }
        RemoteAction::Doctor { json } => doctor(&ssh, json),
        RemoteAction::Deploy { artifact } => {
            deploy::ensure(&ssh, paths, artifact.as_deref(), true).await?;
            Ok(())
        }
        RemoteAction::Update { artifact } => update(&ssh, paths, artifact).await,
        RemoteAction::Service { action } => {
            deploy::require_deployed(&ssh)?;
            current(&ssh, &["service", service_name(action)], true)
        }
        RemoteAction::Purge { yes } => purge(&ssh, yes),
    }
}

async fn connect(
    ssh: &Ssh,
    paths: &StatePaths,
    install: bool,
    upgrade: bool,
    artifact: Option<PathBuf>,
    fallback: bool,
    no_open: bool,
) -> Result<()> {
    deploy::ensure(
        ssh,
        paths,
        artifact.as_deref(),
        upgrade || artifact.is_some(),
    )
    .await?;
    authenticate(ssh, paths, false, fallback, no_open)?;
    let action = if install { "install" } else { "start" };
    current(ssh, &["service", action], true)
}

fn authenticate(
    ssh: &Ssh,
    paths: &StatePaths,
    force: bool,
    fallback: bool,
    no_open: bool,
) -> Result<()> {
    if !force {
        let output = ssh.current_output(&["agent", "session"])?;
        let event: AgentEvent =
            serde_json::from_slice(&output.stdout).context("remote session response is invalid")?;
        match event {
            AgentEvent::Session { valid: true, .. } => return Ok(()),
            AgentEvent::Session {
                authentication_required: true,
                ..
            } => {}
            AgentEvent::Session { .. } => bail!("remote session could not be validated"),
            _ => bail!("remote agent returned an unexpected session response"),
        }
    }
    if fallback {
        login::run(ssh, paths, no_open)
    } else {
        ssh.current(&["login".to_owned(), "--force".to_owned()], true)
    }
}

fn doctor(ssh: &Ssh, json: bool) -> Result<()> {
    let info = deploy::inspect(ssh)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&info)?);
    } else {
        println!("SSH: reachable with existing OpenSSH configuration");
        println!("Target: {} ({})", info.os, info.architecture);
        println!("systemd: {}", availability(info.systemd));
        println!("/dev/net/tun: {}", availability(info.tun));
        println!(
            "hitconn: {}",
            if info.deployed {
                "deployed"
            } else {
                "not deployed"
            }
        );
    }
    Ok(())
}

async fn update(ssh: &Ssh, paths: &StatePaths, artifact: Option<PathBuf>) -> Result<()> {
    deploy::require_deployed(ssh)?;
    let previous = deploy::previous_link(ssh).context("target has no rollback artifact link")?;
    deploy::ensure(ssh, paths, artifact.as_deref(), true).await?;
    let installed = ssh.status(&["test", "-f", "/etc/systemd/system/hitconn.service"])?;
    let active = ssh.status(&["systemctl", "is-active", "--quiet", "hitconn.service"])?;
    let activation = if installed {
        current(ssh, &["service", "install"], true)
    } else if active {
        current(ssh, &["service", "stop"], true)
            .and_then(|()| current(ssh, &["service", "start"], true))
    } else {
        Ok(())
    };
    if let Err(error) = activation {
        deploy::restore_link(ssh, &previous)?;
        let _ = if installed {
            current(ssh, &["service", "install"], true)
        } else if active {
            current(ssh, &["service", "start"], true)
        } else {
            Ok(())
        };
        return Err(error).context("remote update activation failed; previous artifact restored");
    }
    println!("Remote update completed.");
    Ok(())
}

fn purge(ssh: &Ssh, yes: bool) -> Result<()> {
    if !yes {
        print!("Remove the remote service, binary cache, and authentication state? [y/N] ");
        io::stdout().flush()?;
        let mut answer = String::new();
        io::stdin().read_line(&mut answer)?;
        if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
            println!("Cancelled.");
            return Ok(());
        }
    }
    if executable_exists(ssh)? {
        let _ = current(ssh, &["service", "uninstall"], true);
        let output = ssh.current_output(&["agent", "purge"])?;
        let event: AgentEvent = serde_json::from_slice(&output.stdout)?;
        if !matches!(event, AgentEvent::Purged { .. }) {
            bail!("remote purge returned an unexpected response");
        }
    } else {
        ssh.output(&["rm", "-rf", "--", ".cache/hitconn", ".local/state/hitconn"])?;
    }
    println!("Removed remote hitconn runtime and user state; this cannot be recovered.");
    Ok(())
}

fn current(ssh: &Ssh, arguments: &[&str], tty: bool) -> Result<()> {
    deploy::require_deployed(ssh)?;
    ssh.current(
        &arguments
            .iter()
            .map(|value| (*value).to_owned())
            .collect::<Vec<_>>(),
        tty,
    )
}

fn service_name(action: ServiceAction) -> &'static str {
    match action {
        ServiceAction::Install => "install",
        ServiceAction::Uninstall => "uninstall",
        ServiceAction::Start => "start",
        ServiceAction::Restart => "restart",
        ServiceAction::Stop => "stop",
    }
}

fn flag(arguments: &mut Vec<String>, enabled: bool, value: &str) {
    if enabled {
        arguments.push(value.to_owned());
    }
}

fn availability(value: bool) -> &'static str {
    if value { "available" } else { "missing" }
}
