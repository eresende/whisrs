//! Regex-based filler word and stutter removal from transcribed speech.
//!
//! This crate removes common English filler words (such as "um", "uh", "like",
//! "you know", etc.) and repeated-word stutters (e.g., "I I I went" → "I
//! went") from text — typically output from speech-to-text engines.
//!
//! # Built-in patterns are English-only
//!
//! The default filler patterns target English speech. For other languages,
//! supply your own custom filler words via [`FillerFilter::new`].
//!
//! # Example
//!
//! ```rust
//! use filler_remove::FillerFilter;
//!
//! // Built-in English patterns (no custom words).
//! let no_custom: [&str; 0] = [];
//! let filter = FillerFilter::new(&no_custom).unwrap();
//! assert_eq!(filter.apply("um, I uh went to the store"), "I went to the store");
//!
//! // Custom words for other languages.
//! let filter = FillerFilter::new(&["ну", "типа"]).unwrap();
//! assert_eq!(filter.apply("ну типа я пошёл"), "я пошёл");
//! ```
//!
//! For one-off / non-hot-path usage, [`remove_filler_words`] is also provided
//! as a convenience but recompiles regexes on every invocation.

use std::sync::LazyLock;

use regex::Regex;

/// Built-in filler patterns (case-insensitive, word-boundary-aware).
///
/// "like" requires a trailing comma to avoid removing the verb form.
const DEFAULT_FILLER_PATTERNS: &[&str] = &[
    r"\bum\b,?\s*",
    r"\buh\b,?\s*",
    r"\blike,\s*",
    r"\byou know,?\s*",
    r"\bbasically,?\s*",
    r"\bactually,?\s*",
    r"\bI mean,?\s*",
    r"\bsort of\b",
    r"\bkind of\b",
];

/// Pre-compiled built-in filler regexes.
static BUILTIN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    DEFAULT_FILLER_PATTERNS
        .iter()
        .map(|p| Regex::new(&format!("(?i){p}")).unwrap())
        .collect()
});

/// Pre-compiled regex for collapsing multiple spaces into one.
static SPACE_COLLAPSE_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r" {2,}").unwrap());

/// A reusable filler-word filter that pre-compiles its regexes once.
///
/// Construct with [`FillerFilter::new`] and call [`FillerFilter::apply`]
/// repeatedly — each call avoids recompiling regexes, which matters when the
/// filter is invoked per audio chunk or per recording.
///
/// # Behavior
///
/// - If `custom_words` is empty, the filter uses the built-in English patterns
///   (already pre-compiled via [`LazyLock`]).
/// - If `custom_words` is non-empty, those words are compiled into custom
///   regexes (each wrapped with `\b...\b,?\s*`) and the built-in defaults are
///   **not** used — same semantics as the legacy [`remove_filler_words`].
/// - Stutter removal and whitespace collapse are always applied.
///
/// # Errors
///
/// Returns the underlying [`regex::Error`] if any custom word produces an
/// invalid pattern. The built-in patterns are statically validated and never
/// fail.
pub struct FillerFilter {
    /// Compiled custom-word patterns. Empty when using built-ins.
    custom: Vec<Regex>,
}

impl FillerFilter {
    /// Build a new filter, pre-compiling all custom-word patterns.
    ///
    /// Pass an empty slice to use the built-in English filler list.
    ///
    /// # Errors
    ///
    /// Surfaces a [`regex::Error`] if any custom word produces an invalid
    /// regex. Note that custom words are escaped via [`regex::escape`] before
    /// insertion, so failures here are rare in practice — but a malformed
    /// (non-UTF-8 or zero-length) input could still trip the compiler.
    pub fn new<S: AsRef<str>>(custom_words: &[S]) -> Result<Self, regex::Error> {
        let mut custom = Vec::with_capacity(custom_words.len());
        for word in custom_words {
            let pattern_str = format!(r"(?i)\b{},?\s*", regex::escape(word.as_ref()));
            custom.push(Regex::new(&pattern_str)?);
        }
        Ok(Self { custom })
    }

    /// Apply the filter to `text`, returning a cleaned string.
    ///
    /// This is cheap to call repeatedly — no regex compilation happens here.
    pub fn apply(&self, text: &str) -> String {
        let mut result = text.to_string();

        if self.custom.is_empty() {
            // Use pre-compiled built-in patterns.
            for re in BUILTIN_PATTERNS.iter() {
                result = re.replace_all(&result, "").to_string();
            }
        } else {
            // Use pre-compiled custom-word patterns.
            for re in &self.custom {
                result = re.replace_all(&result, "").to_string();
            }
        }

        // Remove stutters (repeated consecutive words like "I I I went" -> "I went").
        // The regex crate doesn't support backreferences, so we do this manually.
        result = remove_stutters(&result);

        // Collapse multiple spaces and trim.
        result = SPACE_COLLAPSE_RE.replace_all(&result, " ").to_string();

        result.trim().to_string()
    }
}

/// Remove filler words and stutters from the given text.
///
/// When `custom_words` is non-empty, those patterns are used instead of the
/// built-in defaults. Each custom word is wrapped with `\b...\b,?\s*` to
/// create a word-boundary-aware pattern.
///
/// Always removes stutters regardless of the filler list.
///
/// # Built-in patterns are English-only
///
/// The default filler list targets English speech. For other languages,
/// supply custom words.
///
/// # Performance
///
/// This convenience function compiles a fresh [`FillerFilter`] on every call.
/// For hot paths (per-chunk / per-recording filtering), construct a
/// [`FillerFilter`] once via [`FillerFilter::new`] and reuse it.
///
/// # Panics
///
/// Panics if any `custom_words` entry produces an invalid regex. Use
/// [`FillerFilter::new`] for fallible construction. (In practice, every
/// custom word is escaped via [`regex::escape`], so failures here are rare.)
pub fn remove_filler_words(text: &str, custom_words: &[String]) -> String {
    let filter = FillerFilter::new(custom_words)
        .expect("custom filler word produced an invalid regex; use FillerFilter::new for fallible construction");
    filter.apply(text)
}

/// Remove consecutive repeated words (case-insensitive).
/// "I I I went" -> "I went", "the the cat" -> "the cat".
fn remove_stutters(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }

    let mut result = Vec::with_capacity(words.len());
    result.push(words[0]);

    for word in &words[1..] {
        if let Some(prev) = result.last() {
            if !prev.eq_ignore_ascii_case(word) {
                result.push(word);
            }
        }
    }

    result.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removes_um() {
        assert_eq!(
            remove_filler_words("um I went to the store", &[]),
            "I went to the store"
        );
    }

    #[test]
    fn removes_uh() {
        assert_eq!(remove_filler_words("I uh went home", &[]), "I went home");
    }

    #[test]
    fn removes_like_filler() {
        // "like," with comma is treated as filler.
        assert_eq!(
            remove_filler_words("it was like, really cool", &[]),
            "it was really cool"
        );
    }

    #[test]
    fn preserves_like_as_verb() {
        // "like" followed by end of string (no trailing space/comma) is preserved
        assert_eq!(remove_filler_words("I like cats", &[]), "I like cats");
    }

    #[test]
    fn removes_you_know() {
        assert_eq!(
            remove_filler_words("it was, you know, pretty good", &[]),
            "it was, pretty good"
        );
    }

    #[test]
    fn removes_basically() {
        assert_eq!(remove_filler_words("basically it works", &[]), "it works");
    }

    #[test]
    fn removes_actually() {
        assert_eq!(
            remove_filler_words("actually I think so", &[]),
            "I think so"
        );
    }

    #[test]
    fn removes_i_mean() {
        assert_eq!(
            remove_filler_words("I mean it was fine", &[]),
            "it was fine"
        );
    }

    #[test]
    fn removes_sort_of() {
        assert_eq!(
            remove_filler_words("it was sort of okay", &[]),
            "it was okay"
        );
    }

    #[test]
    fn removes_kind_of() {
        assert_eq!(
            remove_filler_words("it was kind of nice", &[]),
            "it was nice"
        );
    }

    #[test]
    fn removes_stutters() {
        assert_eq!(
            remove_filler_words("I I I went to the store", &[]),
            "I went to the store"
        );
    }

    #[test]
    fn removes_double_stutter() {
        assert_eq!(remove_filler_words("the the cat sat", &[]), "the cat sat");
    }

    #[test]
    fn removes_multiple_fillers() {
        assert_eq!(
            remove_filler_words("um uh like, you know basically it works", &[]),
            "it works"
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(remove_filler_words("Um I went", &[]), "I went");
        assert_eq!(remove_filler_words("UH okay", &[]), "okay");
    }

    #[test]
    fn collapses_spaces() {
        assert_eq!(remove_filler_words("I  um  went  home", &[]), "I went home");
    }

    #[test]
    fn empty_input() {
        assert_eq!(remove_filler_words("", &[]), "");
    }

    #[test]
    fn no_fillers() {
        assert_eq!(
            remove_filler_words("the cat sat on the mat", &[]),
            "the cat sat on the mat"
        );
    }

    #[test]
    fn custom_words() {
        let custom = vec!["well".to_string(), "so".to_string()];
        assert_eq!(
            remove_filler_words("well so I went home", &custom),
            "I went home"
        );
    }

    #[test]
    fn custom_words_ignores_defaults() {
        // With custom words, default fillers like "um" should NOT be removed.
        let custom = vec!["well".to_string()];
        assert_eq!(remove_filler_words("well um I went", &custom), "um I went");
    }

    #[test]
    fn trims_result() {
        assert_eq!(remove_filler_words("  um hello  ", &[]), "hello");
    }

    #[test]
    fn filler_with_comma() {
        assert_eq!(remove_filler_words("like, it was good", &[]), "it was good");
    }

    // --- Non-ASCII language tests ---

    #[test]
    fn cyrillic_no_fillers() {
        let text = "Привет мир, как дела?";
        assert_eq!(remove_filler_words(text, &[]), text);
    }

    #[test]
    fn cyrillic_with_english_filler() {
        // Transcription APIs sometimes mix English fillers into non-English text.
        assert_eq!(remove_filler_words("um Привет мир", &[]), "Привет мир");
    }

    #[test]
    fn cyrillic_stutter_removal() {
        assert_eq!(remove_filler_words("я я пошёл домой", &[]), "я пошёл домой");
    }

    #[test]
    fn arabic_no_fillers() {
        let text = "مرحبا بالعالم";
        assert_eq!(remove_filler_words(text, &[]), text);
    }

    #[test]
    fn arabic_stutter_removal() {
        assert_eq!(remove_filler_words("هذا هذا اختبار", &[]), "هذا اختبار");
    }

    #[test]
    fn cjk_no_fillers() {
        let text = "你好世界";
        assert_eq!(remove_filler_words(text, &[]), text);
    }

    #[test]
    fn japanese_no_fillers() {
        let text = "こんにちは世界";
        assert_eq!(remove_filler_words(text, &[]), text);
    }

    #[test]
    fn korean_no_fillers() {
        let text = "안녕하세요 세계";
        assert_eq!(remove_filler_words(text, &[]), text);
    }

    #[test]
    fn mixed_script_with_filler() {
        assert_eq!(
            remove_filler_words("basically Привет, 世界 uh okay", &[]),
            "Привет, 世界 okay"
        );
    }

    #[test]
    fn custom_cyrillic_filler() {
        let custom = vec!["ну".to_string(), "типа".to_string()];
        assert_eq!(remove_filler_words("ну типа я пошёл", &custom), "я пошёл");
    }

    #[test]
    fn emoji_in_text() {
        let text = "um 😀 that was basically 🎉 great";
        assert_eq!(remove_filler_words(text, &[]), "😀 that was 🎉 great");
    }

    // --- FillerFilter tests ---

    #[test]
    fn filter_caches_builtin_patterns() {
        // Reusing a single FillerFilter across many calls must produce the
        // same output as recompiling on every call — and not panic.
        let filter = FillerFilter::new::<&str>(&[]).unwrap();
        for _ in 0..100 {
            assert_eq!(filter.apply("um hello uh world"), "hello world");
        }
    }

    #[test]
    fn filter_caches_custom_patterns() {
        // The custom-word path must also be reusable without recompiling.
        let filter = FillerFilter::new(&["well", "so"]).unwrap();
        for _ in 0..100 {
            assert_eq!(filter.apply("well so I went home"), "I went home");
            // Custom words override defaults — "um" should remain.
            assert_eq!(filter.apply("well um okay"), "um okay");
        }
    }

    #[test]
    fn filter_matches_free_function_builtin() {
        // FillerFilter::apply must produce the same output as the legacy
        // free function for the built-in path.
        let filter = FillerFilter::new::<&str>(&[]).unwrap();
        let cases = [
            "um I went to the store",
            "it was like, really cool",
            "basically it works",
        ];
        for case in cases {
            assert_eq!(filter.apply(case), remove_filler_words(case, &[]));
        }
    }

    #[test]
    fn filter_new_returns_result_type() {
        // The constructor signature is `Result<Self, regex::Error>` so any
        // failure to compile a custom word now surfaces as a real error
        // instead of being silently dropped (the previous behavior used
        // `if let Ok(re) = ...` and discarded compilation errors).
        let res: Result<FillerFilter, regex::Error> = FillerFilter::new(&["um", "uh"]);
        assert!(res.is_ok());
    }

    #[test]
    fn filter_surfaces_invalid_pattern_error() {
        // Confirm the error propagation path actually fires when the
        // underlying regex compilation fails.
        //
        // Custom-word inputs are escaped via `regex::escape` before
        // compilation, which sanitizes any malformed regex syntax. The only
        // realistic remaining failure mode is a custom word large enough to
        // overrun the regex compiler's `size_limit`. We use a very large
        // literal here to trigger that error; the test is slow but it's the
        // only way to genuinely exercise the `Err` arm of `FillerFilter::new`
        // through public API. This guards against a future refactor
        // accidentally restoring the silent-drop behavior.
        // 128 KiB is the smallest size we measured that reliably triggers
        // `CompiledTooBig` against the regex crate's default 10 MiB limit,
        // and keeps the test under a couple seconds in debug builds.
        let huge = "a".repeat(128 * 1024);
        let result = FillerFilter::new(&[huge.as_str()]);
        assert!(
            result.is_err(),
            "FillerFilter::new must surface oversized-pattern errors as Err, \
             not silently drop them",
        );

        // And the legacy free function must panic on the same input,
        // proving the Result is wired through end-to-end rather than
        // swallowed at a lower layer.
        let panicked = std::panic::catch_unwind(|| remove_filler_words("hello", &[huge]));
        assert!(
            panicked.is_err(),
            "remove_filler_words must panic on an invalid custom pattern",
        );
    }
}
