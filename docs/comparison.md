# How whisrs Compares

| Feature | whisrs | [nerd-dictation](https://github.com/ideasman42/nerd-dictation) | [Speech Note](https://github.com/mkiol/dsnote) | [Wispr Flow](https://wisprflow.com/) |
|---|---|---|---|---|
| **Platform** | Linux | Linux | Linux | macOS only |
| **Wayland support** | Yes (native) | Partial (xdotool) | Yes (GUI app) | N/A |
| **Offline transcription** | Yes (whisper.cpp) | Yes (VOSK) | Yes (multiple) | No |
| **Cloud transcription** | Groq, OpenAI, OpenAI Realtime | No | No | Proprietary |
| **True streaming** | Yes (OpenAI Realtime) | No | No | Yes |
| **Keyboard injection** | uinput + XKB (layout-aware) | xdotool | Clipboard paste | Native |
| **Window tracking** | Hyprland, Sway, X11, GNOME, KDE | No | No | Native |
| **Architecture** | Daemon + CLI (bind to any hotkey) | Script | GUI app | GUI app |
| **Language** | Rust | Python | C++/Qt | Closed source |
| **Setup** | Interactive (`whisrs setup`) | Manual config | GUI | Installer |
