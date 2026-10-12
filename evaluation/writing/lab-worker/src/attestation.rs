//! Shared word-boundary checks for Lab context lookup.
//! These checks are neither dictionary validation nor authority to edit a field.

pub(super) fn word_scalars(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|character| {
            character.is_alphabetic()
                || matches!(
                    character,
                    '\u{200c}' | '\u{0300}'..='\u{036f}' | '\u{064b}'..='\u{065f}' | '\u{0670}'
                )
        })
        && badi_broker::segment::valid_orthographic_joiners(word)
}

pub(super) fn lexical_separator(character: char) -> bool {
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
