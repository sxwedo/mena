//! Locate the separate native app without coupling CLI builds to Swift or `UniFFI`.

use anyhow::Result;

#[cfg(target_os = "macos")]
pub fn open() -> Result<()> {
    use anyhow::{Context, bail};
    use std::path::PathBuf;
    use std::process::Command;

    let mut candidates = vec![PathBuf::from("/Applications/Mena.app")];
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join("Applications/Mena.app"));
    }
    if let Some(directory) = std::env::current_exe()?.parent() {
        candidates.push(directory.join("Mena.app"));
    }
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/macos/Mena.app"));
    let app = candidates.into_iter().find(|path| path.join("Contents/MacOS/Mena").is_file())
        .context("Mena.app is not installed. Build it with `mise run gui-build`, or install it in /Applications.")?;
    let status = Command::new("/usr/bin/open")
        .arg(&app)
        .status()
        .context("failed to open Mena.app")?;
    if !status.success() {
        bail!("macOS could not open {}", app.display());
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn open() -> Result<()> {
    anyhow::bail!(
        "the native desktop app requires macOS 14 or later; use `mena ss` on this platform"
    )
}
