use anyhow::Result;
use hitconn_core::auth::AuthSessionSnapshot;

#[cfg(target_os = "macos")]
pub fn load() -> Result<Option<AuthSessionSnapshot>> {
    use std::process::{Command, Stdio};

    use anyhow::Context;
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;
    use serde::Deserialize;
    use zeroize::{Zeroize, Zeroizing};

    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct AppSession {
        opaque_session: String,
    }

    let output = Command::new("/usr/bin/security")
        .args([
            "find-generic-password",
            "-s",
            "site.yinmo.hitsz-conn",
            "-a",
            "authenticated-session",
            "-w",
        ])
        .stderr(Stdio::null())
        .output()
        .context("cannot query the HITSZ Connect Keychain session")?;
    if !output.status.success() {
        return Ok(None);
    }
    let keychain_data = Zeroizing::new(output.stdout);
    let mut app: AppSession = serde_json::from_slice(&keychain_data)
        .context("the HITSZ Connect Keychain session is invalid")?;
    let decoded = Zeroizing::new(
        STANDARD
            .decode(app.opaque_session.trim())
            .context("the HITSZ Connect opaque session is invalid")?,
    );
    app.opaque_session.zeroize();
    Ok(Some(
        serde_json::from_slice(&decoded).context("the HITSZ Connect Core session is invalid")?,
    ))
}

#[cfg(not(target_os = "macos"))]
pub fn load() -> Result<Option<AuthSessionSnapshot>> {
    Ok(None)
}
