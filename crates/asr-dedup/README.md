# asr-dedup

Deduplication strategies for chunked/sliding-window ASR transcription — eliminates
duplicate words caused by overlapping audio windows in streaming speech-to-text.

## Installation

```sh
cargo add asr-dedup
```

## Usage

### Timestamp-based dedup (cloud APIs like Groq, Deepgram)

Use when your ASR provider returns per-word start/end timestamps.

```rust
use asr_dedup::{DedupTracker, Word};

let mut tracker = DedupTracker::new();

// Chunk 1: 1.0 seconds of audio, words at t=0.0..0.5
let words1 = vec![
    Word { text: "Hello".into(), start_secs: 0.0, end_secs: 0.5 },
    Word { text: "world".into(), start_secs: 0.6, end_secs: 1.0 },
];
let novel1 = tracker.filter_words(&words1);
// → "Hello", "world"

// Advance offset for the next chunk
tracker.advance_offset(1.0);

// Chunk 2: next 1.0s, timestamps restart from 0 internally
let words2 = vec![
    Word { text: "world".into(), start_secs: 0.0, end_secs: 0.4 },
    Word { text: "how".into(),   start_secs: 0.5, end_secs: 0.8 },
];
let novel2 = tracker.filter_words(&words2);
// → "how" (adjusted start_secs = 1.5, beyond transcribed_up_to)
```

### Text-based anchor dedup (local whisper.cpp sliding window)

Use when you have no timestamps — just the full transcription string per window.

```rust
use asr_dedup::DedupTracker;

let mut tracker = DedupTracker::new();

// Window 1
let r1 = tracker.filter_text("the quick brown fox");
assert_eq!(r1, "the quick brown fox");

// Window 2 overlaps with window 1
let r2 = tracker.filter_text("the quick brown fox jumps over");
assert_eq!(r2, "jumps over");

// Window 3 also overlaps
let r3 = tracker.filter_text("brown fox jumps over the lazy dog");
assert_eq!(r3, "the lazy dog");
```

The text strategy handles whisper's tendency to slightly rephrase between
windows (e.g. inserting or dropping a word at the boundary). It uses an
anchor-based search with fuzzy per-word matching (Jaro-Winkler ≥ 0.85).

## When to use each strategy

|                  | Timestamp strategy        | Text anchor strategy    |
|------------------|---------------------------|-------------------------|
| **Input**        | Word list + start/end secs| Full-text string        |
| **Requires offset** | Yes (`advance_offset`) | No                      |
| **Fuzzy matching**  | No (exact time cut)    | Yes (Jaro-Winkler)      |
| **Best for**     | Cloud APIs (Groq, Deepgram) | Local whisper.cpp     |

## Optional logging

Enable the `logging` feature to emit `log::debug!` messages:

```toml
[dependencies]
asr-dedup = { version = "0.1", features = ["logging"] }
```

## License

MIT
