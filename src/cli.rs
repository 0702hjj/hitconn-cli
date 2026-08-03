use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(
    name = "hitconn",
    version,
    about = "HITSZ Connect client and remote orchestrator"
)]
pub struct Cli {
    #[arg(long, global = true, hide = true)]
    pub state_dir: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Authenticate if needed and start a local transient tunnel.
    Connect {
        /// Install and enable the service instead of using a transient unit.
        #[arg(long)]
        install: bool,
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Stop the local tunnel while preserving authentication.
    Disconnect,
    /// Authenticate in the system default browser.
    Login {
        /// Authenticate even when the saved session is still valid.
        #[arg(long)]
        force: bool,
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Stop the tunnel and remove saved authentication and browser trust.
    Logout,
    /// Install or control the background service.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Show service and tunnel status.
    Status {
        /// Refresh status until interrupted.
        #[arg(short, long)]
        watch: bool,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read daemon logs from the system journal.
    Logs {
        /// Continue printing new journal entries.
        #[arg(short, long)]
        follow: bool,
        /// Number of existing entries to print.
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: u32,
    },
    /// List the effective resources authorized by the controller.
    Resources {
        /// Filter by display name, target, protocol, or port.
        #[arg(long)]
        search: Option<String>,
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Check local runtime, browser, trust, and service prerequisites.
    Doctor {
        /// Emit machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect or apply a release update.
    Update {
        #[command(subcommand)]
        action: UpdateAction,
    },
    /// Inspect or change CLI configuration.
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Generate shell completion output.
    Completion { shell: Shell },
    /// Operate the same hitconn binary on a Linux target over OpenSSH.
    Remote {
        /// OpenSSH host or alias, including user/host syntax when needed.
        target: String,
        #[command(subcommand)]
        action: RemoteAction,
    },
    #[command(hide = true)]
    Agent {
        #[command(subcommand)]
        action: AgentAction,
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
pub enum UpdateAction {
    /// Resolve and verify the newest compatible release metadata.
    Check,
    /// Replace the current CLI with the newest compatible artifact.
    Apply,
}

#[derive(Debug, Subcommand)]
pub enum ConfigAction {
    /// Print the configuration file path.
    Path,
    /// Print effective configuration.
    Show,
    /// Set a supported configuration key.
    Set { key: String, value: String },
    /// Restore a supported configuration key to its default.
    Unset { key: String },
}

#[derive(Debug, Subcommand)]
pub enum RemoteAction {
    /// Deploy if needed, authenticate locally, and start the remote tunnel.
    Connect {
        /// Install and enable the target service instead of using a transient unit.
        #[arg(long)]
        install: bool,
        /// Upgrade an already deployed target before connecting.
        #[arg(long)]
        upgrade: bool,
        /// Use a reviewed local target artifact instead of the release manifest.
        #[arg(long)]
        artifact: Option<PathBuf>,
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Stop the target tunnel while preserving authentication.
    Disconnect,
    /// Run browser authentication for the target.
    Login {
        /// Authenticate even when the remote session is valid.
        #[arg(long)]
        force: bool,
        /// Print the login URL instead of opening a browser.
        #[arg(long)]
        no_open: bool,
    },
    /// Stop the target and remove its saved authentication.
    Logout,
    /// Show remote service and tunnel status without deploying.
    Status {
        #[arg(short, long)]
        watch: bool,
        #[arg(long)]
        json: bool,
    },
    /// Read remote daemon logs without deploying.
    Logs {
        #[arg(short, long)]
        follow: bool,
        #[arg(short = 'n', long, default_value_t = 100)]
        lines: u32,
    },
    /// List effective remote resources without deploying.
    Resources {
        #[arg(long)]
        search: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Check SSH and target prerequisites without deploying.
    Doctor {
        #[arg(long)]
        json: bool,
    },
    /// Download, verify, and stage a compatible target artifact.
    Deploy {
        #[arg(long)]
        artifact: Option<PathBuf>,
    },
    /// Explicitly upgrade the staged binary and active installed service.
    Update {
        #[arg(long)]
        artifact: Option<PathBuf>,
    },
    /// Control the target service without silently deploying or upgrading.
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Remove service, binary cache, authentication, and runtime state.
    Purge {
        /// Skip the destructive confirmation prompt.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum AgentAction {
    Handshake,
    Session,
    Login { port: u16 },
    Purge,
}

#[derive(Debug, Subcommand)]
pub enum DaemonAction {
    Run,
}
