# filler-remove

Regex-based filler word and stutter removal from transcribed speech.

## Usage

### Built-in English patterns

```rust
use filler_remove::remove_filler_words;

let cleaned = remove_filler_words("um, I uh went to the store", &[]);
assert_eq!(cleaned, "I went to the store");
```

### Custom filler words for other languages

Built-in patterns are English-only. For other languages, supply your own
custom filler words:

```rust
use filler_remove::remove_filler_words;

let custom = vec!["ну".to_string(), "типа".to_string()];
let cleaned = remove_filler_words("ну типа я пошёл", &custom);
assert_eq!(cleaned, "я пошёл");
```

### Stutter removal

Stutters (consecutive repeated words) are always removed regardless of the
filler list:

```rust
use filler_remove::remove_filler_words;

let cleaned = remove_filler_words("I I I went to the store", &[]);
assert_eq!(cleaned, "I went to the store");
```

## Installation

```sh
cargo add filler-remove
```

## License

MIT
