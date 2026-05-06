# audio-silence-gate

Lightweight RMS-based silence detection and auto-stop for audio capture — zero dependencies, microsecond latency.

## Quick start

```rust
use audio_silence_gate::{is_silent, rms_energy};

let samples: Vec<i16> = /* from cpal or hound */;
let rms = rms_energy(&samples);
if is_silent(&samples, 0.005) {
    println!("silence (RMS = {rms:.4})");
}
```

## Real-time auto-stop

```rust
use audio_silence_gate::AutoStopDetector;

let mut detector = AutoStopDetector::new(
    0.005,   // RMS threshold
    2_000,   // 2 seconds of silence triggers stop
    16_000,  // sample rate (Hz)
);

loop {
    let chunk = capture.next_chunk();
    if detector.feed(&chunk) {
        println!("silence timeout — stopping recording");
        break;
    }
}
detector.reset(); // ready for next utterance
```

## Batch gate

Filter out obviously unusable recordings before they reach a transcription API:

```rust
use audio_silence_gate::{audio_gate_reason, GateReason, SILENCE_RMS_THRESHOLD};

match audio_gate_reason(&samples, 16_000, 300, SILENCE_RMS_THRESHOLD) {
    Some(GateReason::Empty) => eprintln!("empty buffer — discard"),
    Some(GateReason::TooShort) => eprintln!("recording too short — discard"),
    Some(GateReason::Silent) => eprintln!("no speech detected — discard"),
    Some(GateReason::Invalid) => eprintln!("invalid config (e.g. sample_rate == 0) — discard"),
    // `GateReason` is `#[non_exhaustive]`; future variants land here.
    Some(_) => eprintln!("unknown gate reason — discard"),
    None => send_to_transcription_api(&samples),
}
```

## When to use audio-silence-gate vs Silero VAD

|                   | audio-silence-gate  | Silero VAD             |
|-------------------|---------------------|------------------------|
| Dependencies      | **zero** (pure std) | ONNX runtime           |
| Latency           | **microseconds**    | milliseconds           |
| Accuracy          | RMS energy only     | neural, state-of-the-art |
| Model size        | none                | ~2 MB                  |
| Best for          | fast pre-filtering, auto-stop | precise speech segmentation |

Use `audio-silence-gate` when you need a simple, fast gate that catches the obvious cases
(accidental hotkey taps, background hum, long trailing silence). Reach for Silero VAD
when you need frame-level speech probability and word-boundary precision.

## Installation

```sh
cargo add audio-silence-gate
```

## License

MIT
