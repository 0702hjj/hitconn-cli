use std::process::Command;

use anyhow::{Result, bail};

pub fn open(url: &str, no_open: bool) -> Result<()> {
    if no_open {
        println!("Open this URL in a browser on this machine:\n{url}");
        return Ok(());
    }
    if try_open(url) {
        Ok(())
    } else {
        println!("Open this URL in a browser on this machine:\n{url}");
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn try_open(url: &str) -> bool {
    Command::new("open")
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(target_os = "linux")]
fn try_open(url: &str) -> bool {
    [("xdg-open", None), ("gio", Some("open"))]
        .into_iter()
        .any(|(program, prefix)| {
            let mut command = Command::new(program);
            if let Some(argument) = prefix {
                command.arg(argument);
            }
            command
                .arg(url)
                .status()
                .is_ok_and(|status| status.success())
        })
}

#[cfg(target_os = "windows")]
fn try_open(url: &str) -> bool {
    Command::new("cmd")
        .args(["/C", "start", "", url])
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn try_open(_url: &str) -> bool {
    false
}

pub fn require_supported() -> Result<()> {
    if cfg!(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "windows"
    )) {
        Ok(())
    } else {
        bail!("opening a browser is unsupported on this platform")
    }
}
