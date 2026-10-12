//! Lab-only earlier-field window. The start moves in large jumps so consecutive
//! keystrokes keep the same prompt prefix. This is not installed in the broker.

/// About 200 English tokens, at four scalars per token.
pub(crate) const WINDOW_CHARS: usize = 800;
/// About 64 tokens. The window start stays put until the field grows by this much.
pub(crate) const JUMP_CHARS: usize = 256;

/// The earlier field text that stays a stable prompt prefix.
pub(crate) fn stable_prefix(earlier: &str) -> &str {
    let count = earlier.chars().count();
    if count <= WINDOW_CHARS {
        return earlier;
    }
    let start_chars = (count - WINDOW_CHARS) / JUMP_CHARS * JUMP_CHARS;
    let start = byte_at_char(earlier, start_chars);
    let rest = &earlier[start..];
    match rest.find(char::is_whitespace) {
        Some(index) => rest[index..].trim_start(),
        None => rest,
    }
}

fn byte_at_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_short_field_is_kept_whole() {
        assert_eq!(stable_prefix("Earlier note. "), "Earlier note. ");
    }

    #[test]
    fn the_start_stays_put_until_the_field_grows_by_a_jump() {
        let stem = format!("note {}", "a".repeat(WINDOW_CHARS));
        let more = format!("{stem}{}", "b".repeat(100));
        let first = stable_prefix(&stem);
        assert!(stable_prefix(&more).starts_with(first));
        let jumped = format!("{stem}{}", "b".repeat(JUMP_CHARS));
        assert!(!stable_prefix(&jumped).starts_with(first));
    }

    #[test]
    fn the_window_does_not_start_inside_a_word() {
        let earlier = format!("{} midword", "a".repeat(WINDOW_CHARS + 10));
        assert_eq!(stable_prefix(&earlier), "midword");
    }
}
