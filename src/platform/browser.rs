use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};

pub struct ProxiedBrowser {
    child: Option<Child>,
    profile: PathBuf,
}

impl ProxiedBrowser {
    pub fn close(mut self) -> Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> Result<()> {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        match fs::remove_dir_all(&self.profile) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => {
                Err(error).with_context(|| format!("cannot remove {}", self.profile.display()))
            }
        }
    }
}

impl Drop for ProxiedBrowser {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

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

pub fn open_through_remote(
    url: &str,
    no_open: bool,
    socks_port: u16,
    cache: &Path,
) -> Result<ProxiedBrowser> {
    let executable = chromium_executable().context(
        "remote login needs Chrome, Chromium, Edge, or Brave so only the login browser can use the SSH proxy",
    )?;
    let profile = create_private_profile(cache)?;
    let mut browser = ProxiedBrowser {
        child: None,
        profile,
    };
    let user_agent = chromium_user_agent(&executable)?;
    let arguments = proxied_arguments(url, socks_port, &browser.profile, &user_agent);
    if no_open {
        println!(
            "Run this isolated browser command while hitconn is waiting:\n{} {}",
            shell_quote(&executable.to_string_lossy()),
            arguments
                .iter()
                .map(|argument| shell_quote(argument))
                .collect::<Vec<_>>()
                .join(" ")
        );
    } else {
        browser.child = Some(
            Command::new(&executable)
                .args(&arguments)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .with_context(|| format!("cannot start {}", executable.display()))?,
        );
    }
    Ok(browser)
}

pub fn cleanup_stale_profiles(cache: &Path) -> Result<()> {
    let entries = match fs::read_dir(cache) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).context("cannot inspect the login browser cache"),
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let Some(pid) = profile_process_id(&path) else {
            continue;
        };
        if !process_is_running(pid) {
            fs::remove_dir_all(&path)
                .with_context(|| format!("cannot remove {}", path.display()))?;
        }
    }
    Ok(())
}

fn profile_process_id(path: &Path) -> Option<u32> {
    path.file_name()?
        .to_str()?
        .strip_prefix("remote-browser-")?
        .split('-')
        .next()?
        .parse()
        .ok()
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    let Ok(pid) = libc::pid_t::try_from(pid) else {
        return false;
    };
    // SAFETY: signal zero checks process existence without delivering a signal.
    unsafe {
        libc::kill(pid, 0) == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}

#[cfg(not(unix))]
fn process_is_running(_pid: u32) -> bool {
    false
}

fn proxied_arguments(url: &str, socks_port: u16, profile: &Path, user_agent: &str) -> Vec<String> {
    vec![
        format!("--user-data-dir={}", profile.display()),
        format!("--user-agent={user_agent}"),
        format!("--proxy-server=socks5://127.0.0.1:{socks_port}"),
        "--proxy-bypass-list=localhost;127.0.0.1".to_owned(),
        "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE localhost, EXCLUDE 127.0.0.1".to_owned(),
        "--disable-features=LocalNetworkAccessChecks".to_owned(),
        "--disable-background-networking".to_owned(),
        "--disable-background-mode".to_owned(),
        "--disable-component-update".to_owned(),
        "--disable-default-apps".to_owned(),
        "--disable-sync".to_owned(),
        "--no-pings".to_owned(),
        "--no-first-run".to_owned(),
        "--no-default-browser-check".to_owned(),
        url.to_owned(),
    ]
}

fn chromium_user_agent(executable: &Path) -> Result<String> {
    let output = Command::new(executable)
        .arg("--version")
        .output()
        .with_context(|| format!("cannot inspect {} version", executable.display()))?;
    if !output.status.success() {
        bail!("{} did not report its version", executable.display());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let version = stdout
        .split_whitespace()
        .find(|value| {
            value.contains('.')
                && value
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
        })
        .context("Chromium browser returned an unrecognized version")?;
    Ok(format!(
        "Mozilla/5.0 ({}) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{version} Safari/537.36 SPCClientType aTrustTray",
        chromium_platform()
    ))
}

fn chromium_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "Macintosh; Intel Mac OS X 10_15_7"
    } else if cfg!(target_os = "windows") {
        "Windows NT 10.0; Win64; x64"
    } else {
        "X11; Linux x86_64"
    }
}

fn create_private_profile(cache: &Path) -> Result<PathBuf> {
    fs::create_dir_all(cache)
        .with_context(|| format!("cannot create browser cache {}", cache.display()))?;
    for attempt in 0..16_u8 {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let profile = cache.join(format!(
            "remote-browser-{}-{nonce}-{attempt}",
            std::process::id()
        ));
        match fs::create_dir(&profile) {
            Ok(()) => {
                set_private_permissions(&profile)?;
                return Ok(profile);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error).with_context(|| format!("cannot create {}", profile.display()));
            }
        }
    }
    bail!("could not allocate a private browser profile")
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<()> {
    Ok(())
}

fn chromium_executable() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
    }
    #[cfg(not(target_os = "macos"))]
    {
        [
            "google-chrome",
            "chromium",
            "chromium-browser",
            "microsoft-edge",
            "brave-browser",
        ]
        .into_iter()
        .find_map(find_in_path)
    }
}

#[cfg(not(target_os = "macos"))]
fn find_in_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
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
