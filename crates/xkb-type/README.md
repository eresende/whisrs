# xkb-type

Layout-aware text injection for Linux — types correct characters regardless of the user's active keyboard layout.

## Problem

Linux has no "type this text at the cursor" API. Every tool that injects text (wtype, ydotool, xdotool) sends fixed keycodes, which produce wrong characters when the user has a non-US keyboard layout. German users get "z" and "y" swapped; French users get "a" and "q" swapped; Russian users get complete gibberish.

## Solution

`xkb-type` uses `xkbcommon` to read the active XKB keyboard layout, builds a reverse mapping from every Unicode character to the physical key + modifiers needed to produce it, then injects the correct keystrokes via a Linux uinput virtual keyboard.

Characters not found in the keymap (emoji, rare symbols) are pasted via clipboard (Ctrl+V).

## Usage

```rust
use xkb_type::Keyboard;
use std::time::Duration;

// Auto-detect layout and clipboard backend.
let mut kb = Keyboard::new(Duration::from_millis(2))?;
kb.type_text("Hello — こんにちは — €100 — 😀")?;
kb.backspace(5)?;
kb.send_combo(&[Key::KEY_LEFTCTRL, Key::KEY_C])?;
```

## Requirements

- Linux
- Write access to `/dev/uinput` (user in `input` group or udev rule)
- `libxkbcommon` (installed by default on most Linux desktops)
- `wl-clipboard` for Wayland clipboard operations

## Installation

```fish
cargo add xkb-type
```

## License

MIT
