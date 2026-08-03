use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use hitconn_core::auth::AuthSessionSnapshot;

#[derive(Debug, Clone)]
pub struct StatePaths {
    pub root: PathBuf,
    pub session: PathBuf,
    pub web_agent: PathBuf,
    pub trust_cleanup: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
}

impl StatePaths {
    pub fn resolve(explicit: Option<PathBuf>) -> Result<Self> {
        let root = explicit.map_or_else(default_state_dir, Ok)?;
        if !root.is_absolute() {
            bail!("state directory must be an absolute path");
        }
        let config = default_config_dir()?.join("config.toml");
        let cache = default_cache_dir()?;
        Ok(Self {
            session: root.join("session.json"),
            web_agent: root.join("web-agent"),
            trust_cleanup: root.join("trust-cleanup.json"),
            root,
            config,
            cache,
        })
    }

    pub fn ensure_private(&self) -> Result<()> {
        create_private_directory(&self.root)
    }

    pub fn ensure_cache(&self) -> Result<()> {
        create_private_directory(&self.cache)
    }

    pub fn save_session(&self, snapshot: &AuthSessionSnapshot) -> Result<()> {
        self.ensure_private()?;
        let encoded = serde_json::to_vec(snapshot).context("cannot encode authentication state")?;
        let temporary = self.root.join("session.json.tmp");
        write_private(&temporary, &encoded)?;
        fs::rename(&temporary, &self.session).with_context(|| {
            format!("cannot replace saved session at {}", self.session.display())
        })?;
        Ok(())
    }

    pub fn load_session(&self) -> Result<AuthSessionSnapshot> {
        let encoded = fs::read(&self.session).with_context(|| {
            format!(
                "no saved login at {}; run `hitconn login` first",
                self.session.display()
            )
        })?;
        serde_json::from_slice(&encoded).context("saved authentication state is invalid")
    }

    pub fn remove_session(&self) -> Result<bool> {
        match fs::remove_file(&self.session) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error).context("cannot remove saved authentication state"),
        }
    }
}

pub fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(parent) = path.parent() {
        create_private_directory(parent)?;
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot write {}", path.display()))?;
    file.write_all(contents)?;
    file.sync_all()?;
    Ok(())
}

fn default_state_dir() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(value).join("hitconn"));
    }
    Ok(user_home()?.join(".local/state/hitconn"))
}

fn default_config_dir() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(value).join("hitconn"));
    }
    Ok(user_home()?.join(".config/hitconn"))
}

fn default_cache_dir() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(value).join("hitconn"));
    }
    Ok(user_home()?.join(".cache/hitconn"))
}

fn user_home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("user home directory is not available")
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("cannot create {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
