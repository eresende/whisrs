//! Detect prompt-echo hallucinations from cloud STT backends.
//!
//! Whisper-family models (OpenAI `whisper-1`, `gpt-4o-*-transcribe`, and most
//! Whisper-derived APIs) condition decoding on the optional `prompt` parameter.
//! When the audio carries no speech the model has nothing to anchor decoding
//! to and falls back to its strongest prior — the prompt itself — emitting it
//! verbatim (or in long contiguous chunks) as the "transcription".
//!
//! Without filtering, those echoes are typed at the cursor by whisrs, which
//! for a multi-hundred-character prompt can take tens of seconds at the
//! configured key delay. This module provides a conservative substring/word-run
//! heuristic that flags the obvious cases without false-positiving on real
//! speech that happens to use vocabulary present in the prompt.

/// Lowercase, drop non-alphanumeric characters (replaced with whitespace), and
/// collapse runs of whitespace. The output is suitable for substring/word-run
/// comparisons that ignore punctuation, casing, and incidental spacing.
fn normalize(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = true;
    for c in s.chars() {
        if c.is_alphanumeric() {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

/// Heuristically classify `response` as an echo of `prompt`.
///
/// Two checks, both intentionally conservative:
///
/// 1. After normalisation, the entire response is a substring of the prompt.
///    This covers the common case where the model regurgitates a contiguous
///    chunk of the prompt verbatim.
/// 2. The longest contiguous word-run that appears in both response and prompt
///    spans at least 6 words **and** covers at least 70% of the response. This
///    catches partial echoes where the model added a couple of stray words
///    around an otherwise verbatim regurgitation.
///
/// Short responses (fewer than 8 normalised characters or 6 words for the
/// run-based check) are never flagged: they could plausibly be a real one-word
/// utterance that overlaps the prompt's vocabulary, and the pain-from-typing
/// cost of letting such a short response through is negligible.
pub fn is_prompt_echo(response: &str, prompt: &str) -> bool {
    let resp = normalize(response);
    let prompt_n = normalize(prompt);

    if resp.chars().count() < 8 || prompt_n.is_empty() {
        return false;
    }

    if prompt_n.contains(&resp) {
        return true;
    }

    let resp_words: Vec<&str> = resp.split_whitespace().collect();
    let prompt_words: Vec<&str> = prompt_n.split_whitespace().collect();
    if resp_words.len() < 6 {
        return false;
    }
    let max_run = longest_common_word_run(&resp_words, &prompt_words);
    max_run >= 6 && max_run.saturating_mul(10) >= resp_words.len().saturating_mul(7)
}

/// Length of the longest contiguous run of equal words shared between `a` and
/// `b`, computed with the standard rolling longest-common-substring DP.
///
/// O(|a| * |b|) time, O(|b|) space. Prompts in practice are well under a
/// thousand words, so this is comfortably fast.
fn longest_common_word_run(a: &[&str], b: &[&str]) -> usize {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    let mut best = 0usize;
    let mut prev = vec![0usize; b.len()];
    let mut curr = vec![0usize; b.len()];
    for ai in a {
        for (j, bj) in b.iter().enumerate() {
            curr[j] = if ai == bj {
                if j == 0 {
                    1
                } else {
                    prev[j - 1] + 1
                }
            } else {
                0
            };
            if curr[j] > best {
                best = curr[j];
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.fill(0);
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_PROMPT: &str = "John Doe speaking. Professional, culinary register: \
        French pastry, sourdough baking, fermentation science, restaurant kitchen workflows. \
        Speech is in English or French; transcribe in the spoken language.";

    #[test]
    fn empty_prompt_never_echoes() {
        assert!(!is_prompt_echo("hello world this is a test", ""));
    }

    #[test]
    fn empty_response_not_echo() {
        assert!(!is_prompt_echo("", SAMPLE_PROMPT));
    }

    #[test]
    fn short_response_not_echo() {
        // Could be a legitimate single word; refuse to flag.
        assert!(!is_prompt_echo("John.", SAMPLE_PROMPT));
        assert!(!is_prompt_echo("pastry", SAMPLE_PROMPT));
    }

    #[test]
    fn full_prompt_echo_detected() {
        assert!(is_prompt_echo(SAMPLE_PROMPT, SAMPLE_PROMPT));
    }

    #[test]
    fn prefix_chunk_echo_detected() {
        let chunk = "John Doe speaking. Professional, culinary register: \
            French pastry, sourdough baking";
        assert!(is_prompt_echo(chunk, SAMPLE_PROMPT));
    }

    #[test]
    fn punctuation_and_case_insensitive() {
        let chunk = "JOHN DOE SPEAKING — professional / culinary register";
        assert!(is_prompt_echo(chunk, SAMPLE_PROMPT));
    }

    #[test]
    fn partial_echo_with_extra_words_detected() {
        // Model regurgitated a long prompt run with a couple of stray words.
        let resp = "okay um John Doe speaking professional culinary register French \
            pastry sourdough baking right";
        assert!(is_prompt_echo(resp, SAMPLE_PROMPT));
    }

    #[test]
    fn real_speech_not_flagged() {
        // Real utterance that happens to share vocabulary with the prompt but
        // doesn't echo a long contiguous run.
        let resp = "let's rebase this branch onto master and push it up to my fork";
        assert!(!is_prompt_echo(resp, SAMPLE_PROMPT));
    }

    #[test]
    fn real_speech_with_isolated_prompt_terms_not_flagged() {
        // Several prompt terms appear but scattered, no long contiguous run.
        let resp = "I am working on the sourdough recipe for a French pastry tonight";
        assert!(!is_prompt_echo(resp, SAMPLE_PROMPT));
    }

    #[test]
    fn longest_run_basic() {
        let a = ["the", "quick", "brown", "fox"];
        let b = ["jumps", "over", "the", "quick", "brown", "dog"];
        assert_eq!(longest_common_word_run(&a, &b), 3);
    }

    #[test]
    fn longest_run_no_overlap() {
        let a = ["alpha", "beta"];
        let b = ["gamma", "delta"];
        assert_eq!(longest_common_word_run(&a, &b), 0);
    }
}
