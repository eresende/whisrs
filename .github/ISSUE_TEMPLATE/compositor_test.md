---
name: Compositor / Distro Test Report
about: Report your testing results on a specific compositor or distro
title: 'Tested on: [compositor] / [distro]'
labels: testing
assignees: ''
---

## Environment

- **Compositor:** (e.g., Sway 1.9)
- **Distro:** (e.g., Fedora 40)
- **Audio backend:** (PipeWire / PulseAudio / ALSA)

## What Works

- [ ] Daemon starts without errors
- [ ] Audio capture works
- [ ] Transcription returns text
- [ ] Text is typed at cursor correctly
- [ ] Window tracking captures correct window
- [ ] Window focus is restored before typing
- [ ] Clipboard save/restore works
- [ ] Desktop notifications appear
- [ ] Non-QWERTY keyboard layout works (if applicable)

## What Doesn't Work

Describe any issues encountered.

## Daemon Logs

```
(paste relevant logs with RUST_LOG=debug)
```

## Notes

Any other observations (latency, quirks, workarounds, etc.)
