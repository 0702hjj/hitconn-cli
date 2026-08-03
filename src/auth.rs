use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hitconn_core::auth::{AuthProgress, AuthSession};
use hitconn_core::web_agent::WebAgent;
use hitconn_core::{ClientPlatform, GatewayConfig};

use crate::paths::StatePaths;
use crate::service;

pub async fn login(paths: &StatePaths) -> Result<()> {
    require_linux()?;
    if unsafe { libc::geteuid() } == 0 && std::env::var_os("SUDO_USER").is_some() {
        bail!("run `hitconn login` as the desktop user, without sudo");
    }
    paths.ensure_private()?;
    let mut session = AuthSession::new(gateway_config()?)?;
    session.discover().await?;
    let login_url = session.cas_login_url("hitcas")?;
    let agent = WebAgent::start_login(&session, &paths.web_agent).await?;

    #[cfg(target_os = "linux")]
    crate::platform::linux::trust::ensure_trusted(&paths.web_agent.join("web-agent.der"))?;

    println!("Complete authentication in the system browser.");
    open_browser(login_url.as_str())?;
    let ticket = wait_for_ticket(&agent).await?;
    agent.stop().await;
    let AuthProgress::Authenticated { username } = session.adopt_web_login_ticket(&ticket).await?;
    paths.save_session(&session.snapshot())?;
    println!("Authenticated as {username}.");
    println!("Run `hitconn service start` to connect.");
    Ok(())
}

pub fn logout(paths: &StatePaths) -> Result<()> {
    require_linux()?;
    service::stop_if_running()?;
    #[cfg(target_os = "linux")]
    crate::platform::linux::trust::remove_trust()?;
    let removed = paths.remove_session()?;
    if paths.web_agent.exists() {
        fs::remove_dir_all(&paths.web_agent)
            .with_context(|| format!("cannot remove {}", paths.web_agent.display()))?;
    }
    println!(
        "{}",
        if removed {
            "Saved authentication and local WebAgent identity removed."
        } else {
            "No saved authentication was present."
        }
    );
    Ok(())
}

pub fn gateway_config() -> Result<GatewayConfig> {
    Ok(GatewayConfig::new(
        GatewayConfig::HITSZ_GATEWAY.parse()?,
        ClientPlatform::Linux,
    )?)
}

async fn wait_for_ticket(agent: &WebAgent) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(600), async {
        loop {
            if let Some(ticket) = agent.take_login_ticket() {
                return Ok(ticket);
            }
            tokio::select! {
                () = tokio::time::sleep(Duration::from_millis(200)) => {}
                result = tokio::signal::ctrl_c() => {
                    result?;
                    bail!("browser login cancelled");
                }
            }
        }
    })
    .await
    .context("browser login timed out after 10 minutes")?
}

#[cfg(target_os = "linux")]
fn open_browser(url: &str) -> Result<()> {
    use std::process::Command;

    for (program, prefix) in [("xdg-open", None), ("gio", Some("open"))] {
        let mut command = Command::new(program);
        if let Some(argument) = prefix {
            command.arg(argument);
        }
        match command.arg(url).status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(_) | Err(_) => {}
        }
    }
    println!("Open this URL in a browser on this machine:\n{url}");
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn open_browser(_url: &str) -> Result<()> {
    bail!("browser login is currently implemented only on Linux")
}

fn require_linux() -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("hitconn-cli currently supports Linux only")
    }
}
