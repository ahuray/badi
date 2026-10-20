//! Finishes the word being typed with a word already in front of the writer:
//! in the same window, in their earlier writing in this app, or earlier in the
//! draft. Exact reuse only, with no model and no dictionary. A match is
//! evidence that a word exists, never proof that the writer means it, so any
//! second candidate makes it abstain and the model continues as before.

use std::collections::BTreeMap;

use unicode_segmentation::UnicodeSegmentation;

pub mod learned;

#[cfg(feature = "local-model")]
use crate::writing::WritingLanguage;

/// Fewer typed letters too often start an unrelated word: on the 2026-10-20
/// independent set, three or four letters finished 13 of 25 words wrongly,
/// five or more none of 25.
pub const MIN_STEM_SCALARS: usize = 5;
/// Longer words are identifiers or run-together text, not vocabulary.
pub const MAX_WORD_SCALARS: usize = 24;
/// Words one request may carry from the window.
pub const MAX_WINDOW_WORDS: usize = 256;
/// Words one request may carry from earlier writing in the app.
pub const MAX_LEARNED_WORDS: usize = 1024;

/// The words a request may finish a word with. Both lists hold only whole
/// words that [`Vocabulary::words`] accepts; anything else is ignored.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Vocabulary {
    /// Words visible in the same window as the field.
    pub window: Vec<String>,
    /// Words the writer used before in the same app.
    pub learned: Vec<String>,
}

impl Vocabulary {
    /// The window's words from `window` and the earlier writing's words from
    /// `learned`, each bounded.
    #[must_use]
    pub fn from_texts<S: AsRef<str>>(window: &str, learned: &[S]) -> Self {
        let mut learned_words = Vec::new();
        for text in learned {
            for word in Self::words(text.as_ref()) {
                if learned_words.len() < MAX_LEARNED_WORDS && !learned_words.contains(&word) {
                    learned_words.push(word);
                }
            }
        }
        let mut window = Self::words(window);
        window.truncate(MAX_WINDOW_WORDS);
        Self {
            window,
            learned: learned_words,
        }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.window.is_empty() && self.learned.is_empty()
    }

    /// The distinct whole words of `text`, in order: letters only, more than
    /// [`MIN_STEM_SCALARS`] and at most [`MAX_WORD_SCALARS`] long, with a
    /// separator or the text's edge on both sides. A word joined to others by
    /// a hyphen, underscore, apostrophe, digit or dot belongs to a longer
    /// token (`client.rs`, `don't`, `x-ray`) and is not evidence of its own.
    #[must_use]
    pub fn words(text: &str) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for (start, word) in text.unicode_word_indices() {
            if bounded_word(text, start, word) && !words.iter().any(|seen| seen == word) {
                words.push(word.to_owned());
            }
        }
        words
    }
}

/// Whether `word` could ever finish a typed word: letters only, more than
/// [`MIN_STEM_SCALARS`] and at most [`MAX_WORD_SCALARS`] long.
#[must_use]
pub fn learnable(word: &str) -> bool {
    let count = word.chars().count();
    count > MIN_STEM_SCALARS && count <= MAX_WORD_SCALARS && word_scalars(word)
}

/// The finished words of a draft worth learning, in order and each once:
/// [`Vocabulary::words`] without the word still being typed at its end, and
/// without the first word when the draft fills the wire's 512 scalars, where
/// it may be the clipped end of a longer one.
#[must_use]
pub fn written_words(before: &str) -> Vec<String> {
    let clipped = before.chars().count() >= crate::protocol::MAX_BEFORE_CHARS;
    let mut words: Vec<String> = Vec::new();
    for (start, word) in before.unicode_word_indices() {
        let end = start + word.len();
        if (start > 0 || !clipped)
            && end < before.len()
            && bounded_word(before, start, word)
            && !words.iter().any(|seen| seen == word)
        {
            words.push(word.to_owned());
        }
    }
    words
}

/// Where a completing word was found.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum Source {
    Window,
    Learned,
    Draft,
}

impl Source {
    const fn bit(self) -> u8 {
        match self {
            Self::Window => 1,
            Self::Learned => 2,
            Self::Draft => 4,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Window => "window",
            Self::Learned => "learned",
            Self::Draft => "draft",
        }
    }
}

/// The untyped rest of one vocabulary word.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Completion {
    pub suffix: String,
    pub word: String,
    pub sources: Vec<Source>,
}

/// Why no word was finished.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Abstention {
    /// The caret does not end a partial word of at least five letters.
    NoStem,
    /// The typed letters form an English function word, or the one fitting
    /// word only inflects the typed one: the writer may be done.
    KnownWord,
    NoMatch,
    /// Two or more words fit and none begins all the others; the choice is
    /// the writer's.
    Ambiguous,
    /// The one fitting word cannot be shown at this caret.
    UnsafeSuffix,
}

impl Abstention {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoStem => "no_stem",
            Self::KnownWord => "known_word",
            Self::NoMatch => "no_match",
            Self::Ambiguous => "ambiguous",
            Self::UnsafeSuffix => "unsafe_suffix",
        }
    }
}

#[cfg(feature = "local-model")]
/// The rest of the word ending at the caret, when exactly one longer word in
/// `vocabulary` or earlier in `before` starts with the typed letters, or one
/// such word begins all the others, matched case-sensitively and without
/// normalization.
///
/// # Errors
///
/// The [`Abstention`] that stopped it.
pub fn complete(
    before: &str,
    after: &str,
    language: WritingLanguage,
    vocabulary: &Vocabulary,
) -> Result<Completion, Abstention> {
    if after
        .chars()
        .next()
        .is_some_and(|next| !lexical_separator(next))
    {
        return Err(Abstention::NoStem);
    }
    let (start, stem) = stem(before, language).ok_or(Abstention::NoStem)?;
    let finished = typed_word_may_be_finished(before, start, stem, language);
    if finished && FUNCTION_WORDS.contains(&stem.to_lowercase().as_str()) {
        return Err(Abstention::KnownWord);
    }
    let mut candidates: BTreeMap<&str, u8> = BTreeMap::new();
    let listed = vocabulary
        .window
        .iter()
        .map(|word| (word.as_str(), Source::Window))
        .chain(
            vocabulary
                .learned
                .iter()
                .map(|word| (word.as_str(), Source::Learned)),
        );
    // The current stem is never evidence, and the draft's first word may be
    // the clipped end of a longer one.
    let draft = &before[..start];
    let drafted = draft
        .unicode_word_indices()
        .filter(|&(index, word)| index > 0 && bounded_word(draft, index, word))
        .map(|(_, word)| (word, Source::Draft));
    for (word, source) in listed.chain(drafted) {
        if word.len() > stem.len()
            && word.starts_with(stem)
            && word.chars().count() <= MAX_WORD_SCALARS
            && word_scalars(word)
            && language.accepts_output(word)
        {
            *candidates.entry(word).or_default() |= source.bit();
        }
    }
    // Byte order puts a word before every longer word it begins, so the first
    // candidate is the only one all others can extend (`almond`, `almonds`):
    // then its letters come next whichever the writer means.
    let (word, sources) = candidates
        .first_key_value()
        .map(|(word, sources)| (*word, *sources))
        .ok_or(Abstention::NoMatch)?;
    if !candidates
        .keys()
        .all(|candidate| candidate.starts_with(word))
    {
        return Err(Abstention::Ambiguous);
    }
    let suffix = &word[stem.len()..];
    // A finished word plus an ending (`crash` and `crashes`) is a likelier
    // pause after the word than a word still being typed.
    if finished && INFLECTIONS.contains(&suffix) {
        return Err(Abstention::KnownWord);
    }
    // Every competing whole word was counted first: an unsafe seam never turns
    // two plausible words into one choice.
    if !suffix.chars().next().is_some_and(char::is_alphabetic)
        || !language.accepts_output(suffix)
        || crate::segment::sanitize_suggestion(suffix).is_err()
        || crate::writing::validate_suggestion_shape(before, after, suffix).is_err()
    {
        return Err(Abstention::UnsafeSuffix);
    }
    Ok(Completion {
        suffix: suffix.to_owned(),
        word: word.to_owned(),
        sources: [Source::Window, Source::Learned, Source::Draft]
            .into_iter()
            .filter(|source| sources & source.bit() != 0)
            .collect(),
    })
}

#[cfg(feature = "local-model")]
/// The partial word ending `before`, with its byte offset, when it follows a
/// separator and has the length and script of a word worth finishing.
fn stem(before: &str, language: WritingLanguage) -> Option<(usize, &str)> {
    let (start, word) = before.unicode_word_indices().next_back()?;
    (start != 0
        && start + word.len() == before.len()
        && (MIN_STEM_SCALARS..MAX_WORD_SCALARS).contains(&word.chars().count())
        && before[..start]
            .chars()
            .next_back()
            .is_some_and(lexical_separator)
        && word_scalars(word)
        && language.accepts_output(word))
    .then_some((start, word))
}

#[cfg(feature = "local-model")]
/// English function words of three or more letters. Typed whole, they are far
/// more often finished than the start of a longer word (`the`, not `theme`).
#[cfg(feature = "local-model")]
const FUNCTION_WORDS: &[&str] = &[
    "about", "above", "after", "again", "against", "all", "also", "and", "any", "are", "because",
    "been", "before", "being", "below", "between", "both", "but", "can", "could", "did", "does",
    "down", "during", "each", "few", "for", "from", "had", "has", "have", "her", "here", "hers",
    "him", "his", "how", "into", "its", "just", "may", "might", "more", "most", "must", "nor",
    "not", "now", "off", "once", "only", "onto", "other", "our", "out", "over", "own", "same",
    "she", "should", "some", "such", "than", "that", "the", "their", "them", "then", "there",
    "these", "they", "this", "those", "through", "too", "under", "until", "upon", "very", "was",
    "were", "what", "when", "where", "which", "while", "who", "whom", "why", "will", "with",
    "would", "yet", "you", "your",
];

/// Endings that turn a finished English word into another form of it.
#[cfg(feature = "local-model")]
const INFLECTIONS: &[&str] = &["s", "es", "d", "ed", "ing", "er", "ers", "est", "ly"];

/// Whether an English writer may have finished the typed word: it is in the
/// lexicon and is lowercase, or starts a sentence where any word is
/// capitalized. A capital elsewhere marks a name, so `Mar` in `Hi Mar` still
/// looks for one.
fn typed_word_may_be_finished(
    before: &str,
    start: usize,
    stem: &str,
    language: WritingLanguage,
) -> bool {
    if language != WritingLanguage::English || !crate::writing::known_english_word(stem) {
        return false;
    }
    let capitalized = stem.chars().next().is_some_and(char::is_uppercase);
    !capitalized || sentence_start(&before[..start])
}

#[cfg(feature = "local-model")]
/// Whether a word starting after `before` begins a sentence or a line.
fn sentence_start(before: &str) -> bool {
    before.ends_with('\n')
        || before
            .trim_end()
            .chars()
            .next_back()
            .is_none_or(|last| matches!(last, '.' | '!' | '?' | ':' | '"' | '\u{201c}'))
}

fn bounded_word(text: &str, start: usize, word: &str) -> bool {
    let count = word.chars().count();
    count > MIN_STEM_SCALARS
        && count <= MAX_WORD_SCALARS
        && word_scalars(word)
        && text[..start]
            .chars()
            .next_back()
            .is_none_or(lexical_separator)
        && text[start + word.len()..]
            .chars()
            .next()
            .is_none_or(lexical_separator)
}

/// Letters, with the joiners and combining marks of the supported scripts.
#[must_use]
pub fn word_scalars(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|character| {
            character.is_alphabetic()
                || matches!(
                    character,
                    '\u{200c}' | '\u{0300}'..='\u{036f}' | '\u{064b}'..='\u{065f}' | '\u{0670}'
                )
        })
        && crate::segment::valid_orthographic_joiners(word)
}

/// Whitespace and the punctuation that ends a word without joining it to the
/// next: not hyphens, apostrophes, underscores, slashes or dots inside
/// tokens such as file names.
#[must_use]
pub fn lexical_separator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '.' | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '"'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{201e}'
                | '\u{00ab}'
                | '\u{00bb}'
                | '\u{2026}'
                | '\u{060c}'
                | '\u{061b}'
                | '\u{061f}'
        )
}

#[cfg(test)]
mod tests;
