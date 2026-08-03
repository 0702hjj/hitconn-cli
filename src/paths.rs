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
}

impl StatePaths {
    pub fn resolve(explicit: Option<PathBuf>) -> Result<Self> {
        let root = explicit.map_or_else(default_state_dir, Ok)?;
        if !root.is_absolute() {
            bail!("state directory must be an absolute path");
        }
        Ok(Self {
            session: root.join("session.json"),
            web_agent: root.join("web-agent"),
            root,
        })
    }

    pub fn ensure_private(&self) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("cannot create {}", self.root.display()))?;
        set_directory_mode(&self.root)?;
        Ok(())
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

fn default_state_dir() -> Result<PathBuf> {
    if let Some(value) = std::env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(value).join("hitconn"));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".local/state/hitconn"))
}

fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
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

#[cfg(unix)]
fn set_directory_mode(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_directory_mode(_path: &Path) -> Result<()> {
    Ok(())
}
