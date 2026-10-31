//! Explicit revision of one selection (ADR 0012): a grammar check, a
//! refined wording, or one synonym for a word or short phrase (ADR 0013),
//! from an instruction-tuned model. This module builds what the model is
//! asked and decides whether its answer may be shown. It never reads or
//! edits a field.

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

pub mod runtime;
pub mod service;

/// The longest selection a revision accepts, in Unicode scalar values.
pub const MAX_SELECTED_SCALARS: usize = 600;

/// The longest revision shown, in Unicode scalar values: the refine mode's
/// length bound for the longest selection.
pub const MAX_REVISED_SCALARS: usize = 1_100;

/// How long a shown revision may be applied.
pub const REVISION_TTL_MS: u64 = 60_000;

/// A grammar check of this many words or fewer may change all of them:
/// "its ok" becomes "it's okay".
const SHORT_SELECTION_WORDS: usize = 3;

/// The most words a selection may have for a synonym: a word or a short
/// phrase, never a whole clause.
pub const MAX_SYNONYM_WORDS: usize = 3;

/// Whether `selected` (its surrounding spaces aside) is a word or short
/// phrase a synonym may replace: one to [`MAX_SYNONYM_WORDS`] words with a
/// letter each, and no digit, address or sentence mark, which a synonym
/// must never silently drop or invent.
#[must_use]
pub fn synonym_selection(selected: &str) -> bool {
    let core = selected.trim();
    let words: Vec<&str> = core.split_whitespace().collect();
    safe_revision_text(core)
        && (1..=MAX_SYNONYM_WORDS).contains(&words.len())
        && words
            .iter()
            .all(|word| word.chars().any(char::is_alphabetic))
        && !core.chars().any(|character| {
            character.is_ascii_digit() || matches!(character, '@' | '.' | '!' | '?' | ';' | ':')
        })
}

/// What a revision does to the selection.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionMode {
    /// Fix grammar, spelling, capitalization and punctuation; keep wording.
    Grammar,
    /// Rewrite for clarity and natural flow; keep meaning and facts.
    Refine,
    /// Replace the selected word or short phrase with one synonym that
    /// fits its sentence; keep everything else (ADR 0013).
    Synonyms,
}

impl RevisionMode {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Grammar => "grammar",
            Self::Refine => "refine",
            Self::Synonyms => "synonyms",
        }
    }
}

/// A language a revision is offered in, with the one worked example the
/// model sees. An example in another language made the model answer in that
/// language.
#[derive(Debug)]
pub struct RevisionLanguage {
    pub tag: &'static str,
    pub name: &'static str,
    example: (&'static str, &'static str),
    /// One worked synonym: the same text with only its last word replaced.
    /// The gate refuses its answer word for any other selection, which a
    /// small model otherwise copies when unsure.
    synonym_example: (&'static str, &'static str),
}

const LANGUAGES: [RevisionLanguage; 5] = [
    RevisionLanguage {
        tag: "en",
        name: "English",
        example: (
            "She dont like going their on monday.",
            "She doesn't like going there on Monday.",
        ),
        synonym_example: (
            "The view from the hill is beautiful",
            "The view from the hill is lovely",
        ),
    },
    RevisionLanguage {
        tag: "de",
        name: "German",
        example: (
            "Er hat nach Hause gegangen und hat das Buch gelest.",
            "Er ist nach Hause gegangen und hat das Buch gelesen.",
        ),
        synonym_example: (
            "Der Ausblick vom Hügel ist schön",
            "Der Ausblick vom Hügel ist herrlich",
        ),
    },
    RevisionLanguage {
        tag: "fr",
        name: "French",
        example: (
            "Nous avons aller au cinéma et nous avons manger des pop-corn.",
            "Nous sommes allés au cinéma et nous avons mangé du pop-corn.",
        ),
        synonym_example: (
            "La vue depuis la colline est belle",
            "La vue depuis la colline est magnifique",
        ),
    },
    RevisionLanguage {
        tag: "es",
        name: "Spanish",
        example: (
            "Ella no sabe donde esta el parada de autobús.",
            "Ella no sabe dónde está la parada de autobús.",
        ),
        synonym_example: (
            "La vista desde la colina es bonita",
            "La vista desde la colina es hermosa",
        ),
    },
    RevisionLanguage {
        tag: "pt",
        name: "Portuguese",
        example: (
            "Eles vai chegar amanhã e nós vamos buscar eles no aeroporto.",
            "Eles vão chegar amanhã e nós vamos buscá-los no aeroporto.",
        ),
        synonym_example: ("A vista da colina é bonita", "A vista da colina é linda"),
    },
];

/// The revision language of a BCP 47 `tag`, by its primary subtag.
#[must_use]
pub fn language(tag: &str) -> Option<&'static RevisionLanguage> {
    let primary = tag.split('-').next()?.to_ascii_lowercase();
    LANGUAGES.iter().find(|language| language.tag == primary)
}

/// The longest sentence start the model reads with a mid-sentence selection.
const LEAD_SCALARS: usize = 160;

/// The start of the sentence a mid-sentence selection continues: `before`
/// after its last sentence end, at most [`LEAD_SCALARS`] long and starting
/// at a word; empty at a sentence start. The model reads it with the
/// selection, so a clause keeps its grammar (a German clause after "dass"
/// keeps its verb last), and [`accept`] requires it back unchanged.
#[must_use]
pub fn sentence_lead(before: &str) -> &str {
    let start = before
        .rfind(['.', '!', '?', '…', '\n', '\r'])
        .map_or(0, |end| {
            end + before[end..].chars().next().map_or(1, char::len_utf8)
        });
    let lead = before[start..].trim_start();
    if lead.trim().is_empty() {
        return "";
    }
    let Some((cut, _)) = lead.char_indices().rev().nth(LEAD_SCALARS - 1) else {
        return lead;
    };
    // Begin at the next word, never inside one.
    lead[cut..]
        .find(char::is_whitespace)
        .map_or("", |space| lead[cut + space..].trim_start())
}

/// What the model is asked to revise: the selection, after its sentence's
/// start when it begins mid-sentence.
fn model_text(before: &str, selected: &str) -> String {
    format!("{}{selected}", sentence_lead(before))
        .trim()
        .to_owned()
}

/// The conversation the model sees: what to do, one worked example in the
/// field's language, then the selection (with its sentence's start, see
/// [`sentence_lead`]). The text around it decides case and final marks
/// afterwards ([`accept`]).
#[must_use]
pub fn messages(
    mode: RevisionMode,
    language: &RevisionLanguage,
    before: &str,
    selected: &str,
) -> [(&'static str, String); 4] {
    messages_excluding(mode, language, before, selected, &[])
}

/// [`messages`], with synonyms already shown for this selection named as
/// words not to use again, so a repeated request offers another one.
#[must_use]
pub fn messages_excluding(
    mode: RevisionMode,
    language: &RevisionLanguage,
    before: &str,
    selected: &str,
    excluded: &[String],
) -> [(&'static str, String); 4] {
    let name = language.name;
    let system = match mode {
        RevisionMode::Grammar => format!(
            "You are a careful {name} proofreader. Fix every error in the user's {name} text: \
             grammar (verb forms, auxiliaries and tenses, agreement, articles, prepositions, word \
             order), spelling, capitalization and punctuation. Keep it in {name}, keep its meaning, \
             tone, names and numbers, and keep words that are already correct. Reply with the \
             corrected text only."
        ),
        RevisionMode::Refine => format!(
            "You are an editor. Rewrite the user's {name} text so it is correct, clear and natural \
             {name}. Keep it in {name}, keep its meaning, names and numbers, add no facts, and keep \
             about the same length. Reply with the rewritten text only."
        ),
        RevisionMode::Synonyms => {
            let core = selected.trim();
            let avoid = if excluded.is_empty() {
                String::new()
            } else {
                format!(" Do not use: {}.", excluded.join(", "))
            };
            format!(
                "You are a careful {name} editor. The user's {name} text ends with the selected \
                 word or short phrase \"{core}\". Reply with the same text, its start unchanged, \
                 with only that selected word or phrase at its end replaced by one synonym that \
                 fits the sentence. Keep it in {name}, keep its meaning, names and numbers, and \
                 add no facts.{avoid} Reply with the text only."
            )
        }
    };
    let (example, answer) = if mode == RevisionMode::Synonyms {
        language.synonym_example
    } else {
        language.example
    };
    [
        ("system", system),
        ("user", example.to_owned()),
        ("assistant", answer.to_owned()),
        ("user", model_text(before, selected)),
    ]
}

/// `messages` in the `ChatML` template of Qwen3, with an empty reasoning block
/// so the model answers at once.
#[must_use]
pub fn chatml_prompt(messages: &[(&str, String)]) -> String {
    let mut prompt = String::new();
    for (role, content) in messages {
        prompt.push_str("<|im_start|>");
        prompt.push_str(role);
        prompt.push('\n');
        prompt.push_str(content);
        prompt.push_str("<|im_end|>\n");
    }
    prompt.push_str("<|im_start|>assistant\n<think>\n\n</think>\n\n");
    prompt
}

/// Why a model answer is not shown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevisionAbstain {
    /// The selection needs no change.
    Unchanged,
    /// More than one line, markup, or nothing.
    Shape,
    /// Another language or script came back.
    Language,
    /// A number, address, handle or name of the selection is missing.
    Facts,
    /// Much shorter or longer than the selection.
    Length,
    /// A grammar check that rewrote most of the words.
    Edits,
    /// The sentence start given with a mid-sentence selection came back
    /// changed, so where the selection's revision begins is unknown.
    Context,
}

impl RevisionAbstain {
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unchanged => "unchanged",
            Self::Shape => "shape",
            Self::Language => "language",
            Self::Facts => "facts",
            Self::Length => "length",
            Self::Edits => "edits",
            Self::Context => "context",
        }
    }
}

/// The selection's replacement when the model's `output` may be shown: the
/// same leading and trailing whitespace, the selection's own first-letter
/// case when it starts mid-sentence, no added final mark when text follows
/// on the line, and the gates of ADR 0012.
///
/// # Errors
/// The [`RevisionAbstain`] class of an answer that is not shown.
pub fn accept(
    mode: RevisionMode,
    language: &RevisionLanguage,
    before: &str,
    selected: &str,
    after: &str,
    output: &str,
) -> Result<String, RevisionAbstain> {
    if mode == RevisionMode::Synonyms && !synonym_selection(selected) {
        return Err(RevisionAbstain::Shape);
    }
    let core = selected.trim();
    let lead = &selected[..selected.len() - selected.trim_start().len()];
    let trail = &selected[selected.trim_end().len()..];
    let answer = strip_quotes(core, output.trim());
    let answer =
        strip_sentence_lead(answer, sentence_lead(before), lead).ok_or(RevisionAbstain::Context)?;
    let mut revised = answer.trim().to_owned();
    if revised.is_empty()
        || !safe_revision_text(&revised)
        || (["<", ">", "`"].iter().any(|mark| revised.contains(mark))
            && !["<", ">", "`"].iter().any(|mark| core.contains(mark)))
    {
        return Err(RevisionAbstain::Shape);
    }
    keep_first_letter_case(before, core, &mut revised);
    drop_added_final_mark(core, after, &mut revised);
    // A synonym never adds the sentence's final mark, at the line's end either.
    if mode == RevisionMode::Synonyms
        && !core.ends_with(['.', '!', '?', '…'])
        && revised.ends_with('.')
        && !revised.ends_with("...")
    {
        revised.pop();
    }
    if revised == core {
        return Err(RevisionAbstain::Unchanged);
    }
    // A synonym replaces its word; an answer that still holds it (the
    // model answered "schnell alternativ" for "schnell") is no synonym.
    if mode == RevisionMode::Synonyms && still_holds(core, &revised) {
        return Err(RevisionAbstain::Shape);
    }
    if mode == RevisionMode::Synonyms && copies_example(language, core, &revised) {
        return Err(RevisionAbstain::Shape);
    }
    if !same_language(language, core, &revised) {
        return Err(RevisionAbstain::Language);
    }
    if !keeps_facts(language, core, &revised) {
        return Err(RevisionAbstain::Facts);
    }
    if !length_fits(mode, core, &revised) {
        return Err(RevisionAbstain::Length);
    }
    if mode == RevisionMode::Grammar
        && core.split_whitespace().count() > SHORT_SELECTION_WORDS
        && changed_word_share(core, &revised) > 0.5
    {
        return Err(RevisionAbstain::Edits);
    }
    Ok(format!("{lead}{revised}{trail}"))
}

/// Whether the revised words still contain the selected words, in order,
/// case aside: a shared word of a phrase ("a great deal" for "a lot") is
/// fine, the phrase itself is not.
fn still_holds(core: &str, revised: &str) -> bool {
    let wanted: Vec<String> = core.split_whitespace().map(str::to_lowercase).collect();
    let words: Vec<String> = revised.split_whitespace().map(str::to_lowercase).collect();
    !wanted.is_empty()
        && words
            .windows(wanted.len())
            .any(|window| window == wanted.as_slice())
}

/// Whether the answer copies the worked example's synonym for a different
/// selection: unsure, the model answered "lovely" for "fast". The example's
/// own word may of course take it.
fn copies_example(language: &RevisionLanguage, core: &str, revised: &str) -> bool {
    let (example, answer) = language.synonym_example;
    let (Some(example_word), Some(answer_word)) = (
        example.split_whitespace().last(),
        answer.split_whitespace().last(),
    ) else {
        return false;
    };
    !core.eq_ignore_ascii_case(example_word)
        && revised
            .split_whitespace()
            .any(|word| word.eq_ignore_ascii_case(answer_word))
}

/// The answer without the sentence start the model was given, which must
/// come back as it was (its first letter may change case), followed by the
/// selection's own leading space.
fn strip_sentence_lead<'a>(answer: &'a str, sentence: &str, space: &str) -> Option<&'a str> {
    if sentence.is_empty() {
        return Some(answer);
    }
    let expected = format!("{sentence}{space}");
    let mut given = expected.chars();
    let mut got = answer.chars();
    let first = given.next()?;
    let answered = got.next()?;
    let same_first = first == answered || first.to_lowercase().eq(answered.to_lowercase());
    let rest = given.as_str();
    let tail = got.as_str();
    // A trailing space the model may join to the selection's first word.
    let rest = if tail.starts_with(rest) {
        rest
    } else {
        rest.trim_end()
    };
    (same_first && tail.starts_with(rest)).then(|| &tail[rest.len()..])
}

/// Quotes around the whole answer that the selection did not have.
fn strip_quotes<'a>(core: &str, output: &'a str) -> &'a str {
    for (open, close) in [("\"", "\""), ("“", "”"), ("„", "“"), ("«", "»"), ("'", "'")] {
        if let Some(inner) = output
            .strip_prefix(open)
            .and_then(|rest| rest.strip_suffix(close))
        {
            if !core.starts_with(open) {
                return inner.trim();
            }
        }
    }
    output
}

/// A selection that starts mid-sentence in lowercase stays lowercase: the
/// model sees it alone and treats it as a sentence start.
fn keep_first_letter_case(before: &str, core: &str, revised: &mut String) {
    let mid_sentence = before
        .trim_end()
        .chars()
        .next_back()
        .is_some_and(|last| !matches!(last, '.' | '!' | '?' | '…' | ':' | '\n'));
    let original = core.chars().next();
    let first = revised.chars().next();
    if let (true, Some(original), Some(first)) = (mid_sentence, original, first) {
        if original.is_lowercase() && first.is_uppercase() {
            let lowered: String = first.to_lowercase().collect();
            revised.replace_range(..first.len_utf8(), &lowered);
        }
    }
}

/// A final period the model added is dropped when the line goes on after
/// the selection without a mark of its own.
fn drop_added_final_mark(core: &str, after: &str, revised: &mut String) {
    let line = after
        .split(['\n', '\r'])
        .next()
        .unwrap_or_default()
        .trim_start();
    // Words or marks of its own follow: "plan., if" would be wrong either way.
    let continues = !line.is_empty();
    let had_mark = core.ends_with(['.', '!', '?', '…']);
    if continues && !had_mark && revised.ends_with('.') && !revised.ends_with("...") {
        revised.pop();
    }
}

/// The answer is in the field's language: a confident detection must agree,
/// and every letter must be Latin, as Badi's revision languages are.
fn same_language(language: &RevisionLanguage, core: &str, revised: &str) -> bool {
    let latin = |text: &str| {
        text.chars()
            .filter(|character| character.is_alphabetic())
            .all(|character| matches!(character, 'A'..='Z' | 'a'..='z' | '\u{c0}'..='\u{24f}'))
    };
    if latin(core) && !latin(revised) {
        return false;
    }
    match crate::language::confident_language(revised) {
        Some(detected) => detected_tag(detected).is_none_or(|tag| tag == language.tag),
        None => true,
    }
}

/// A confidently detected language as a revision tag; another Latin-script
/// European language is "other" (a mismatch), anything else unknown.
fn detected_tag(language: whatlang::Lang) -> Option<&'static str> {
    match language {
        whatlang::Lang::Eng => Some("en"),
        whatlang::Lang::Deu => Some("de"),
        whatlang::Lang::Fra => Some("fr"),
        whatlang::Lang::Spa => Some("es"),
        whatlang::Lang::Por => Some("pt"),
        whatlang::Lang::Ita
        | whatlang::Lang::Nld
        | whatlang::Lang::Pol
        | whatlang::Lang::Cat
        | whatlang::Lang::Ron => Some("other"),
        _ => None,
    }
}

/// Numbers, addresses and handles come back unchanged, and (outside German,
/// which capitalizes nouns) so does every capitalized word after the first.
fn keeps_facts(language: &RevisionLanguage, core: &str, revised: &str) -> bool {
    let tokens = |text: &str, keep: &dyn Fn(&str) -> bool| -> Vec<String> {
        let mut found: Vec<String> = text
            .split_whitespace()
            .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric() && c != '@' && c != '/'))
            .filter(|token| keep(token))
            .map(str::to_owned)
            .collect();
        found.sort();
        found
    };
    let digits = |token: &str| token.chars().any(|c| c.is_ascii_digit());
    let address =
        |token: &str| token.contains('@') || token.contains("://") || token.starts_with("www.");
    if tokens(core, &digits) != tokens(revised, &digits)
        || tokens(core, &address) != tokens(revised, &address)
    {
        return false;
    }
    if language.tag == "de" {
        return true;
    }
    let names: Vec<&str> = core
        .unicode_words()
        .skip(1)
        .filter(|word| {
            let mut letters = word.chars();
            letters.next().is_some_and(char::is_uppercase) && letters.any(char::is_lowercase)
        })
        .collect();
    let kept: Vec<&str> = revised.unicode_words().collect();
    names.iter().all(|name| kept.contains(name))
}

/// Text a revision may read or write: one line with no control, format
/// or line-separator character, and no space other than U+0020 and the
/// no-break spaces French sets before its marks.
#[must_use]
pub fn safe_revision_text(text: &str) -> bool {
    text.chars().all(|character| {
        !(character.is_control()
            || (character.is_whitespace() && !matches!(character, ' ' | '\u{a0}' | '\u{202f}'))
            || is_format_character(character))
    })
}

/// Unicode general category Cf, which can reorder or hide text: soft
/// hyphen, Arabic marks, zero-width and bidirectional controls, tags.
const fn is_format_character(character: char) -> bool {
    matches!(
        character,
        '\u{ad}'
            | '\u{600}'..='\u{605}'
            | '\u{61c}'
            | '\u{6dd}'
            | '\u{70f}'
            | '\u{890}'..='\u{891}'
            | '\u{8e2}'
            | '\u{180e}'
            | '\u{200b}'..='\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206f}'
            | '\u{feff}'
            | '\u{fff9}'..='\u{fffb}'
            | '\u{110bd}'
            | '\u{110cd}'
            | '\u{13430}'..='\u{1343f}'
            | '\u{1bca0}'..='\u{1bca3}'
            | '\u{1d173}'..='\u{1d17a}'
            | '\u{e0001}'
            | '\u{e0020}'..='\u{e007f}'
    )
}

fn length_fits(mode: RevisionMode, core: &str, revised: &str) -> bool {
    let original = core.chars().count();
    let length = revised.chars().count();
    if length > MAX_REVISED_SCALARS {
        return false;
    }
    let (low, high) = match mode {
        RevisionMode::Grammar => (0.5, 1.6),
        RevisionMode::Refine | RevisionMode::Synonyms => (0.4, 1.8),
    };
    // Short selections get a little absolute room: "its" becomes "it's".
    let slack = 12.0;
    #[allow(clippy::cast_precision_loss)]
    let (original, length) = (original as f64, length as f64);
    length >= original * low - slack && length <= original * high + slack
}

/// The share of words a revision changed: one minus their longest common
/// subsequence over the longer of the two word counts. Case and the marks
/// around a word do not count, so "home" and "Home." are the same word.
#[must_use]
pub fn changed_word_share(original: &str, revised: &str) -> f64 {
    let words = |text: &str| -> Vec<String> {
        text.split_whitespace()
            .map(|word| {
                word.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .collect()
    };
    let (a, b) = (words(original), words(revised));
    let longest = a.len().max(b.len());
    if longest == 0 {
        return 0.0;
    }
    let mut row = vec![0_usize; b.len() + 1];
    for word in &a {
        let mut diagonal = 0;
        for (index, other) in b.iter().enumerate() {
            let above = row[index + 1];
            row[index + 1] = if word == other {
                diagonal + 1
            } else {
                above.max(row[index])
            };
            diagonal = above;
        }
    }
    #[allow(clippy::cast_precision_loss)]
    let share = 1.0 - row[b.len()] as f64 / longest as f64;
    share
}

#[cfg(test)]
mod tests;
