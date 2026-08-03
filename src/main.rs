mod auth;
mod cli;
mod daemon;
mod paths;
mod platform;
mod service;
mod status;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command, DaemonAction};
use paths::StatePaths;

#[tokio::main]
async fn main() {
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    let paths = StatePaths::resolve(cli.state_dir)?;
    match cli.command {
        Command::Login => auth::login(&paths).await,
        Command::Logout => auth::logout(&paths),
        Command::Service { action } => service::execute(action, &paths),
        Command::Status => status::print(),
        Command::Logs { follow, lines } => service::logs(follow, lines),
        Command::Daemon {
            action: DaemonAction::Run,
        } => daemon::run(paths).await,
    }
}
