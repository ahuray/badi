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
}

pub fn sanitize_suggestion(raw: &str) -> Result<String, OutputError> {
    if raw.chars().any(is_forbidden_output_scalar) {
        return Err(OutputError::ForbiddenControl);
    }

    let mut output = String::new();
    let mut words = 0_usize;
    for boundary in raw.split_word_bounds() {
        let boundary_words = boundary.unicode_words().count();
        if boundary_words > 0 && words.saturating_add(boundary_words) > MAX_SUGGESTION_WORDS {
            break;
        }
        for character in boundary.chars() {
            if output.chars().count() == MAX_SUGGESTION_CHARS {
                break;
            }
            output.push(character);
        }
        words = words.saturating_add(boundary_words);
        if output.chars().count() == MAX_SUGGESTION_CHARS {
            break;
        }
    }

    while output.ends_with(char::is_whitespace) {
        output.pop();
    }
    if output.is_empty() {
        Err(OutputError::Empty)
    } else {
        Ok(output)
    }
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
    let end = leading_bytes + part_end.unwrap_or(0);
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
mod tests {
    use serde::Deserialize;
    use unicode_segmentation::UnicodeSegmentation;

    use super::{OutputError, accept_word, sanitize_suggestion};

    #[derive(Deserialize)]
    struct FixtureSet {
        v: u8,
        algorithm: String,
        cases: Vec<FixtureCase>,
    }

    #[derive(Deserialize)]
    struct FixtureCase {
        name: String,
        input: String,
        accepted: String,
        remainder: String,
    }

    #[test]
    fn shared_protocol_accept_word_fixtures_match() {
        let fixtures: FixtureSet =
            serde_json::from_str(include_str!("../../protocol/v1/accept-word-fixtures.json"))
                .expect("shared accept-word fixtures");
        assert_eq!(fixtures.v, 1);
        assert_eq!(
            fixtures.algorithm,
            "leading_whitespace_plus_first_unicode_word_or_grapheme_v1"
        );
        assert!(fixtures.cases.len() >= 7);
        for fixture in fixtures.cases {
            let parts = accept_word(&fixture.input);
            assert_eq!(
                parts.accepted, fixture.accepted,
                "{} accepted",
                fixture.name
            );
            assert_eq!(
                parts.remainder, fixture.remainder,
                "{} remainder",
                fixture.name
            );
        }
    }

    #[test]
    fn attaches_leading_space_to_unicode_word() {
        let parts = accept_word("  café κόσμε");
        assert_eq!(parts.accepted, "  café");
        assert_eq!(parts.remainder, " κόσμε");

        let combining = accept_word(" e\u{301}lan vital");
        assert_eq!(combining.accepted, " e\u{301}lan");
        assert_eq!(combining.remainder, " vital");
    }

    #[test]
    fn handles_emoji_without_splitting_a_grapheme() {
        let parts = accept_word(" 👩🏽‍💻 next");
        assert_eq!(parts.accepted, " 👩🏽‍💻");
        assert_eq!(parts.remainder, " next");
        assert!(parts.accepted.graphemes(true).any(|part| part == "👩🏽‍💻"));
    }

    #[test]
    fn output_is_bounded_by_scalars_and_words() {
        let long = " one two three four five six seven eight nine ten";
        let output = sanitize_suggestion(long).expect("safe output");
        assert!(output.chars().count() <= 64);
        assert_eq!(output.unicode_words().count(), 8);
    }

    #[test]
    fn rejects_controls_instead_of_escaping_them() {
        assert_eq!(
            sanitize_suggestion(" do it\nnow"),
            Err(OutputError::ForbiddenControl)
        );
        assert_eq!(
            sanitize_suggestion(" press\tTab"),
            Err(OutputError::ForbiddenControl)
        );
        assert_eq!(
            sanitize_suggestion(" escape\u{1b}"),
            Err(OutputError::ForbiddenControl)
        );
        assert_eq!(
            sanitize_suggestion(" two\u{2028}lines"),
            Err(OutputError::ForbiddenControl)
        );
    }

    #[test]
    fn rejects_unicode_format_controls() {
        for hostile in [
            "safe\u{202e}hostile",
            "safe\u{2066}hostile",
            "safe\u{200b}hostile",
            "safe\u{00ad}hostile",
            "safe\u{13430}hostile",
            "safe\u{e007f}hostile",
        ] {
            assert_eq!(
                sanitize_suggestion(hostile),
                Err(OutputError::ForbiddenControl)
            );
        }
    }
}
