use std::fs;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::cli::ConfigAction;
use crate::paths::{StatePaths, write_private};

pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/YinMo19/hitconn-cli/releases/latest/download/manifest.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub manifest_url: String,
    pub channel: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            manifest_url: DEFAULT_MANIFEST_URL.to_owned(),
            channel: "stable".to_owned(),
        }
    }
}

impl Settings {
    pub fn load(paths: &StatePaths) -> Result<Self> {
        match fs::read_to_string(&paths.config) {
            Ok(contents) => toml::from_str(&contents).context("hitconn configuration is invalid"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => {
                Err(error).with_context(|| format!("cannot read {}", paths.config.display()))
            }
        }
    }

    fn save(&self, paths: &StatePaths) -> Result<()> {
        let contents = toml::to_string_pretty(self)?;
        write_private(&paths.config, contents.as_bytes())
    }
}

pub fn execute(action: ConfigAction, paths: &StatePaths) -> Result<()> {
    match action {
        ConfigAction::Path => println!("{}", paths.config.display()),
        ConfigAction::Show => print!("{}", toml::to_string_pretty(&Settings::load(paths)?)?),
        ConfigAction::Set { key, value } => {
            let mut settings = Settings::load(paths)?;
            set(&mut settings, &key, value)?;
            settings.save(paths)?;
            println!("Updated {key} in {}.", paths.config.display());
        }
        ConfigAction::Unset { key } => {
            let mut settings = Settings::load(paths)?;
            let defaults = Settings::default();
            let value = match key.as_str() {
                "manifest_url" => defaults.manifest_url,
                "channel" => defaults.channel,
                _ => bail!("unsupported configuration key {key:?}"),
            };
            set(&mut settings, &key, value)?;
            settings.save(paths)?;
            println!("Restored {key} to its default value.");
        }
    }
    Ok(())
}

fn set(settings: &mut Settings, key: &str, value: String) -> Result<()> {
    if value.trim().is_empty() {
        bail!("configuration values cannot be empty");
    }
    match key {
        "manifest_url" => {
            if !(value.starts_with("https://") || value.starts_with("file://")) {
                bail!("manifest_url must use https:// or file://");
            }
            settings.manifest_url = value;
        }
        "channel" => settings.channel = value,
        _ => bail!("unsupported configuration key {key:?}"),
    }
    Ok(())
}
