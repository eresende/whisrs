# Contributing to whisrs

Thanks for considering a contribution to whisrs. This document covers the basics.

## Ways to Contribute

### Testing on your setup

The most impactful thing you can do right now is test whisrs on your compositor and distro, then report what works and what doesn't.

**Compositors that need testing:**
- Sway / i3
- KDE Plasma (Wayland)
- GNOME (Wayland and X11)
- Any X11 window manager

**Distros that need testing:**
- Fedora, Ubuntu, Debian, NixOS, Void, Gentoo, etc.

When reporting, include:
- Compositor + version
- Distro + version
- Audio backend (PipeWire / PulseAudio / ALSA)
- What worked, what didn't
- Daemon logs (`RUST_LOG=debug whisrsd`)

### Bug reports

Open an issue using the bug report template. Include:
- Steps to reproduce
- Expected vs actual behavior
- Daemon logs
- Your environment (compositor, distro, audio backend)

### Code contributions

1. Fork the repo
2. Create a feature branch (`git checkout -b feature/my-thing`)
3. Make your changes
4. Run the checks: `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`
5. Commit with a clear message
6. Open a PR using the template

## Development Setup

### Prerequisites

```bash
# Arch Linux
sudo pacman -S base-devel alsa-lib libxkbcommon pkg-config

# Ubuntu/Debian
sudo apt install build-essential libasound2-dev libxkbcommon-dev pkg-config

# Fedora
sudo dnf install alsa-lib-devel libxkbcommon-devel pkg-config
```

### Build and test

```bash
cargo build
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

### Running locally

```bash
# Terminal 1: start the daemon with debug logging
RUST_LOG=debug cargo run --bin whisrsd

# Terminal 2: send commands
cargo run --bin whisrs -- toggle
cargo run --bin whisrs -- status
cargo run --bin whisrs -- cancel
```

### Project structure

```
src/
  cli/main.rs          — thin CLI client
  daemon/main.rs       — daemon (the real application)
  lib.rs               — shared types, config, IPC protocol
  state.rs             — state machine
  audio/               — cpal capture, WAV encoding, silence detection
  transcription/       — backend trait + Groq, OpenAI, local implementations
  input/               — uinput keyboard, XKB keymap, clipboard
  window/              — compositor-specific window tracking
  config/              — interactive setup
```

## Code Style

- Use `thiserror` for library error types, `anyhow` for binary error handling
- Use `tracing` for all logging (not `println!` or `log`)
- Run `cargo fmt` before committing
- No warnings from `cargo clippy --all-targets -- -D warnings`
- Keep dependencies minimal — don't add a crate for something you can write in 20 lines

## Commit Messages

Keep them concise. First line is a summary (imperative mood), then a blank line, then details if needed.

```
Add Sway window tracking via IPC

Use swayipc crate to query focused window and restore focus.
Tested on Sway 1.9 with Wayland.
```
