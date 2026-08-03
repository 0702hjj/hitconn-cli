use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::service;

pub const RUNTIME_DIRECTORY: &str = "/run/hitconn";
const STATUS_FILE: &str = "/run/hitconn/status.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    pub state: String,
    pub updated_at_unix_seconds: u64,
    pub virtual_address: Option<String>,
    pub route_count: usize,
    pub packets_sent: u64,
    pub packets_received: u64,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub message: Option<String>,
}

impl DaemonStatus {
    pub fn new(state: impl Into<String>, message: Option<String>) -> Self {
        Self {
            state: state.into(),
            updated_at_unix_seconds: now(),
            virtual_address: None,
            route_count: 0,
            packets_sent: 0,
            packets_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            message,
        }
    }

    #[cfg(target_os = "linux")]
    pub fn touch(&mut self) {
        self.updated_at_unix_seconds = now();
    }
}

pub fn write(value: &DaemonStatus) -> Result<()> {
    let directory = Path::new(RUNTIME_DIRECTORY);
    fs::create_dir_all(directory).context("cannot create daemon runtime directory")?;
    let temporary = directory.join("status.json.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o644))?;
    }
    fs::rename(temporary, STATUS_FILE)?;
    Ok(())
}

pub fn remove() {
    let _ = fs::remove_file(STATUS_FILE);
}

pub fn print() -> Result<()> {
    service::print_service_state()?;
    match fs::read(STATUS_FILE) {
        Ok(bytes) => {
            let value: DaemonStatus =
                serde_json::from_slice(&bytes).context("daemon status file is invalid")?;
            println!("Tunnel: {}", value.state);
            if let Some(address) = value.virtual_address {
                println!("Virtual address: {address}");
            }
            println!("Routes: {}", value.route_count);
            println!(
                "Traffic: sent {} packets / {} bytes, received {} packets / {} bytes",
                value.packets_sent, value.bytes_sent, value.packets_received, value.bytes_received
            );
            if let Some(message) = value.message {
                println!("Detail: {message}");
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!("Tunnel: no daemon status available");
        }
        Err(error) => return Err(error).context("cannot read daemon status"),
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
