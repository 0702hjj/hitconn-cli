use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "hitconn", version, about = "Headless HITSZ Connect client")]
pub struct Cli {
    #[arg(long, global = true, hide = true)]
    pub state_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate in the system default browser.
    Login,
    /// Remove the saved browser session and local browser trust entry.
    Logout,
    /// Install or control the background service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Show service and tunnel status.
    Status,
    /// Read daemon logs from the system journal.
    Logs {
        /// Continue printing new journal entries.
        #[arg(short, long)]
        follow: bool,
        /// Number of existing entries to print.
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: u32,
    },
    #[command(hide = true)]
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
}

#[derive(Debug, Clone, Copy, Subcommand)]
pub enum ServiceAction {
    /// Install, enable, and start the systemd service.
    Install,
    /// Stop and remove the installed systemd service and binary.
    Uninstall,
    /// Start an installed or transient service.
    Start,
    /// Restart the current service.
    Restart,
    /// Stop the current service.
    Stop,
}

#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    Run,
}
