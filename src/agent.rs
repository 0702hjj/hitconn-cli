use std::fs;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hitconn_core::Error as CoreError;
use hitconn_core::auth::{AuthProgress, AuthSession};
use hitconn_core::web_agent::{LOOPBACK_PORTS, WebAgent};

use crate::auth;
use crate::cli::AgentAction;
use crate::paths::StatePaths;
use crate::protocol::{self, AgentEvent};
use crate::status;

pub async fn execute(action: AgentAction, paths: &StatePaths) -> Result<()> {
    require_linux()?;
    match action {
        AgentAction::Handshake => handshake(paths),
        AgentAction::Session => session(paths).await,
        AgentAction::Login { port } => login(paths, port).await,
        AgentAction::Purge => purge(paths),
    }
}

fn handshake(paths: &StatePaths) -> Result<()> {
    protocol::emit(&AgentEvent::Hello {
        protocol: protocol::VERSION,
        protocol_min: protocol::VERSION,
        protocol_max: protocol::VERSION,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        session_present: paths.session.is_file(),
    })
}

async fn session(paths: &StatePaths) -> Result<()> {
    let snapshot = match paths.load_session() {
        Ok(snapshot) => snapshot,
        Err(_) => return emit_session(false, true),
    };
    let mut session = AuthSession::from_snapshot(auth::gateway_config()?, snapshot)?;
    match session.resume().await {
        Ok(_) => emit_session(true, false),
        Err(error) if authentication_error(&error) => emit_session(false, true),
        Err(error) => Err(error).context("could not validate the saved remote session"),
    }
}

async fn login(paths: &StatePaths, port: u16) -> Result<()> {
    if !LOOPBACK_PORTS.contains(&port) {
        bail!("unsupported WebAgent port {port}");
    }
    paths.ensure_private()?;
    let mut session = AuthSession::new(auth::gateway_config()?)?;
    session.discover().await?;
    let login_url = session.cas_login_url("hitcas")?;
    let agent = WebAgent::start_ephemeral_login(&session, port).await?;
    protocol::emit(&AgentEvent::LoginReady {
        protocol: protocol::VERSION,
        login_url: login_url.into(),
        port,
        certificate_der: STANDARD.encode(&agent.info().certificate_der),
    })?;
    let ticket = wait_for_ticket(&agent).await?;
    agent.stop().await;
    let AuthProgress::Authenticated { username } = session.adopt_web_login_ticket(&ticket).await?;
    paths.save_session(&session.snapshot())?;
    protocol::emit(&AgentEvent::LoginComplete {
        protocol: protocol::VERSION,
        username,
    })
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

fn emit_session(valid: bool, authentication_required: bool) -> Result<()> {
    protocol::emit(&AgentEvent::Session {
        protocol: protocol::VERSION,
        valid,
        authentication_required,
    })
}

fn authentication_error(error: &CoreError) -> bool {
    matches!(error, CoreError::AuthenticationExpired)
        || error
            .to_string()
            .contains("authentication session has expired")
}

fn purge(paths: &StatePaths) -> Result<()> {
    ensure_scoped(&paths.root, ".local/state/hitconn")?;
    ensure_scoped(&paths.cache, ".cache/hitconn")?;
    status::remove();
    remove_directory(&paths.root)?;
    remove_directory(&paths.cache)?;
    protocol::emit(&AgentEvent::Purged {
        protocol: protocol::VERSION,
    })
}

fn ensure_scoped(path: &std::path::Path, suffix: &str) -> Result<()> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if !path.is_absolute() || !normalized.ends_with(suffix) {
        bail!("refusing to purge unexpected path {}", path.display());
    }
    Ok(())
}

fn remove_directory(path: &std::path::Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("cannot remove {}", path.display())),
    }
}

fn require_linux() -> Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        bail!("the remote runtime protocol is supported only on Linux")
    }
}
