# whisrs — Version Roadmap (0.1.2 → 0.1.5)

Incremental feature releases building toward v1.5.

---

## v0.1.2 — Multi-language & Transcription History ✓

- [x] **Multi-language support + auto-detection**: Language selection menu in setup with 18 common languages + auto-detect + custom ISO codes
- [x] **Transcription history** (`whisrs log [-n N] [--clear]`): JSONL storage at `~/.local/share/whisrs/history.jsonl`, newest-first, with timestamp/backend/language/duration
- [x] **whisper-rs update**: 0.15→0.16 (fixes bindgen compatibility)
- [x] **Feature gate**: `local-whisper` module properly cfg-gated for no-default-features builds

---

## v0.1.3 — Command Mode & Custom Vocabulary ✓

- [x] **Command mode** (`whisrs command`): Select text + hotkey → record voice instruction → LLM rewrites selected text in place. Toggleable (press again to stop early). Simulates Ctrl+C/Ctrl+V via uinput.
- [x] **Custom vocabulary**: `vocabulary = ["term1", "term2"]` in config — passed as prompt hint to Groq, OpenAI REST, and local whisper backends
- [x] **LLM integration**: `[llm]` config section with provider selection (OpenAI, Groq, OpenRouter, Google Gemini) and model menus with latest models. "Other" option for custom model names.

---

## v0.1.4 — System Tray & Configurable Hotkey ✓

- [x] **System tray indicator**: ksni StatusNotifierItem with colored circle icons — grey (idle), red (recording), yellow (transcribing). Works with waybar, KDE Plasma, GNOME (AppIndicator). Feature-gated behind `tray` feature (enabled by default).
- [x] **Configurable global hotkeys**: evdev-based passive keyboard listener. Config: `[hotkeys] toggle/cancel/command = "Super+Shift+D"`. Supports Super/Alt/Ctrl/Shift modifiers, left/right variants, letters, F-keys, named keys. No device grabbing — works alongside WM keybinds.
- [x] **State broadcasting**: Watch channel for real-time tray updates at all state transitions.

---

## v0.1.5 — Local Vosk Backend

- [ ] **Vosk backend**: CPU-only local speech recognition via `vosk` crate — true streaming, small models (~40 MB), works on Intel (no GPU required)
- [ ] Final polish pass for v1.5 readiness

---

## Deferred

- **Parakeet backend** — requires NVIDIA GPU
- **Static binary releases / install script** — post-feature distribution work
- **Cross-compositor testing** — community/contributor effort
- **Non-QWERTY layout testing** — later
- **Demo GIF** — later
- **Anthropic LLM support** — Anthropic uses a different API format (`/v1/messages` instead of `/v1/chat/completions`). Need to add an adapter in `llm.rs` to support the Messages API. Users can access Anthropic models via OpenRouter in the meantime.
