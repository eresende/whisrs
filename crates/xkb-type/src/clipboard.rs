//! Clipboard implementations — Wayland (wl-copy/wl-paste), X11 (arboard), and noop.

use crate::ClipboardBackend;
use anyhow::Context;
use std::process::Command;

// ---------------------------------------------------------------------------
// Wayland: shell out to wl-paste / wl-copy
// ---------------------------------------------------------------------------

/// Clipboard backend that shells out to `wl-paste` (get) and `wl-copy` (set).
pub struct WaylandClipboard;

impl ClipboardBackend for WaylandClipboard {
    fn get_text(&self) -> anyhow::Result<String> {
        let output = Command::new("wl-paste")
            .arg("--no-newline")
            .output()
            .context("failed to run wl-paste — is wl-clipboard installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no suitable type") || stderr.contains("nothing is copied") {
                return Ok(String::new());
            }
            anyhow::bail!("wl-paste failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn set_text(&self, text: &str) -> anyhow::Result<()> {
        use std::io::Write;

        let mut child = Command::new("wl-copy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .context("failed to run wl-copy — is wl-clipboard installed?")?;

        if let Some(ref mut stdin) = child.stdin {
            stdin
                .write_all(text.as_bytes())
                .context("failed to write to wl-copy stdin")?;
        }

        let status = child.wait().context("failed to wait for wl-copy")?;
        if !status.success() {
            #[cfg(feature = "logging")]
            log::warn!("wl-copy exited with status {status}");
        }

        Ok(())
    }

    fn get_primary_selection(&self) -> anyhow::Result<String> {
        let output = Command::new("wl-paste")
            .args(["--no-newline", "--primary"])
            .output()
            .context("failed to run wl-paste --primary — is wl-clipboard installed?")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no suitable type") || stderr.contains("nothing is copied") {
                return Ok(String::new());
            }
            anyhow::bail!("wl-paste --primary failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}

// ---------------------------------------------------------------------------
// X11: arboard crate (behind "arboard" feature)
// ---------------------------------------------------------------------------

/// Clipboard backend that uses the `arboard` crate (X11).
#[cfg(feature = "arboard")]
pub struct X11Clipboard;

#[cfg(feature = "arboard")]
impl ClipboardBackend for X11Clipboard {
    fn get_text(&self) -> anyhow::Result<String> {
        let mut clipboard = arboard::Clipboard::new().context("failed to open X11 clipboard")?;
        clipboard
            .get_text()
            .context("failed to get text from X11 clipboard")
    }

    fn set_text(&self, text: &str) -> anyhow::Result<()> {
        let mut clipboard = arboard::Clipboard::new().context("failed to open X11 clipboard")?;
        clipboard
            .set_text(text)
            .context("failed to set text on X11 clipboard")
    }

    fn get_primary_selection(&self) -> anyhow::Result<String> {
        use arboard::GetExtLinux;
        let mut clipboard = arboard::Clipboard::new().context("failed to open X11 clipboard")?;
        clipboard
            .get()
            .clipboard(arboard::LinuxClipboardKind::Primary)
            .text()
            .context("failed to get text from X11 primary selection")
    }
}

// When arboard is not available, X11Clipboard is not available — callers on
// X11 without the feature will get NoopClipboard from default_clipboard().
#[cfg(not(feature = "arboard"))]
pub struct X11Clipboard;

#[cfg(not(feature = "arboard"))]
impl ClipboardBackend for X11Clipboard {
    fn get_text(&self) -> anyhow::Result<String> {
        anyhow::bail!("X11Clipboard requires the 'arboard' feature");
    }
    fn set_text(&self, _text: &str) -> anyhow::Result<()> {
        anyhow::bail!("X11Clipboard requires the 'arboard' feature");
    }
    fn get_primary_selection(&self) -> anyhow::Result<String> {
        anyhow::bail!("X11Clipboard requires the 'arboard' feature");
    }
}

// ---------------------------------------------------------------------------
// Noop clipboard
// ---------------------------------------------------------------------------

/// Clipboard backend that never succeeds or fails — all operations are no-ops.
pub struct NoopClipboard;

impl ClipboardBackend for NoopClipboard {
    fn get_text(&self) -> anyhow::Result<String> {
        Ok(String::new())
    }

    fn set_text(&self, _text: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn get_primary_selection(&self) -> anyhow::Result<String> {
        Ok(String::new())
    }
}

// ---------------------------------------------------------------------------
// Auto-detection
// ---------------------------------------------------------------------------

/// Return the appropriate clipboard backend for the current display server.
///
/// Checks `WAYLAND_DISPLAY` to decide: Wayland if set, X11 otherwise.
pub fn default_clipboard() -> Box<dyn ClipboardBackend> {
    if std::env::var("WAYLAND_DISPLAY").is_ok() {
        Box::new(WaylandClipboard)
    } else {
        Box::new(X11Clipboard)
    }
}
