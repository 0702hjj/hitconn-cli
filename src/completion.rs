use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::CommandFactory;
use clap_complete::Shell;

use crate::cli::{Cli, CompletionAction};

const ZSH_COMMAND: &str = "eval \"$(hitconn completion zsh)\"";
const ZSH_BLOCK: &str = "# >>> hitconn completion >>>\nautoload -Uz compinit\n(( $+functions[compdef] )) || compinit\neval \"$(hitconn completion zsh)\"\n# <<< hitconn completion <<<\n";

pub fn execute(shell: Shell, action: Option<CompletionAction>) -> Result<()> {
    match action {
        None => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "hitconn",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Some(CompletionAction::Install) => install(shell),
    }
}

fn install(shell: Shell) -> Result<()> {
    if shell != Shell::Zsh {
        bail!("automatic completion installation currently supports only zsh");
    }
    let zshrc = user_home()?.join(".zshrc");
    let existing = match fs::read(&zshrc) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("cannot read {}", zshrc.display()));
        }
    };
    if contains(&existing, ZSH_COMMAND) {
        println!(
            "zsh completion is already installed in {}.",
            zshrc.display()
        );
        return Ok(());
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&zshrc)
        .with_context(|| format!("cannot update {}", zshrc.display()))?;
    if !existing.is_empty() {
        file.write_all(if existing.ends_with(b"\n") {
            b"\n"
        } else {
            b"\n\n"
        })?;
    }
    file.write_all(ZSH_BLOCK.as_bytes())?;
    file.sync_all()?;
    println!("Installed zsh completion in {}.", zshrc.display());
    println!("Restart zsh or run `source {}`.", zshrc.display());
    Ok(())
}

fn contains(contents: &[u8], needle: &str) -> bool {
    contents
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn user_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| Path::new(path).is_absolute())
        .context("user home directory is not available")
}
