mod agent;
mod artifact;
mod auth;
mod cli;
mod completion;
mod config;
mod daemon;
mod doctor;
mod paths;
mod platform;
mod protocol;
mod remote;
mod resources;
mod service;
mod status;
mod update;

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
    if let Command::Completion { shell, action } = &cli.command {
        return completion::execute(*shell, *action);
    }
    let paths = StatePaths::resolve(cli.state_dir)?;
    if let Err(error) = platform::temporary_trust::cleanup_stale(&paths) {
        eprintln!("warning: could not remove stale temporary browser trust: {error:#}");
    }
    if let Err(error) = platform::browser::cleanup_stale_profiles(&paths.cache.join("browser")) {
        eprintln!("warning: could not remove a stale login browser profile: {error:#}");
    }
    match cli.command {
        Command::Connect {
            install,
            fallback,
            no_open,
        } => {
            auth::login(&paths, false, fallback, no_open).await?;
            service::execute(
                if install {
                    cli::ServiceAction::Install
                } else {
                    cli::ServiceAction::Start
                },
                &paths,
            )
        }
        Command::Disconnect => service::execute(cli::ServiceAction::Stop, &paths),
        Command::Login {
            force,
            fallback,
            no_open,
        } => auth::login(&paths, force, fallback, no_open).await,
        Command::Logout => auth::logout(&paths),
        Command::Service { action } => service::execute(action, &paths),
        Command::Status { watch, json } => status::print(watch, json),
        Command::Logs { follow, lines } => service::logs(follow, lines),
        Command::Resources { search, json } => {
            resources::print(&paths, search.as_deref(), json).await
        }
        Command::Doctor { json } => doctor::print(&paths, json),
        Command::Update { action } => update::execute(action, &paths).await,
        Command::Config { action } => config::execute(action, &paths),
        Command::Completion { .. } => unreachable!("completion returned before initialization"),
        Command::Remote { target, action } => remote::execute(target, action, &paths).await,
        Command::Agent { action } => agent::execute(action, &paths).await,
        Command::Daemon {
            action: DaemonAction::Run,
        } => daemon::run(paths).await,
    }
}
