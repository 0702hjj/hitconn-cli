use std::io;
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};

use anyhow::{Context, Result, bail};

pub const CURRENT_BINARY: &str = ".cache/hitconn/current/hitconn";

#[derive(Debug, Clone)]
pub struct Ssh {
    target: String,
}

impl Ssh {
    pub fn new(target: String) -> Result<Self> {
        if target.is_empty() || target.starts_with('-') || target.chars().any(char::is_control) {
            bail!("invalid OpenSSH target");
        }
        Ok(Self { target })
    }

    pub fn output(&self, arguments: &[&str]) -> Result<Output> {
        let output = self.command(arguments).output().context("cannot run ssh")?;
        if output.status.success() {
            Ok(output)
        } else {
            let message = String::from_utf8_lossy(&output.stderr);
            bail!("ssh target command failed: {}", message.trim());
        }
    }

    pub fn output_text(&self, arguments: &[&str]) -> Result<String> {
        Ok(String::from_utf8(self.output(arguments)?.stdout)?
            .trim()
            .to_owned())
    }

    pub fn status(&self, arguments: &[&str]) -> Result<bool> {
        Ok(self
            .command(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("cannot run ssh")?
            .success())
    }

    pub fn stream(&self, arguments: &[String], tty: bool) -> Result<()> {
        let mut command = Command::new("ssh");
        if tty {
            command.arg("-t");
        }
        let remote = remote_command(arguments.iter().map(String::as_str));
        let status = command
            .arg("--")
            .arg(&self.target)
            .arg(remote)
            .status()
            .context("cannot run ssh")?;
        if status.success() {
            Ok(())
        } else {
            bail!("ssh exited with {status}")
        }
    }

    pub fn current(&self, arguments: &[String], tty: bool) -> Result<()> {
        let mut command = vec![CURRENT_BINARY.to_owned()];
        command.extend_from_slice(arguments);
        self.stream(&command, tty)
    }

    pub fn current_output(&self, arguments: &[&str]) -> Result<Output> {
        let mut command = vec![CURRENT_BINARY];
        command.extend_from_slice(arguments);
        self.output(&command)
    }

    pub fn copy(&self, local: &Path, remote: &str) -> Result<()> {
        if remote.starts_with('-') || remote.chars().any(char::is_control) {
            bail!("invalid remote copy path");
        }
        let destination = format!("{}:{remote}", self.target);
        let status = Command::new("scp")
            .arg("--")
            .arg(local)
            .arg(destination)
            .status()
            .context("cannot run scp")?;
        if status.success() {
            Ok(())
        } else {
            bail!("scp exited with {status}")
        }
    }

    pub fn spawn_login(&self, port: u16, socks_port: u16) -> Result<Child> {
        let forward = format!("{port}:127.0.0.1:{port}");
        let socks = format!("127.0.0.1:{socks_port}");
        let remote = remote_command([CURRENT_BINARY, "agent", "login", &port.to_string()]);
        Command::new("ssh")
            .args([
                "-o",
                "ExitOnForwardFailure=yes",
                "-D",
                &socks,
                "-L",
                &forward,
                "--",
            ])
            .arg(&self.target)
            .arg(remote)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("cannot start SSH login bridge")
    }

    fn command(&self, arguments: &[&str]) -> Command {
        let mut command = Command::new("ssh");
        command
            .arg("--")
            .arg(&self.target)
            .arg(remote_command(arguments.iter().copied()));
        command
    }
}

fn remote_command<'a>(arguments: impl IntoIterator<Item = &'a str>) -> String {
    arguments
        .into_iter()
        .map(quote)
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn executable_exists(ssh: &Ssh) -> Result<bool> {
    ssh.status(&["test", "-x", CURRENT_BINARY])
}

pub fn port_is_free(port: u16) -> io::Result<bool> {
    std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .map(|listener| {
            drop(listener);
            true
        })
        .or_else(|error| {
            if error.kind() == io::ErrorKind::AddrInUse {
                Ok(false)
            } else {
                Err(error)
            }
        })
}

pub fn free_loopback_port() -> io::Result<u16> {
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
    listener.local_addr().map(|address| address.port())
}
