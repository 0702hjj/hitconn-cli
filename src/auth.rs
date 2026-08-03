use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hitconn_core::auth::{AuthProgress, AuthSession};
use hitconn_core::web_agent::WebAgent;
use hitconn_core::{ClientPlatform, GatewayConfig};

use crate::paths::StatePaths;
use crate::platform;
use crate::service;

pub async fn login(paths: &StatePaths, force: bool, no_open: bool) -> Result<()> {
    require_linux()?;
    if unsafe { libc::geteuid() } == 0 && std::env::var_os("SUDO_USER").is_some() {
        bail!("run `hitconn login` as the desktop user, without sudo");
    }
    if !force && session_is_valid(paths).await? {
        println!("Saved authentication is still valid.");
        return Ok(());
    }
    paths.ensure_private()?;
    let mut session = AuthSession::new(gateway_config()?)?;
    session.discover().await?;
    let login_url = session.cas_login_url("hitcas")?;
    let agent = WebAgent::start_login(&session, &paths.web_agent).await?;

    #[cfg(target_os = "linux")]
    crate::platform::linux::trust::ensure_trusted(&paths.web_agent.join("web-agent.der"))?;

    println!("Complete authentication in the system browser.");
    platform::browser::open(login_url.as_str(), no_open)?;
    let ticket = wait_for_ticket(&agent).await?;
    agent.stop().await;
    let AuthProgress::Authenticated { username } = session.adopt_web_login_ticket(&ticket).await?;
    paths.save_session(&session.snapshot())?;
    println!("Authenticated as {username}.");
    Ok(())
}

pub async fn session_is_valid(paths: &StatePaths) -> Result<bool> {
    let Ok(snapshot) = paths.load_session() else {
        return Ok(false);
    };
    let mut session = AuthSession::from_snapshot(gateway_config()?, snapshot)?;
    match session.resume().await {
        Ok(_) => Ok(true),
        Err(error)
            if matches!(error, hitconn_core::Error::AuthenticationExpired)
                || error
                    .to_string()
                    .contains("authentication session has expired") =>
        {
            Ok(false)
        }
        Err(error) => Err(error).context("could not validate the saved session"),
    }
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

fn require_linux() -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("a local tunnel is currently supported only on Linux; use `hitconn remote`")
    }
}
