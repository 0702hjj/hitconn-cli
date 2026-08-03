use std::time::Duration;

use anyhow::{Result, bail};
#[cfg(target_os = "linux")]
use hitconn_core::L3PacketLoopEvent;
use hitconn_core::auth::AuthSession;
use hitconn_core::{Client, Error as CoreError, L3PacketLoop};

use crate::auth;
use crate::paths::StatePaths;
use crate::status::{self, DaemonStatus};

pub async fn run(paths: StatePaths) -> Result<()> {
    if !cfg!(target_os = "linux") {
        bail!("the tunnel daemon is currently implemented only on Linux");
    }
    tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .init();
    tracing::info!("hitconn daemon starting");
    let mut retry_delay = Duration::from_secs(5);
    loop {
        match connect(&paths).await {
            Ok(packet_loop) => {
                retry_delay = Duration::from_secs(5);
                match run_connected(packet_loop).await? {
                    RunOutcome::Stopped => break,
                    RunOutcome::Restart => {
                        write_state(DaemonStatus::new(
                            "reconnecting",
                            Some("tunnel policy or transport requires a restart".to_owned()),
                        ));
                    }
                    RunOutcome::AuthenticationRequired => {
                        write_state(DaemonStatus::new(
                            "authentication_required",
                            Some("run `hitconn login` as the desktop user".to_owned()),
                        ));
                    }
                }
            }
            Err(ConnectError::Authentication(message)) => {
                write_state(DaemonStatus::new("authentication_required", Some(message)));
                retry_delay = Duration::from_secs(10);
            }
            Err(ConnectError::Transient(error)) => {
                tracing::warn!(error = %error, "tunnel connection failed");
                write_state(DaemonStatus::new("reconnecting", Some(error.to_string())));
                retry_delay = (retry_delay * 2).min(Duration::from_secs(60));
            }
        }
        if wait_or_stop(retry_delay).await? {
            break;
        }
    }
    status::remove();
    tracing::info!("hitconn daemon stopped");
    Ok(())
}

async fn connect(paths: &StatePaths) -> std::result::Result<L3PacketLoop, ConnectError> {
    let snapshot = paths
        .load_session()
        .map_err(|error| ConnectError::Authentication(error.to_string()))?;
    let config = auth::gateway_config().map_err(ConnectError::Transient)?;
    let mut session = AuthSession::from_snapshot(config, snapshot).map_err(classify_core)?;
    session.resume().await.map_err(classify_core)?;
    let client = Client::from_authenticated(session)
        .await
        .map_err(classify_core)?;
    client
        .into_l3_packet_loop(false)
        .await
        .map_err(classify_core)
}

#[cfg(target_os = "linux")]
async fn run_connected(mut packet_loop: L3PacketLoop) -> Result<RunOutcome> {
    use crate::platform::linux::NetworkAdapter;

    let settings = packet_loop.network_settings().clone();
    let mut network = NetworkAdapter::create(&settings)?;
    let mut current = DaemonStatus::new("connected", None);
    current.virtual_address = Some(settings.address.clone());
    current.route_count = settings.included_routes.len();
    write_state(current.clone());
    tracing::info!(
        virtual_address = %settings.address,
        routes = settings.included_routes.len(),
        "tunnel connected"
    );

    let mut buffer = vec![0_u8; 65_535];
    let mut status_interval = tokio::time::interval(Duration::from_secs(2));
    status_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let outcome = loop {
        tokio::select! {
            result = network.receive(&mut buffer) => {
                let length = match result {
                    Ok(length) => length,
                    Err(error) => {
                        tracing::warn!(error = %error, "TUN receive failed");
                        break RunOutcome::Restart;
                    }
                };
                let packet = buffer[..length].to_vec();
                match packet_loop.send_packet(packet).await {
                    Ok(()) => {
                        current.packets_sent += 1;
                        current.bytes_sent += length as u64;
                    }
                    Err(error) => tracing::debug!(error = %error, "dropped an unauthorized or malformed packet"),
                }
            }
            event = packet_loop.receive_event() => match event {
                Ok(L3PacketLoopEvent::Packet(packet)) => {
                    if let Err(error) = network.send(&packet).await {
                        tracing::warn!(error = %error, "TUN send failed");
                        break RunOutcome::Restart;
                    }
                    current.packets_received += 1;
                    current.bytes_received += packet.len() as u64;
                }
                Ok(L3PacketLoopEvent::PolicyUpdated(settings)) => {
                    if let Err(error) = network.apply(&settings) {
                        tracing::warn!(error = %error, "policy update could not be installed");
                        break RunOutcome::Restart;
                    }
                    current.route_count = settings.included_routes.len();
                    tracing::info!(routes = current.route_count, "applied refreshed policy");
                }
                Ok(L3PacketLoopEvent::TunnelRestartRequired) => break RunOutcome::Restart,
                Ok(L3PacketLoopEvent::ReauthenticationRequired) => {
                    break RunOutcome::AuthenticationRequired;
                }
                Err(CoreError::AuthenticationExpired) => break RunOutcome::AuthenticationRequired,
                Err(error) => {
                    tracing::warn!(error = %error, "core packet loop failed");
                    break RunOutcome::Restart;
                }
            },
            _ = status_interval.tick() => {
                current.touch();
                write_state(current.clone());
            }
            result = shutdown_signal() => {
                result?;
                break RunOutcome::Stopped;
            }
        }
    };

    network.cleanup();
    if let Err(error) = packet_loop.shutdown().await {
        tracing::debug!(error = %error, "core shutdown returned an error");
    }
    Ok(outcome)
}

#[cfg(not(target_os = "linux"))]
async fn run_connected(_packet_loop: L3PacketLoop) -> Result<RunOutcome> {
    bail!("the tunnel daemon is currently implemented only on Linux")
}

async fn wait_or_stop(delay: Duration) -> Result<bool> {
    tokio::select! {
        () = tokio::time::sleep(delay) => Ok(false),
        result = shutdown_signal() => {
            result?;
            Ok(true)
        }
    }
}

fn classify_core(error: CoreError) -> ConnectError {
    if matches!(error, CoreError::AuthenticationExpired)
        || error
            .to_string()
            .contains("authentication session has expired")
    {
        ConnectError::Authentication("saved login expired; run `hitconn login` again".to_owned())
    } else {
        ConnectError::Transient(error.into())
    }
}

fn write_state(value: DaemonStatus) {
    if let Err(error) = status::write(&value) {
        tracing::warn!(error = %error, "could not update daemon status");
    }
}

#[cfg(unix)]
async fn shutdown_signal() -> Result<()> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut terminate = signal(SignalKind::terminate())?;
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        _ = terminate.recv() => {}
    }
    Ok(())
}

#[cfg(not(unix))]
async fn shutdown_signal() -> Result<()> {
    tokio::signal::ctrl_c().await?;
    Ok(())
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum RunOutcome {
    Stopped,
    Restart,
    AuthenticationRequired,
}

enum ConnectError {
    Authentication(String),
    Transient(anyhow::Error),
}
