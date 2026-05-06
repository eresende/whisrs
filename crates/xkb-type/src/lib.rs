//! Virtual keyboard typing via evdev/uinput with XKB layout-aware key mapping.
//!
//! This crate provides:
//! - [`Keyboard`] — a virtual uinput device that injects keystrokes.
//! - [`XkbKeymap`] — reverse char→keycode lookup from the active XKB layout.
//! - [`ClipboardBackend`] — trait + auto-detected clipboard implementations.
//!
//! # Example
//!
//! ```no_run
//! use xkb_type::{Keyboard, XkbKeymap, KeyboardLayout, default_clipboard};
//! use std::time::Duration;
//!
//! let layout = KeyboardLayout::detect();
//! let keymap = XkbKeymap::from_layout(&layout).unwrap();
//! let keyboard = Keyboard::new(keymap, default_clipboard(), Duration::from_millis(5)).unwrap();
//! ```

pub mod clipboard;
pub mod keyboard;
pub mod keymap;

use std::time::Duration;

// ---------------------------------------------------------------------------
// Core types
// ---------------------------------------------------------------------------

/// A single keypress with optional Shift and/or AltGr modifiers.
#[derive(Debug, Clone, Copy)]
pub struct KeyTap {
    pub keycode: u16,
    pub shift: bool,
    pub altgr: bool,
}

/// Information needed to produce a character at the cursor.
///
/// For most characters this is a single [`KeyTap`]. For characters that
/// XKB only exposes as a dead-key combination (e.g. `ã` = `dead_tilde + a`
/// on `us:intl`, or `'` = `dead_acute + space`), a `follow` tap is
/// recorded so the typer emits the dead-key keypress followed by the
/// base-letter (or space) keypress in sequence.
#[derive(Debug, Clone, Copy)]
pub struct KeyMapping {
    pub main: KeyTap,
    pub follow: Option<KeyTap>,
}

// ---------------------------------------------------------------------------
// ClipboardBackend trait
// ---------------------------------------------------------------------------

/// Trait for clipboard get/set operations.
pub trait ClipboardBackend: Send + Sync {
    /// Read the current clipboard text content.
    fn get_text(&self) -> anyhow::Result<String>;

    /// Set the clipboard to the given text.
    fn set_text(&self, text: &str) -> anyhow::Result<()>;

    /// Read the primary selection (highlighted text, no Ctrl+C needed).
    fn get_primary_selection(&self) -> anyhow::Result<String>;
}

// ---------------------------------------------------------------------------
// Re-exports
// ---------------------------------------------------------------------------

pub use clipboard::{default_clipboard, NoopClipboard, WaylandClipboard, X11Clipboard};
pub use keyboard::Keyboard;
pub use keymap::{KeyboardLayout, XkbKeymap};

/// Convenience: build a [`Keyboard`] from the detected layout with a
/// sensible default key delay (5 ms).
///
/// Returns an error if the XKB keymap cannot be built (missing locale data)
/// or if `/dev/uinput` is not writable.
pub fn keyboard_from_detected_layout() -> anyhow::Result<Keyboard> {
    let layout = KeyboardLayout::detect();
    let keymap = XkbKeymap::from_layout(&layout)?;
    Keyboard::new(keymap, default_clipboard(), Duration::from_millis(5))
}
