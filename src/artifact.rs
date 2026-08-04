use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::{Signature, VerifyingKey};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::config::Settings;
use crate::paths::{StatePaths, write_private};
use crate::protocol;

const RELEASE_PUBLIC_KEY: [u8; 32] = [
    0x23, 0x58, 0x26, 0x98, 0xd6, 0x7c, 0xdd, 0x74, 0x32, 0x91, 0xfd, 0x1e, 0x95, 0xd4, 0xfa, 0x8f,
    0x2c, 0xac, 0x65, 0x7f, 0x0e, 0x44, 0x14, 0xc0, 0x9d, 0x2b, 0x61, 0xba, 0x04, 0x7d, 0x99, 0xc7,
];
const MAX_ARTIFACT_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SignedManifest {
    signed: String,
    signature: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestPayload {
    schema_version: u16,
    channel: String,
    version: String,
    protocol_min: u16,
    protocol_max: u16,
    artifacts: Vec<ManifestArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ManifestArtifact {
    target: String,
    url: String,
    #[serde(default)]
    mirrors: Vec<String>,
    size: u64,
    sha256: String,
}

#[derive(Debug, Clone)]
pub struct ResolvedArtifact {
    pub path: PathBuf,
    pub version: String,
    pub target: String,
    pub sha256: String,
}

pub async fn resolve(
    paths: &StatePaths,
    target: &str,
    local_override: Option<&Path>,
) -> Result<ResolvedArtifact> {
    if let Some(path) = local_override {
        return resolve_override(path, target);
    }
    let settings = Settings::load(paths)?;
    let envelope = fetch_first([
        settings.manifest_url.as_str(),
        settings.fallback_manifest_url.as_str(),
    ])
    .await?;
    let payload = verify_manifest(&envelope)?;
    if payload.schema_version != 1 || payload.channel != settings.channel {
        bail!("release manifest schema or channel is incompatible");
    }
    if !(payload.protocol_min..=payload.protocol_max).contains(&protocol::VERSION) {
        bail!(
            "release does not support agent protocol {}",
            protocol::VERSION
        );
    }
    let artifact = payload
        .artifacts
        .into_iter()
        .find(|artifact| artifact.target == target)
        .with_context(|| format!("release {} has no artifact for {target}", payload.version))?;
    validate_metadata(&artifact)?;
    paths.ensure_cache()?;
    let destination = paths
        .cache
        .join("artifacts")
        .join(&payload.version)
        .join(target)
        .join("hitconn");
    if destination.is_file() && verify_file(&destination, artifact.size, &artifact.sha256)? {
        return Ok(ResolvedArtifact {
            path: destination,
            version: payload.version,
            target: target.to_owned(),
            sha256: artifact.sha256,
        });
    }
    let bytes = fetch_first(
        std::iter::once(artifact.url.as_str()).chain(artifact.mirrors.iter().map(String::as_str)),
    )
    .await?;
    if bytes.len() as u64 != artifact.size {
        bail!("downloaded artifact size does not match the signed manifest");
    }
    verify_digest(&bytes, &artifact.sha256)?;
    write_private(&destination, &bytes)?;
    make_executable(&destination)?;
    Ok(ResolvedArtifact {
        path: destination,
        version: payload.version,
        target: target.to_owned(),
        sha256: artifact.sha256,
    })
}

pub fn host_target() -> Result<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, architecture) => bail!("unsupported host target {architecture}-{os}"),
    }
}

fn resolve_override(path: &Path, target: &str) -> Result<ResolvedArtifact> {
    let path = path
        .canonicalize()
        .with_context(|| format!("cannot read {}", path.display()))?;
    let metadata = fs::metadata(&path)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_ARTIFACT_BYTES {
        bail!("local artifact has an invalid size");
    }
    let digest = hex::encode(Sha256::digest(fs::read(&path)?));
    Ok(ResolvedArtifact {
        path,
        version: format!("custom-{}", &digest[..12]),
        target: target.to_owned(),
        sha256: digest,
    })
}

fn verify_manifest(bytes: &[u8]) -> Result<ManifestPayload> {
    let envelope: SignedManifest =
        serde_json::from_slice(bytes).context("release manifest envelope is invalid")?;
    let signed = STANDARD
        .decode(envelope.signed)
        .context("release manifest payload is not valid base64")?;
    let signature_bytes = STANDARD
        .decode(envelope.signature)
        .context("release manifest signature is not valid base64")?;
    let signature = Signature::from_slice(&signature_bytes)
        .context("release manifest signature has an invalid length")?;
    VerifyingKey::from_bytes(&RELEASE_PUBLIC_KEY)?
        .verify_strict(&signed, &signature)
        .context("release manifest signature verification failed")?;
    serde_json::from_slice(&signed).context("signed release manifest payload is invalid")
}

fn validate_metadata(artifact: &ManifestArtifact) -> Result<()> {
    if artifact.size == 0 || artifact.size > MAX_ARTIFACT_BYTES {
        bail!("signed artifact size exceeds the local limit");
    }
    if artifact.sha256.len() != 64 || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("signed artifact SHA-256 is invalid");
    }
    if artifact.mirrors.len() > 4 {
        bail!("signed artifact has too many mirrors");
    }
    if std::iter::once(&artifact.url)
        .chain(&artifact.mirrors)
        .any(|url| !(url.starts_with("https://") || url.starts_with("file://")))
    {
        bail!("signed artifact URLs must use https:// or file://");
    }
    Ok(())
}

async fn fetch_first<'a>(urls: impl IntoIterator<Item = &'a str>) -> Result<Vec<u8>> {
    let mut attempted = 0_usize;
    for url in urls {
        attempted += 1;
        if let Ok(bytes) = fetch(url).await {
            return Ok(bytes);
        }
    }
    if attempted == 0 {
        bail!("no download source is configured");
    }
    bail!("download failed from all {attempted} configured sources")
}

async fn fetch(url: &str) -> Result<Vec<u8>> {
    if let Some(path) = url.strip_prefix("file://") {
        return fs::read(path).with_context(|| format!("cannot read {path}"));
    }
    let client = reqwest::Client::builder().build()?;
    let mut request = client.get(url);
    if url.starts_with("https://github.com/")
        && let Some(token) = github_token()
    {
        request = request.bearer_auth(token);
    }
    let response = match request.send().await {
        Ok(response) if response.status().is_success() => response,
        Ok(response) => {
            if let Some(bytes) = github_release_download(url)? {
                return Ok(bytes);
            }
            return Err(response.error_for_status().unwrap_err().into());
        }
        Err(error) => {
            if let Some(bytes) = github_release_download(url)? {
                return Ok(bytes);
            }
            return Err(error.into());
        }
    };
    let length = response.content_length().unwrap_or(0);
    if length > MAX_ARTIFACT_BYTES {
        bail!("download exceeds the local artifact limit");
    }
    let bytes = response.bytes().await?;
    if bytes.len() as u64 > MAX_ARTIFACT_BYTES {
        bail!("download exceeds the local artifact limit");
    }
    Ok(bytes.to_vec())
}

fn github_release_download(url: &str) -> Result<Option<Vec<u8>>> {
    let Some(path) = url.strip_prefix("https://github.com/") else {
        return Ok(None);
    };
    let parts = path.split('/').collect::<Vec<_>>();
    let (owner, repository, release, asset) = match parts.as_slice() {
        [owner, repository, "releases", "latest", "download", asset] => {
            (*owner, *repository, None, *asset)
        }
        [owner, repository, "releases", "download", release, asset] => {
            (*owner, *repository, Some(*release), *asset)
        }
        _ => return Ok(None),
    };
    for value in [owner, repository, asset].into_iter().chain(release) {
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            bail!("GitHub release URL contains an unsupported component");
        }
    }
    let mut command = Command::new("gh");
    command.args(["release", "download"]);
    if let Some(release) = release {
        command.arg(release);
    }
    let output = command
        .args([
            "--repo",
            &format!("{owner}/{repository}"),
            "--pattern",
            asset,
            "--output",
            "-",
        ])
        .stderr(Stdio::piped())
        .output()
        .context("private GitHub release requires an authenticated `gh` CLI")?;
    if !output.status.success() {
        let message = String::from_utf8_lossy(&output.stderr);
        bail!(
            "gh could not download the private release asset: {}",
            message.trim()
        );
    }
    if output.stdout.len() as u64 > MAX_ARTIFACT_BYTES {
        bail!("download exceeds the local artifact limit");
    }
    Ok(Some(output.stdout))
}

fn github_token() -> Option<String> {
    let output = Command::new("gh")
        .args(["auth", "token"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn verify_file(path: &Path, size: u64, digest: &str) -> Result<bool> {
    let bytes = fs::read(path)?;
    Ok(bytes.len() as u64 == size
        && hex::encode(Sha256::digest(&bytes)).eq_ignore_ascii_case(digest))
}

fn verify_digest(bytes: &[u8], expected: &str) -> Result<()> {
    let actual = hex::encode(Sha256::digest(bytes));
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        bail!("downloaded artifact SHA-256 does not match the signed manifest")
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}
