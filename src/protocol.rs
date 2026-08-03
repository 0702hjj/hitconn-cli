use std::io::{self, Write};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    Hello {
        protocol: u16,
        protocol_min: u16,
        protocol_max: u16,
        version: String,
        os: String,
        architecture: String,
        session_present: bool,
    },
    Session {
        protocol: u16,
        valid: bool,
        authentication_required: bool,
    },
    LoginReady {
        protocol: u16,
        login_url: String,
        port: u16,
        certificate_der: String,
    },
    LoginComplete {
        protocol: u16,
        username: String,
    },
    Purged {
        protocol: u16,
    },
}

pub fn emit(event: &AgentEvent) -> Result<()> {
    let mut output = io::stdout().lock();
    serde_json::to_writer(&mut output, event)?;
    output.write_all(b"\n")?;
    output.flush()?;
    Ok(())
}
