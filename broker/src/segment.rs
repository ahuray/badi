use unicode_segmentation::UnicodeSegmentation;

use crate::protocol::{MAX_SUGGESTION_CHARS, MAX_SUGGESTION_WORDS};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedParts {
    pub accepted: String,
    pub remainder: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputError {
    Empty,
    ForbiddenControl,
    TooLong,
    InvalidShape,
}

pub fn sanitize_suggestion(raw: &str) -> Result<String, OutputError> {
    if raw.is_empty() || raw.chars().all(char::is_whitespace) {
        return Err(OutputError::Empty);
    }
    if !valid_orthographic_joiners(raw)
        || raw
            .chars()
            .any(|character| character != '\u{200c}' && is_forbidden_output_scalar(character))
    {
        return Err(OutputError::ForbiddenControl);
    }
    if has_invalid_spacing(raw) {
        return Err(OutputError::InvalidShape);
    }
    if raw.chars().count() > MAX_SUGGESTION_CHARS
        || raw.unicode_words().count() > MAX_SUGGESTION_WORDS
    {
        return Err(OutputError::TooLong);
    }
    // One trailing ASCII space is the mid-sentence separator. Every other
    // trailing whitespace, including a second space, stays invalid here;
    // `validate_completion_shape` decides when that one space may be shown.
    Ok(raw.to_owned())
}

/// Keep this exact Arabic-letter set aligned with the wire schemas and adapters.
#[must_use]
pub fn valid_orthographic_joiners(value: &str) -> bool {
    value.char_indices().all(|(index, character)| {
        character != '\u{200c}'
            || (value[..index]
                .chars()
                .next_back()
                .is_some_and(is_arabic_joiner_letter)
                && value[index + character.len_utf8()..]
                    .chars()
                    .next()
                    .is_some_and(is_arabic_joiner_letter))
    })
}

const fn is_arabic_joiner_letter(character: char) -> bool {
    matches!(character,
        '\u{0620}'..='\u{063f}' | '\u{0641}'..='\u{064a}' | '\u{066e}'..='\u{066f}'
        | '\u{0671}'..='\u{06d3}' | '\u{06d5}' | '\u{06e5}'..='\u{06e6}'
        | '\u{06ee}'..='\u{06ef}' | '\u{06fa}'..='\u{06fc}' | '\u{06ff}')
}

pub fn validate_suggestion_shape(
    before: &str,
    after: &str,
    suggestion: &str,
) -> Result<(), OutputError> {
    validate_completion_shape(before, after, suggestion, false)
}

pub(crate) fn validate_completion_shape(
    before: &str,
    after: &str,
    suggestion: &str,
    allow_word_suffix: bool,
) -> Result<(), OutputError> {
    let first = suggestion.chars().next().ok_or(OutputError::Empty)?;
    let last = suggestion.chars().next_back().ok_or(OutputError::Empty)?;
    let before_last = before.chars().next_back();
    let after_first = after.chars().next();

    if (before.is_empty() && first.is_whitespace())
        || has_invalid_spacing(suggestion)
        || (before_last == Some(' ') && first == ' ')
        || (!allow_word_suffix && before_last.is_some_and(|left| needs_word_separator(left, first)))
        || after_first.is_some_and(|right| needs_word_separator(last, right))
    {
        return Err(OutputError::InvalidShape);
    }

    let candidate = suggestion.trim_start().to_lowercase();
    let normalized_before = before.trim_end().to_lowercase();
    let normalized_after = after.trim_start().to_lowercase();
    if candidate.is_empty()
        || has_boundary_suffix(&normalized_before, &candidate)
        || has_boundary_prefix(&normalized_after, &candidate)
    {
        return Err(OutputError::InvalidShape);
    }

    let repeats_before_token = before
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
        && before
            .unicode_words()
            .next_back()
            .zip(suggestion.unicode_words().next())
            .is_some_and(|(left, right)| left.to_lowercase() == right.to_lowercase());
    let repeats_after_token = suggestion
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
        && after
            .trim_start()
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
        && suggestion
            .unicode_words()
            .next_back()
            .zip(after.unicode_words().next())
            .is_some_and(|(left, right)| left.to_lowercase() == right.to_lowercase());
    if repeats_before_token || repeats_after_token {
        return Err(OutputError::InvalidShape);
    }
    // The space belongs to the suggestion only when it keeps the insertion
    // from joining the next word. Typing into that word may continue it.
    if last == ' ' {
        let next_is_word = after_first.is_some_and(char::is_alphanumeric);
        let caret_follows_whitespace = before_last.is_some_and(char::is_whitespace);
        let continues_typed_word = allow_word_suffix
            && before_last.is_some_and(char::is_alphanumeric)
            && first.is_alphanumeric();
        if !(next_is_word && (caret_follows_whitespace || continues_typed_word)) {
            return Err(OutputError::InvalidShape);
        }
    }
    Ok(())
}

fn has_invalid_spacing(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_whitespace() && character != ' ')
        || value.contains("  ")
}

fn needs_word_separator(left: char, right: char) -> bool {
    left.is_alphanumeric()
        && right.is_alphanumeric()
        && !(is_unspaced_script(left) && is_unspaced_script(right))
}

const fn is_unspaced_script(character: char) -> bool {
    matches!(
        character,
        '\u{1100}'..='\u{11ff}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{3100}'..='\u{312f}'
            | '\u{3130}'..='\u{318f}'
            | '\u{31a0}'..='\u{31bf}'
            | '\u{31f0}'..='\u{31ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7af}'
            | '\u{d7b0}'..='\u{d7ff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff66}'..='\u{ff9d}'
            | '\u{20000}'..='\u{2fa1f}'
    )
}

fn has_boundary_suffix(value: &str, candidate: &str) -> bool {
    let Some(start) = value.len().checked_sub(candidate.len()) else {
        return false;
    };
    value.ends_with(candidate)
        && (start == 0
            || !value[..start]
                .chars()
                .next_back()
                .is_some_and(char::is_alphanumeric)
            || !candidate.chars().next().is_some_and(char::is_alphanumeric))
}

fn has_boundary_prefix(value: &str, candidate: &str) -> bool {
    let Some(remainder) = value.strip_prefix(candidate) else {
        return false;
    };
    remainder.is_empty()
        || !candidate
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
        || !remainder.chars().next().is_some_and(char::is_alphanumeric)
}

#[must_use]
pub fn accept_word(text: &str) -> AcceptedParts {
    let leading_bytes = text
        .char_indices()
        .take_while(|(_, character)| character.is_whitespace())
        .map(|(index, character)| index + character.len_utf8())
        .last()
        .unwrap_or(0);
    let tail = &text[leading_bytes..];
    let word_end = tail
        .unicode_word_indices()
        .next()
        .filter(|(start, _)| *start == 0)
        .map(|(_, word)| word.len());
    let part_end = word_end.or_else(|| {
        tail.grapheme_indices(true)
            .next()
            .map(|(index, grapheme)| index + grapheme.len())
    });
    let mut end = leading_bytes + part_end.unwrap_or(0);
    // Word acceptance on the last word includes the one space that
    // separates it from the text after the caret. A space before a later
    // word stays with that word.
    if &text[end..] == " " {
        end += 1;
    }
    AcceptedParts {
        accepted: text[..end].to_owned(),
        remainder: text[end..].to_owned(),
    }
}

fn is_forbidden_output_scalar(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{00ad}'
                | '\u{0600}'..='\u{0605}'
                | '\u{061c}'
                | '\u{06dd}'
                | '\u{070f}'
                | '\u{0890}'..='\u{0891}'
                | '\u{08e2}'
                | '\u{180e}'
                | '\u{200b}'..='\u{200f}'
                | '\u{2028}'..='\u{202e}'
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

#[cfg(test)]
mod tests;
