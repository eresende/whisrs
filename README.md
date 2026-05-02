```
            _     _
 __      __| |__ (_)___  _ __ ___
 \ \ /\ / /| '_ \| / __|| '__/ __|
  \ V  V / | | | | \__ \| |  \__ \
   \_/\_/  |_| |_|_|___/|_|  |___/

  speak. type. done.
```

# whisrs

[![Crates.io](https://img.shields.io/crates/v/whisrs)](https://crates.io/crates/whisrs)
[![docs.rs](https://img.shields.io/docsrs/whisrs)](https://docs.rs/whisrs)

**whisrs is a Linux voice-to-text dictation tool written in Rust that transcribes speech via 6 backends — Groq, Deepgram REST, Deepgram Streaming, OpenAI REST, OpenAI Realtime, and local whisper.cpp — and types it into the focused window. It is the open-source Wispr Flow alternative for Linux.**

Press a hotkey, speak, and your words appear at the cursor in any focused app on Wayland, X11, Hyprland, Sway, GNOME, or KDE. Audio is captured via cpal across PipeWire, PulseAudio, and ALSA. Fully offline local transcription runs in under 500 MB of RAM with `base.en`. Fast, private, open source.

---

## How does whisrs differ from Wispr Flow and Superwhisper?

Wispr Flow and Superwhisper are closed-source dictation apps that don't run on Linux. whisrs is open source (MIT), Linux-native, and ships as a single async Rust process with native keyboard layout support (uinput + XKB), window tracking across Hyprland, Sway, X11, GNOME, and KDE, and 6 swappable transcription backends — both cloud (Groq, Deepgram, OpenAI) and fully offline (whisper.cpp). [xhisper](https://github.com/imaginalnika/xhisper) proved the concept on Linux; whisrs rebuilds it from scratch in Rust with broader compositor support and a daemon/CLI architecture you can bind to any hotkey.

---

## Installation

### Quick install (any distro)

```bash
curl -sSL https://y0sif.github.io/whisrs/install.sh | bash
```

Or clone and run locally:

```bash
git clone https://github.com/y0sif/whisrs && cd whisrs && ./install.sh
```

The install script handles everything: detects your distro, installs system dependencies, builds the project, and runs interactive setup.

After install, **press your hotkey** to start recording, **press again** to stop. Text appears at your cursor.

<details>
<summary><b>Other install methods (pre-built binary, AUR, Cargo, Nix, manual)</b></summary>

### Pre-built binary (Linux x86_64)

Each tagged release publishes a tarball on [GitHub Releases](https://github.com/y0sif/whisrs/releases/latest) with both `whisrs` and `whisrsd` plus the contrib files (udev rule, systemd unit, man pages).

```bash
# Full build (cloud + local whisper.cpp)
curl -sSL -o whisrs.tar.gz https://github.com/y0sif/whisrs/releases/latest/download/whisrs-linux-x86_64.tar.gz

# Or the minimal build (cloud backends only — smaller, no whisper.cpp)
curl -sSL -o whisrs.tar.gz https://github.com/y0sif/whisrs/releases/latest/download/whisrs-linux-x86_64-minimal.tar.gz

tar xzf whisrs.tar.gz
sudo install -m755 whisrs whisrsd /usr/local/bin/
sudo install -m644 contrib/99-whisrs.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
sudo usermod -aG input $USER   # log out / back in for the group change
whisrs setup
```

| Variant | Includes local whisper.cpp | Tarball |
|---|---|---|
| `whisrs-linux-x86_64.tar.gz` | yes | full build |
| `whisrs-linux-x86_64-minimal.tar.gz` | no (cloud backends only) | minimal build |

### Arch Linux (AUR)

```bash
yay -S whisrs-git
```

After install, run `whisrs setup` to configure your backend, API keys, permissions, and keybindings.

### Cargo

```bash
cargo install whisrs
```

Requires system dependencies: `alsa-lib`, `libxkbcommon`, `clang`, `cmake`.

After install, run `whisrs setup`.

### Nix

```bash
nix profile install github:y0sif/whisrs
```

Or add to your flake inputs:
```nix
inputs.whisrs.url = "github:y0sif/whisrs";
```

### Manual install

#### 1. Dependencies

```bash
# Arch Linux
sudo pacman -S base-devel alsa-lib libxkbcommon clang cmake

# Debian/Ubuntu
sudo apt install build-essential libasound2-dev libxkbcommon-dev libclang-dev cmake

# Fedora
sudo dnf install gcc-c++ alsa-lib-devel libxkbcommon-devel clang-devel cmake
```

#### 2. Build

```bash
git clone https://github.com/y0sif/whisrs
cd whisrs
cargo install --path .
```

#### 3. Setup

```bash
whisrs setup
```

The interactive setup will walk you through backend selection, API keys / model download, microphone test, uinput permissions, systemd service, and keybindings.

#### 4. Bind a hotkey

Example for Hyprland (`~/.config/hypr/hyprland.conf`):
```
bind = $mainMod, W, exec, whisrs toggle
```

Example for Sway (`~/.config/sway/config`):
```
bindsym $mod+w exec whisrs toggle
```

</details>

---

## What transcription backends does whisrs support?

| Backend | Type | Streaming | Cost | Best for |
|---|---|---|---|---|
| **Groq** | Cloud | Batch | Free tier available | Getting started, budget use |
| **Deepgram Streaming** | Cloud (WebSocket) | True streaming | $200 free credit | Streaming with free credits |
| **Deepgram REST** | Cloud | Batch | $200 free credit | Simple, 60+ languages |
| **OpenAI Realtime** | Cloud (WebSocket) | True streaming | Paid | Best UX, text as you speak |
| **OpenAI REST** | Cloud | Batch | Paid | Simple fallback |
| **Local whisper.cpp** | Local (CPU/GPU) | Sliding window | Free | Privacy, offline use |
| **ASR sidecar** | Local sidecar | Batch | Free | Custom local ASR models |

Groq is the default. Fast, free tier, good accuracy with `whisper-large-v3-turbo`.

Deepgram offers $200 in free credits on signup (no credit card required) and supports 60+ languages with the Nova-3 model. The streaming backend provides true real-time transcription over WebSocket.

OpenAI Realtime is the premium option: true streaming over WebSocket means text appears at your cursor while you're still speaking.

The ASR sidecar backend sends the recorded WAV to a local HTTP service and types the plain text it returns. This keeps Python/PyTorch/vLLM dependencies out of the Rust daemon and lets you use models such as VibeVoice-ASR, Moonshine, Distil-Whisper, or faster-whisper.

### Local whisper.cpp

Run transcription entirely on your machine. No API key, no internet, no data leaves your device. Included in every build.

```bash
whisrs setup   # select Local > whisper.cpp, pick a model, download automatically
```

| Model | Size | RAM | Speed (CPU) | Accuracy |
|---|---|---|---|---|
| tiny.en | 75 MB | ~273 MB | Real-time | Decent |
| base.en | 142 MB | ~388 MB | Real-time | Good (recommended) |
| small.en | 466 MB | ~852 MB | Borderline | Very good |

### ASR sidecar

The `asr-sidecar` backend expects a local HTTP endpoint that accepts multipart form data:

- `file`: WAV audio
- `model`: model id, default `microsoft/VibeVoice-ASR-HF`
- `language`: ISO 639-1 language code when not set to `auto`
- `hotwords`: optional vocabulary/context prompt

The sidecar should return JSON:

```json
{ "text": "transcribed text" }
```

Example sidecars are available under `contrib/asr-sidecars/`; each one has its
own setup and GPU notes.

| Sidecar | Default model | Best for |
|---|---|---|
| `moonshine` | `UsefulSensors/moonshine-base` | Fast lightweight English dictation |
| `vibevoice` | `microsoft/VibeVoice-ASR-HF` | Long-form local transcription experiments |

---

## Configuration

Config file: `~/.config/whisrs/config.toml` — `whisrs setup` writes a working file. A minimal example:

```toml
[general]
backend = "groq"            # groq | deepgram-streaming | deepgram | openai-realtime | openai | local-whisper | asr-sidecar
language = "en"             # ISO 639-1 or "auto"
silence_timeout_ms = 2000   # auto-stop after silence (streaming only)
notify = true               # desktop notifications
remove_filler_words = true  # strip "um", "uh", "you know", etc.
filler_words = []           # custom list (empty = use built-in defaults)
audio_feedback = true       # play tones on record start/stop/done
audio_feedback_volume = 0.5 # 0.0 to 1.0
vocabulary = ["whisrs", "Hyprland"]  # custom terms for better transcription accuracy
prompt = "Speech is in English or Spanish. Transcribe in the language spoken; never translate."  # optional sentence-style context, prepended to vocabulary
tray = true                 # system tray icon (requires SNI host like waybar)
overlay = false             # bottom-screen recording overlay (Hyprland/Sway, GNOME extension)

# Optional — controls overlay appearance when enabled.
# Defaults to a 100×40 pill with the "carbon" theme.
# When the overlay is on, recording/transcribing toast notifications are
# auto-suppressed (errors still pop) so the same event isn't double-signaled.
[overlay]
theme = "carbon"            # "carbon" (default) | "ember" | "cyan" | "custom"
width = 100                 # 90..=120 (clamped)
height = 40                 # 36..=48 (clamped)

# When theme = "custom", these override the named theme. Hex strings:
# #RGB, #RRGGBB, or #RRGGBBAA. Anything missing falls back to carbon.
# [overlay.colors]
# background   = "#0E0E10EB"
# ring         = "#3A3A4050"
# recording    = "#F0EDF5"
# transcribing = "#9CA3AF"
# glow         = "#F0EDF5"

[audio]
device = "default"

[input]
# Inter-key delay for the virtual keyboard (uinput). Raise this if a TUI
# drops characters while whisrs is typing — e.g. Node/Ink-based apps like
# Claude Code in raw mode. Default: 2.
key_delay_ms = 2

[groq]
api_key = "gsk_..."
model = "whisper-large-v3-turbo"

[deepgram]
api_key = "..."
model = "nova-3"

[openai]
api_key = "sk-..."
model = "gpt-4o-mini-transcribe"

[local-whisper]
model_path = "~/.local/share/whisrs/models/ggml-base.en.bin"

[asr-sidecar]
url = "http://127.0.0.1:8765/transcribe"
model = "microsoft/VibeVoice-ASR-HF"

# Command mode: LLM for voice-driven text rewriting
[llm]
api_key = "sk-..."
model = "gpt-4o-mini"
api_url = "https://api.openai.com/v1/chat/completions"

# Built-in global hotkeys (optional, works without WM keybinds)
[hotkeys]
toggle = "Super+Shift+W"
cancel = "Super+Shift+D"
command = "Super+Shift+G"
```

Env-var overrides: `WHISRS_GROQ_API_KEY`, `WHISRS_DEEPGRAM_API_KEY`, `WHISRS_OPENAI_API_KEY`.

For the full reference (overlay, `[input]`, `[llm]`, `[hotkeys]`, GNOME extension setup), see [docs/configuration.md](docs/configuration.md).

---

## CLI Commands

```
whisrs setup     # Interactive onboarding
whisrs toggle    # Start/stop recording
whisrs cancel    # Cancel recording, discard audio
whisrs status    # Query daemon state
whisrs command   # Command mode: select text + speak instruction → LLM rewrite
whisrs log       # Show recent transcription history
whisrs log -n 5  # Show last 5 entries
whisrs log --clear  # Clear all history
```

---

<a id="supported-environments"></a>

## Does whisrs work on Wayland, GNOME, KDE, Hyprland, and Sway?

Yes. whisrs runs natively on both Wayland and X11 across Hyprland, Sway, i3, GNOME Wayland, KDE Wayland, and any X11 window manager — with daily-driver coverage on Hyprland and community-confirmed reports on GNOME Wayland and Xorg. Audio capture works on PipeWire, PulseAudio, and ALSA via cpal.

| Component | Support |
|---|---|
| **Hyprland** | Tested by maintainer and community (Arch Linux) |
| **Sway / i3** | Implemented; additional reports welcome |
| **X11 (any WM)** | Tested by community on Ubuntu 24.04 (Xorg) |
| **GNOME Wayland** | Tested by community on Ubuntu 24.04 and Arch (mutter); overlay via the bundled [GNOME Shell extension](contrib/gnome-shell-extension/README.md) |
| **KDE Wayland** | Implemented via D-Bus; reports welcome |
| **Audio** | PipeWire, PulseAudio, ALSA (auto-detected via cpal) |
| **Distros** | Confirmed on Arch Linux and Ubuntu 24.04; any Linux with the system dependencies above |

> **Note:** whisrs is daily-driven on Hyprland (Arch Linux), with community confirmation on GNOME Wayland (Ubuntu 24.04 + Arch) and Xorg (Ubuntu 24.04). Sway, i3, and KDE reports are still wanted — if you use whisrs there, please open an issue with what works and what doesn't.

---

## Project Status

whisrs is functional and usable for daily dictation. The core features work:

- [x] Daemon + CLI architecture
- [x] Audio capture and WAV encoding
- [x] Groq, Deepgram (REST + streaming), OpenAI REST, OpenAI Realtime, and ASR sidecar backends
- [x] Local whisper.cpp backend (sliding window, prompt conditioning, model download)
- [x] Layout-aware keyboard injection (uinput + XKB)
- [x] Wayland/X11 clipboard with save/restore
- [x] Window tracking (Hyprland, Sway, X11, GNOME, KDE)
- [x] Desktop notifications and audio feedback
- [x] Interactive setup with LLM provider selection
- [x] Filler word removal
- [x] Transcription history (`whisrs log`)
- [x] Multi-language support (18 languages + auto-detect)
- [x] Custom vocabulary for improved transcription accuracy
- [x] LLM command mode (select text + voice instruction → rewrite)
- [x] System tray indicator (idle/recording/transcribing)
- [x] Configurable global hotkeys via evdev
- [x] Packaging ([AUR](https://aur.archlinux.org/packages/whisrs-git), Nix flake, crates.io)
- [ ] Local Vosk backend
- [ ] Local Parakeet backend (NVIDIA)

---

## Troubleshooting

See [docs/troubleshooting.md](docs/troubleshooting.md).

---

## Contributing

The biggest way to help right now:

1. **Test on your compositor** — Sway, i3, KDE, GNOME. Report what works and what doesn't.
2. **Test on your distro** — Ubuntu, Fedora, NixOS, etc. Build issues, missing deps, etc.
3. **Bug reports** — if text goes to the wrong window, characters get dropped, or audio doesn't capture, open an issue.

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and project structure.

---

## How whisrs Compares to Wispr Flow and Other Dictation Tools

whisrs is the open source alternative to closed-source dictation apps like **Wispr Flow** and **Superwhisper**, neither of which ships a Linux client. The closest open-source equivalents include [nerd-dictation](https://github.com/ideasman42/nerd-dictation), [Speech Note](https://github.com/mkiol/dsnote), and the cross-platform [Handy](https://github.com/cjpais/Handy). Head-to-head against the Linux-native options:

| Feature | whisrs | [nerd-dictation](https://github.com/ideasman42/nerd-dictation) | [Speech Note](https://github.com/mkiol/dsnote) | [Wispr Flow](https://wisprflow.ai/) |
|---|---|---|---|---|
| **Platform** | Linux | Linux | Linux | macOS, Windows (no Linux) |
| **Wayland support** | Yes (native) | Partial (xdotool) | Yes (GUI app) | N/A |
| **Offline transcription** | Yes (whisper.cpp) | Yes (Vosk) | Yes (multiple) | No |
| **Cloud transcription** | Groq, Deepgram (REST + streaming), OpenAI, OpenAI Realtime | No | No | Proprietary |
| **True streaming** | Yes (OpenAI Realtime) | No | No | Yes |
| **Keyboard injection** | uinput + XKB (layout-aware) | xdotool | Clipboard paste | Native |
| **Window tracking** | Hyprland, Sway, X11, GNOME, KDE | No | No | Native |
| **Architecture** | Daemon + CLI (bind to any hotkey) | Script | GUI app | GUI app |
| **Language** | Rust | Python | C++/Qt | Closed source |
| **Setup** | Interactive (`whisrs setup`) | Manual config | GUI | Installer |

For the full comparison, see [docs/comparison.md](docs/comparison.md).

## [FAQ](docs/faq.md)

---

## License

MIT
