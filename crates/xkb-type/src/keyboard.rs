//! Virtual keyboard via evdev/uinput.
//!
//! Creates a virtual keyboard device via `/dev/uinput` and injects key events
//! using the reverse XKB keymap to produce the correct characters regardless
//! of the user's keyboard layout.

use std::thread;
use std::time::Duration;

use anyhow::Context;
use evdev::{uinput::VirtualDevice, AttributeSet, EventType, InputEvent, Key};
// Logging via the `log` crate (only active with `logging` feature).
#[cfg(feature = "logging")]
use log::{debug, warn};
#[cfg(not(feature = "logging"))]
macro_rules! debug {
    ($($arg:tt)*) => {};
}
#[cfg(not(feature = "logging"))]
macro_rules! warn {
    ($($arg:tt)*) => {};
}

use crate::keymap::XkbKeymap;
use crate::{ClipboardBackend, KeyTap};

/// Delay after creating the virtual device to let the kernel register it.
const DEVICE_SETTLE_DELAY: Duration = Duration::from_millis(200);

/// Virtual keyboard that injects keystrokes via uinput.
pub struct Keyboard {
    device: VirtualDevice,
    keymap: XkbKeymap,
    clipboard: Box<dyn ClipboardBackend>,
    key_delay: Duration,
}

impl Keyboard {
    /// Create a new virtual keyboard device.
    ///
    /// Requires write access to `/dev/uinput` (user must be in the `input`
    /// group or have the appropriate udev rule installed).
    ///
    /// `key_delay` is the inter-event delay. Raise it for TUIs that drop
    /// characters in raw mode (e.g. Node/Ink-based apps like Claude Code).
    pub fn new(
        keymap: XkbKeymap,
        clipboard: Box<dyn ClipboardBackend>,
        key_delay: Duration,
    ) -> anyhow::Result<Self> {
        // Register all key codes we might need (1..=247 covers standard
        // keys; KEY_MAX on Linux is ~767 but codes beyond 247 are rare).
        let mut keys = AttributeSet::<Key>::new();
        for code in 1..=247 {
            keys.insert(Key::new(code));
        }

        let device = evdev::uinput::VirtualDeviceBuilder::new()
            .context("failed to create VirtualDeviceBuilder")?
            .name("whisrs virtual keyboard")
            .with_keys(&keys)
            .context("failed to register key events")?
            .build()
            .context("failed to build uinput virtual device — check /dev/uinput permissions")?;

        // Give the kernel time to register the new device.
        thread::sleep(DEVICE_SETTLE_DELAY);

        debug!("uinput virtual keyboard created");

        Ok(Self {
            device,
            keymap,
            clipboard,
            key_delay,
        })
    }

    // -----------------------------------------------------------------------
    // Public API
    // -----------------------------------------------------------------------

    /// Type `text` by injecting keystrokes through the virtual device.
    ///
    /// Characters found in the keymap are typed via direct key events
    /// (with Shift/AltGr modifiers as needed, including dead-key sequences).
    /// Characters *not* found in the keymap are accumulated and pasted as
    /// a batch via the clipboard (Ctrl+V), with clipboard save/restore
    /// so the user's clipboard is not permanently overwritten.
    pub fn type_text(&mut self, text: &str) -> anyhow::Result<()> {
        self.release_all_modifiers()?;

        let mut paste_buf = String::new();

        for ch in text.chars() {
            // Copy the lookup result to release the immutable borrow on
            // self.keymap before calling mutable methods below.
            let mapping = self.keymap.lookup(ch).copied();
            if let Some(mapping) = mapping {
                // Flush any pending paste buffer first.
                if !paste_buf.is_empty() {
                    self.paste_text(&paste_buf)?;
                    paste_buf.clear();
                }
                self.tap_key(&mapping.main)?;
                if let Some(follow) = mapping.follow.as_ref() {
                    self.tap_key(follow)?;
                }
            } else {
                // Character not in keymap — accumulate for clipboard paste.
                paste_buf.push(ch);
            }
        }

        // Flush remaining paste buffer.
        if !paste_buf.is_empty() {
            self.paste_text(&paste_buf)?;
        }

        Ok(())
    }

    /// Emit Backspace `count` times.
    pub fn backspace(&mut self, count: usize) -> anyhow::Result<()> {
        self.release_all_modifiers()?;

        for _ in 0..count {
            self.tap_key(&KeyTap {
                keycode: Key::KEY_BACKSPACE.code(),
                shift: false,
                altgr: false,
            })?;
        }

        Ok(())
    }

    /// Press all keys in `keys`, then release them in reverse order.
    ///
    /// Each press and release is separated by `self.key_delay`.
    pub fn send_combo(&mut self, keys: &[Key]) -> anyhow::Result<()> {
        // Press all keys in order.
        for key in keys {
            self.device
                .emit(&[InputEvent::new(EventType::KEY, key.code(), 1)])?;
            thread::sleep(self.key_delay);
        }

        // Release in reverse order.
        for key in keys.iter().rev() {
            self.device
                .emit(&[InputEvent::new(EventType::KEY, key.code(), 0)])?;
            thread::sleep(self.key_delay);
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Press or release a modifier key.
    fn set_modifier(&mut self, modifier: Key, pressed: bool) -> anyhow::Result<()> {
        let value = if pressed { 1 } else { 0 };
        self.device
            .emit(&[InputEvent::new(EventType::KEY, modifier.code(), value)])?;
        thread::sleep(self.key_delay);
        Ok(())
    }

    /// Press and release a single key, with Shift and/or AltGr held as needed.
    ///
    /// AltGr is `KEY_RIGHTALT`. Both modifiers can be held together for
    /// level-3 chars on layouts like `us:intl` (e.g. Shift+AltGr+something
    /// for less common accented forms).
    fn tap_key(&mut self, tap: &KeyTap) -> anyhow::Result<()> {
        if tap.shift {
            self.set_modifier(Key::KEY_LEFTSHIFT, true)?;
        }
        if tap.altgr {
            self.set_modifier(Key::KEY_RIGHTALT, true)?;
        }

        self.device
            .emit(&[InputEvent::new(EventType::KEY, tap.keycode, 1)])?;
        thread::sleep(self.key_delay);
        self.device
            .emit(&[InputEvent::new(EventType::KEY, tap.keycode, 0)])?;
        thread::sleep(self.key_delay);

        if tap.altgr {
            self.set_modifier(Key::KEY_RIGHTALT, false)?;
        }
        if tap.shift {
            self.set_modifier(Key::KEY_LEFTSHIFT, false)?;
        }

        Ok(())
    }

    /// Release all modifier keys to prevent interference with injected text.
    fn release_all_modifiers(&mut self) -> anyhow::Result<()> {
        let modifiers = [
            Key::KEY_LEFTSHIFT,
            Key::KEY_RIGHTSHIFT,
            Key::KEY_LEFTCTRL,
            Key::KEY_RIGHTCTRL,
            Key::KEY_LEFTALT,
            Key::KEY_RIGHTALT,
            Key::KEY_LEFTMETA,
            Key::KEY_RIGHTMETA,
        ];

        for modifier in &modifiers {
            self.device
                .emit(&[InputEvent::new(EventType::KEY, modifier.code(), 0)])?;
        }
        thread::sleep(self.key_delay);

        Ok(())
    }

    /// Inject Ctrl+V to paste from clipboard.
    fn inject_ctrl_v(&mut self) -> anyhow::Result<()> {
        self.set_modifier(Key::KEY_LEFTCTRL, true)?;
        self.device
            .emit(&[InputEvent::new(EventType::KEY, Key::KEY_V.code(), 1)])?;
        thread::sleep(self.key_delay);
        self.device
            .emit(&[InputEvent::new(EventType::KEY, Key::KEY_V.code(), 0)])?;
        thread::sleep(self.key_delay);
        self.set_modifier(Key::KEY_LEFTCTRL, false)?;
        Ok(())
    }

    /// Set clipboard to `text`, inject Ctrl+V, then restore previous content.
    fn paste_text(&mut self, text: &str) -> anyhow::Result<()> {
        // Save current clipboard.
        let saved = self.clipboard.get_text().ok();

        // Set new clipboard content.
        self.clipboard
            .set_text(text)
            .context("failed to set clipboard for paste")?;

        // Small delay to ensure clipboard is ready.
        thread::sleep(Duration::from_millis(10));

        // Inject Ctrl+V.
        self.release_all_modifiers()?;
        self.inject_ctrl_v()?;

        // Small delay before restoring clipboard.
        thread::sleep(Duration::from_millis(50));

        // Restore previous clipboard content.
        if let Some(previous) = saved {
            if let Err(_e) = self.clipboard.set_text(&previous) {
                warn!("failed to restore clipboard: {_e}");
            }
        }

        Ok(())
    }
}
