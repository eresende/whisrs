//! Deduplication for chunked streaming transcription.
//!
//! Two strategies:
//!
//! 1. **Timestamp-based** (`filter_words`): for APIs like Groq that return
//!    per-word timestamps. Tracks cumulative offset and discards words
//!    whose adjusted start time falls in the already-transcribed range.
//!
//! 2. **Text-based anchor search** (`filter_text`): for local whisper sliding
//!    window. Takes the last N words of previous output as an "anchor" and
//!    searches for it anywhere in the new text. Everything after the anchor
//!    is new text. Handles whisper's tendency to rephrase slightly between
//!    overlapping windows.

use tracing::debug;

use super::groq::GroqWord;

/// Tracks transcription progress across multiple chunks for deduplication.
pub struct DeduplicationTracker {
    /// The end time (in seconds) of the last word we accepted.
    transcribed_up_to: f64,
    /// Cumulative time offset added to each chunk's timestamps.
    cumulative_offset: f64,
    /// Previous window's full transcription (for anchor-based text dedup).
    recent_text: String,
    /// Maximum number of characters to keep in `recent_text` for matching.
    max_recent_chars: usize,
}

impl DeduplicationTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self {
            transcribed_up_to: 0.0,
            cumulative_offset: 0.0,
            recent_text: String::new(),
            max_recent_chars: 500,
        }
    }

    /// Add a time offset for the next chunk (the duration of audio already sent).
    pub fn advance_offset(&mut self, chunk_duration_secs: f64) {
        self.cumulative_offset += chunk_duration_secs;
        debug!(
            "dedup: advanced offset by {:.2}s, cumulative = {:.2}s",
            chunk_duration_secs, self.cumulative_offset
        );
    }

    /// Filter words from a new chunk, returning only the non-duplicate ones.
    ///
    /// Each word's `start` and `end` are adjusted by the cumulative offset.
    /// Words whose adjusted `start` time falls within the already-transcribed
    /// range are discarded.
    pub fn filter_words(&mut self, words: &[GroqWord]) -> Vec<GroqWord> {
        let mut accepted = Vec::new();

        for word in words {
            let adjusted_start = word.start + self.cumulative_offset;
            let adjusted_end = word.end + self.cumulative_offset;

            if adjusted_start >= self.transcribed_up_to - 0.05 {
                // Accept this word.
                accepted.push(GroqWord {
                    word: word.word.clone(),
                    start: adjusted_start,
                    end: adjusted_end,
                });
                self.transcribed_up_to = adjusted_end;
            }
        }

        debug!(
            "dedup: accepted {}/{} words, transcribed_up_to = {:.2}s",
            accepted.len(),
            words.len(),
            self.transcribed_up_to
        );

        accepted
    }

    /// Filter text from a sliding window transcription.
    ///
    /// Finds where the previous output ends within the new transcription
    /// (anchor search) and returns only the text after that point. Stores
    /// the full new transcription as the reference for the next window.
    pub fn filter_text(&mut self, new_text: &str) -> String {
        let result = if self.recent_text.is_empty() {
            new_text.to_string()
        } else {
            remove_overlap(&self.recent_text, new_text)
        };

        // Store the full new transcription as reference for the next window.
        // Each window's complete output is what the next window will overlap with.
        self.recent_text = new_text.to_string();
        if self.recent_text.len() > self.max_recent_chars {
            let trim_at = self.recent_text.len() - self.max_recent_chars;
            self.recent_text = self.recent_text[trim_at..].to_string();
        }

        result
    }
}

impl Default for DeduplicationTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Remove overlapping text between the end of `previous` and `new`.
///
/// Uses an anchor-search strategy: takes the last N words of `previous` and
/// searches for that sequence anywhere in the first 75% of `new`. This handles
/// whisper's tendency to slightly rephrase overlapping regions (word
/// insertions/deletions at boundaries) that break strict prefix alignment.
///
/// Falls back to prefix alignment for short texts (< 3 words in previous).
fn remove_overlap(previous: &str, new: &str) -> String {
    let prev_words: Vec<&str> = previous.split_whitespace().collect();
    let new_words: Vec<&str> = new.split_whitespace().collect();

    if prev_words.is_empty() || new_words.is_empty() {
        return new.to_string();
    }

    // --- Strategy 1: Anchor search ---
    // Take the last N words of previous and search for them in the new text.
    // This handles whisper inserting/removing words at window boundaries.
    let search_limit = (new_words.len() * 3 / 4).max(1);
    let max_anchor = prev_words.len().min(8);

    for anchor_len in (3..=max_anchor).rev() {
        let anchor = &prev_words[prev_words.len() - anchor_len..];

        for pos in 0..new_words.len() {
            if pos + anchor_len > new_words.len() || pos >= search_limit {
                break;
            }
            let candidate = &new_words[pos..pos + anchor_len];
            if ngram_match(anchor, candidate) {
                let new_start = pos + anchor_len;
                if new_start >= new_words.len() {
                    return String::new();
                }
                return new_words[new_start..].join(" ");
            }
        }
    }

    // --- Strategy 2: Prefix alignment (fallback for short texts) ---
    // Check if the end of previous matches the start of new exactly.
    let max_overlap = prev_words.len().min(new_words.len()).min(50);
    for overlap_len in (1..=max_overlap).rev() {
        let prev_suffix = &prev_words[prev_words.len() - overlap_len..];
        let new_prefix = &new_words[..overlap_len];

        if ngram_match(prev_suffix, new_prefix) {
            let remaining = &new_words[overlap_len..];
            if remaining.is_empty() {
                return String::new();
            }
            return remaining.join(" ");
        }
    }

    // No overlap found — return the full new text.
    new.to_string()
}

/// Check if two word slices match (allowing fuzzy matching per word).
fn ngram_match(a: &[&str], b: &[&str]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    a.iter().zip(b.iter()).all(|(wa, wb)| words_match(wa, wb))
}

/// Check if two words match, allowing for minor differences in punctuation
/// and small edit distances.
fn words_match(a: &str, b: &str) -> bool {
    // Normalize: lowercase and strip trailing punctuation.
    let na = normalize_word(a);
    let nb = normalize_word(b);

    if na == nb {
        return true;
    }

    // Use Jaro-Winkler similarity for fuzzy matching.
    let similarity = strsim::jaro_winkler(&na, &nb);
    similarity >= 0.85
}

/// Normalize a word for comparison: lowercase, strip trailing punctuation.
fn normalize_word(word: &str) -> String {
    word.to_lowercase()
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_dedup_no_overlap() {
        let mut tracker = DeduplicationTracker::new();

        let words = vec![
            GroqWord {
                word: "Hello".to_string(),
                start: 0.0,
                end: 0.5,
            },
            GroqWord {
                word: "world".to_string(),
                start: 0.6,
                end: 1.0,
            },
        ];

        let accepted = tracker.filter_words(&words);
        assert_eq!(accepted.len(), 2);
        assert_eq!(accepted[0].word, "Hello");
        assert_eq!(accepted[1].word, "world");
    }

    #[test]
    fn timestamp_dedup_skips_overlapping() {
        let mut tracker = DeduplicationTracker::new();

        // First chunk.
        let words1 = vec![
            GroqWord {
                word: "Hello".to_string(),
                start: 0.0,
                end: 0.5,
            },
            GroqWord {
                word: "world".to_string(),
                start: 0.6,
                end: 1.0,
            },
        ];
        let accepted1 = tracker.filter_words(&words1);
        assert_eq!(accepted1.len(), 2);

        // Second chunk with overlap — these words start before transcribed_up_to.
        let words2 = vec![
            GroqWord {
                word: "world".to_string(),
                start: 0.6,
                end: 1.0,
            },
            GroqWord {
                word: "how".to_string(),
                start: 1.1,
                end: 1.3,
            },
        ];
        // No offset advance — simulate overlap.
        let accepted2 = tracker.filter_words(&words2);
        assert_eq!(accepted2.len(), 1);
        assert_eq!(accepted2[0].word, "how");
    }

    #[test]
    fn timestamp_dedup_with_offset() {
        let mut tracker = DeduplicationTracker::new();

        let words1 = vec![GroqWord {
            word: "Hello".to_string(),
            start: 0.0,
            end: 0.5,
        }];
        tracker.filter_words(&words1);

        // Advance offset by 1 second (first chunk was 1s of audio).
        tracker.advance_offset(1.0);

        // Second chunk: timestamps restart from 0 but offset adjusts them.
        let words2 = vec![GroqWord {
            word: "world".to_string(),
            start: 0.1,
            end: 0.5,
        }];
        let accepted = tracker.filter_words(&words2);
        assert_eq!(accepted.len(), 1);
        assert_eq!(accepted[0].word, "world");
        // Adjusted start should be ~1.1.
        assert!((accepted[0].start - 1.1).abs() < 0.01);
    }

    #[test]
    fn text_dedup_no_previous() {
        let mut tracker = DeduplicationTracker::new();
        let result = tracker.filter_text("Hello world");
        assert_eq!(result, "Hello world");
    }

    #[test]
    fn text_dedup_removes_overlap_prefix() {
        // Simulates sliding window: window 2 re-transcribes window 1 + new text.
        let mut tracker = DeduplicationTracker::new();
        tracker.filter_text("the quick brown fox");
        let result = tracker.filter_text("the quick brown fox jumps over");
        assert_eq!(result, "jumps over");
    }

    #[test]
    fn text_dedup_handles_whisper_rephrase() {
        // Whisper changes "to see" → "and see" in the overlap region.
        // The anchor (last 3+ words of prev) should still find the match.
        let mut tracker = DeduplicationTracker::new();
        tracker.filter_text("trying to test it to see if it works");
        // Window 2 rephrased slightly but the end of prev ("if it works") is intact.
        let result =
            tracker.filter_text("trying to test it and see if it works right now I am speaking");
        assert_eq!(result, "right now I am speaking");
    }

    #[test]
    fn text_dedup_no_overlap_found() {
        let mut tracker = DeduplicationTracker::new();
        tracker.filter_text("Hello world");
        let result = tracker.filter_text("completely different text");
        assert_eq!(result, "completely different text");
    }

    #[test]
    fn text_dedup_full_overlap() {
        let mut tracker = DeduplicationTracker::new();
        tracker.filter_text("Hello world foo bar baz");
        let result = tracker.filter_text("Hello world foo bar baz");
        assert_eq!(result, "");
    }

    #[test]
    fn text_dedup_sliding_window_sequence() {
        // Simulate 3 overlapping windows.
        let mut tracker = DeduplicationTracker::new();

        let r1 = tracker.filter_text("A B C D E F");
        assert_eq!(r1, "A B C D E F");

        let r2 = tracker.filter_text("A B C D E F G H I");
        assert_eq!(r2, "G H I");

        let r3 = tracker.filter_text("D E F G H I J K L");
        assert_eq!(r3, "J K L");
    }

    #[test]
    fn normalize_word_strips_punctuation() {
        assert_eq!(normalize_word("Hello,"), "hello");
        assert_eq!(normalize_word("world."), "world");
        assert_eq!(normalize_word("test"), "test");
    }

    #[test]
    fn words_match_exact() {
        assert!(words_match("hello", "hello"));
        assert!(words_match("Hello", "hello"));
    }

    #[test]
    fn words_match_with_punctuation() {
        assert!(words_match("hello,", "hello"));
        assert!(words_match("world.", "world"));
    }

    #[test]
    fn words_match_fuzzy() {
        // Small edit distance should still match.
        assert!(words_match("hello", "helo"));
    }

    #[test]
    fn words_dont_match_very_different() {
        assert!(!words_match("hello", "world"));
    }

    #[test]
    fn remove_overlap_anchor_search() {
        // Anchor "three four" from end of prev, found in new text.
        let result = remove_overlap("one two three four", "two three four five six");
        assert_eq!(result, "five six");
    }

    #[test]
    fn remove_overlap_with_inserted_word() {
        // Whisper inserted "really" but the end anchor still matches.
        let result = remove_overlap(
            "I think it is going to work",
            "I really think it is going to work now",
        );
        assert_eq!(result, "now");
    }

    #[test]
    fn remove_overlap_none() {
        let result = remove_overlap("hello world", "completely different");
        assert_eq!(result, "completely different");
    }

    #[test]
    fn remove_overlap_prefix_fallback() {
        // Short prev — falls back to prefix alignment.
        let result = remove_overlap("brown fox", "brown fox jumps");
        assert_eq!(result, "jumps");
    }
}
