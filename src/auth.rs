use std::fs;
use std::io::{self, IsTerminal, Write};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use hitconn_core::GatewayConfig;
use hitconn_core::auth::{AuthProgress, AuthSession, PasswordLoginProgress, SmsChallenge};
use hitconn_core::web_agent::WebAgent;
use zeroize::Zeroizing;

use crate::paths::StatePaths;
use crate::platform;
use crate::service;

pub async fn login(paths: &StatePaths, force: bool, fallback: bool, no_open: bool) -> Result<()> {
    require_linux()?;
    if unsafe { libc::geteuid() } == 0 && std::env::var_os("SUDO_USER").is_some() {
        bail!("run `hitconn login` as the desktop user, without sudo");
    }
    if !force && session_is_valid(paths).await? {
        println!("Saved authentication is still valid.");
        return Ok(());
    }
    if fallback {
        return browser_login(paths, no_open).await;
    }
    terminal_login(paths).await
}

async fn terminal_login(paths: &StatePaths) -> Result<()> {
    require_terminal()?;
    paths.ensure_private()?;
    let mut session = new_login_session(paths)?;
    session.discover().await?;
    session.begin_password_login().await?;
    let username = prompt_line("账号：")?;
    session.prepare_password_login(&username).await?;
    let password = Zeroizing::new(rpassword::prompt_password("密码：")?);
    let progress = session
        .submit_password_login(&username, password.as_str())
        .await?;
    drop(password);
    let username = finish_password_login(&mut session, progress).await?;
    paths.save_session(&session.snapshot())?;
    println!("Authenticated as {username}.");
    Ok(())
}

async fn browser_login(paths: &StatePaths, no_open: bool) -> Result<()> {
    paths.ensure_private()?;
    let mut session = new_login_session(paths)?;
    session.discover().await?;
    let login_url = session.cas_login_url("hitcas")?;
    let agent = WebAgent::start_login(&session, &paths.web_agent).await?;

    #[cfg(target_os = "linux")]
    crate::platform::linux::trust::ensure_trusted(&paths.web_agent.join("web-agent.der"))?;

    println!("Complete authentication in the system browser.");
    platform::browser::open(login_url.as_str(), no_open)?;
    let ticket = wait_for_ticket(&agent).await?;
    agent.stop().await;
    let progress = session.adopt_web_login_ticket(&ticket).await?;
    let username = finish_auth_progress(&mut session, progress).await?;
    paths.save_session(&session.snapshot())?;
    println!("Authenticated as {username}.");
    Ok(())
}

async fn finish_password_login(
    session: &mut AuthSession,
    progress: PasswordLoginProgress,
) -> Result<String> {
    match progress {
        PasswordLoginProgress::Authenticated { username } => Ok(username),
        PasswordLoginProgress::SmsCodeRequired(challenge) => {
            finish_sms_challenge(session, challenge).await
        }
    }
}

async fn finish_auth_progress(session: &mut AuthSession, progress: AuthProgress) -> Result<String> {
    match progress {
        AuthProgress::Authenticated { username } => Ok(username),
        AuthProgress::SmsCodeRequired(challenge) => finish_sms_challenge(session, challenge).await,
    }
}

async fn finish_sms_challenge(
    session: &mut AuthSession,
    challenge: SmsChallenge,
) -> Result<String> {
    if !challenge.masked_phone_numbers.is_empty() {
        println!(
            "短信验证码已发送至 {}。",
            challenge.masked_phone_numbers.join("、")
        );
    } else if !challenge.tips.trim().is_empty() {
        println!("{}", challenge.tips.trim());
    } else {
        println!("短信验证码已发送。");
    }
    let code = Zeroizing::new(rpassword::prompt_password("六位短信验证码：")?);
    let trust_device = challenge.can_trust_device && prompt_trust_device()?;
    let progress = session.submit_sms_code(code.as_str(), trust_device).await?;
    drop(code);
    match progress {
        AuthProgress::Authenticated { username } => Ok(username),
        AuthProgress::SmsCodeRequired(_) => {
            bail!("authentication requested an unexpected second SMS challenge")
        }
    }
}

fn prompt_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("account name cannot be empty");
    }
    Ok(value)
}

fn prompt_trust_device() -> Result<bool> {
    print!("信任此设备以减少后续二次验证？[Y/n] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ))
}

fn require_terminal() -> Result<()> {
    if io::stdin().is_terminal() && io::stdout().is_terminal() {
        Ok(())
    } else {
        bail!("terminal login requires an interactive TTY; use --fallback for browser login")
    }
}

fn new_login_session(paths: &StatePaths) -> Result<AuthSession> {
    let config = gateway_config()?;
    match paths.load_session() {
        Ok(snapshot) => Ok(AuthSession::with_device_id(config, snapshot.device_id)?),
        Err(_) => {
            let session = AuthSession::new(config)?;
            paths.save_session(&session.snapshot())?;
            Ok(session)
        }
    }
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
    Ok(GatewayConfig::hitsz())
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
