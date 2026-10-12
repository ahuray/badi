//! Lab-only mid-sentence comparisons. Nothing here is a production gate.
//! The logprob floor is uncalibrated and must not be copied into the broker.

use badi_broker::writing::{self, mid_line_insertion};

use crate::transport::TokenLogprob;

/// Constant frame for the lab `framed` mode, version
/// `everyday-english-2026-10-12.v1`. The two examples avoid names. A test
/// checks them against the development suites. The opening sentence is the
/// measured 2026-10-12 prefix; it overlaps the mid-sentence suite on "notes".
pub(crate) const FRAME_VERSION: &str = "everyday-english-2026-10-12.v1";
pub(crate) const FRAME: &str = "Notes, messages and emails typed by one person, in plain everyday English.\n\nThe kettle is boiling. A cup sits on the counter.\n\nA grey notebook rests beside the lamp.\n\n";

pub(crate) fn framed_warning() -> &'static str {
    match FRAME_VERSION {
        "everyday-english-2026-10-12.v1" => "framed_prompt_is_lab_only",
        _ => "framed_prompt_version_unknown",
    }
}
/// Uncalibrated lab cut for the forced two-word check. Not a production threshold.
pub(crate) const LOGPROB_FLOOR: f64 = -2.0;

/// The first one or two words after the caret, with edge punctuation removed.
/// `None` when the line has no word, such as a lone period.
pub(crate) fn following_words(after: &str) -> Option<Vec<String>> {
    let line = after.split(['\n', '\r']).next().unwrap_or_default();
    let words: Vec<String> = line
        .split_whitespace()
        .filter_map(plain_word)
        .take(2)
        .collect();
    (!words.is_empty()).then_some(words)
}

fn plain_word(token: &str) -> Option<String> {
    let word = token.trim_matches(|character: char| !character.is_ascii_alphanumeric());
    if word.is_empty()
        || !word.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '\'' || character == '-'
        })
    {
        return None;
    }
    Some(word.to_owned())
}

/// Grammar that lets the model write a continuation and then the following words.
pub(crate) fn forced_word_grammar(words: &[String]) -> Option<String> {
    if words.is_empty() || words.len() > 2 {
        return None;
    }
    let literal = serde_json::to_string(&format!(" {}", words.join(" "))).ok()?;
    Some(format!("root ::= [^<>\\n\\r`]* {literal}"))
}

/// Text generated before the forced following words.
pub(crate) fn prefix_before_forced<'a>(raw: &'a str, words: &[String]) -> Option<&'a str> {
    let marker = format!(" {}", words.join(" "));
    raw.strip_suffix(&marker)
}

/// Mean logprob of the tokens that make up `suffix`, when they align on bytes.
pub(crate) fn suffix_mean_logprob(raw: &str, suffix: &str, tokens: &[TokenLogprob]) -> Option<f64> {
    if suffix.is_empty() || !raw.ends_with(suffix) {
        return None;
    }
    let start = raw.len() - suffix.len();
    let mut offset = 0;
    let mut total = 0.0;
    let mut count = 0_usize;
    for token in tokens {
        let next = offset + token.bytes.len();
        if next > start {
            total += token.logprob;
            count += 1;
        }
        offset = next;
    }
    let count = u32::try_from(count).ok().filter(|count| *count > 0)?;
    (offset == raw.len()).then_some(total / f64::from(count))
}

/// Keep the free prefix only when the forced words are at least as likely as
/// the uncalibrated floor. An empty prefix inserts nothing.
pub(crate) fn accept_forced_prefix(prefix: &str, mean_logprob: f64) -> Option<String> {
    if !mean_logprob.is_finite() || mean_logprob < LOGPROB_FLOOR || prefix.is_empty() {
        return None;
    }
    Some(prefix.to_owned())
}

/// Lab-only prefix that names the line after the caret. The continuation is
/// still generated from the text before the caret. Production prompts omit this.
pub(crate) fn following_text_prefix(after: &str) -> String {
    let line = after.split(['\n', '\r']).next().unwrap_or_default().trim();
    if line.is_empty() {
        return String::new();
    }
    let line: String = line.chars().take(120).collect();
    format!("Words after the caret: {line}\n\n")
}

/// Today's rejoin, applied to a lab continuation. Production is unchanged.
pub(crate) fn rejoin(before: &str, after: &str, continuation: &str) -> Option<String> {
    let line = writing::line_rest(after)?;
    mid_line_insertion(before, line, continuation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn following_words_stop_at_two_and_ignore_a_bare_period() {
        assert_eq!(
            following_words("report today and more"),
            Some(vec!["report".to_owned(), "today".to_owned()])
        );
        assert_eq!(
            following_words("slides before the meeting?"),
            Some(vec!["slides".to_owned(), "before".to_owned()])
        );
        assert_eq!(following_words("."), None);
        assert_eq!(following_words("\nreport"), None);
    }

    #[test]
    fn following_text_names_one_line_after_the_caret() {
        assert_eq!(
            following_text_prefix("report today\nand more"),
            "Words after the caret: report today\n\n"
        );
        assert_eq!(following_text_prefix("  \nreport"), "");
        assert_eq!(following_text_prefix(""), "");
        let long = "a".repeat(200);
        let prefix = following_text_prefix(&long);
        let line = prefix
            .strip_prefix("Words after the caret: ")
            .unwrap()
            .trim();
        assert_eq!(line.chars().count(), 120);
    }

    #[test]
    fn the_grammar_forces_the_following_words_as_a_literal() {
        let words = following_words("report today").expect("words");
        let grammar = forced_word_grammar(&words).expect("grammar");
        assert!(grammar.contains("\" report today\""));
        assert!(grammar.starts_with("root ::= "));
    }

    #[test]
    fn an_unlikely_forced_suffix_inserts_nothing() {
        let raw = "final report today";
        let words = following_words("report today").expect("words");
        let prefix = prefix_before_forced(raw, &words).expect("prefix");
        assert_eq!(prefix, "final");
        let tokens = vec![
            token(b"final", -0.2),
            token(b" report", -0.4),
            token(b" today", -4.0),
        ];
        let mean = suffix_mean_logprob(raw, " report today", &tokens).expect("mean");
        assert!(mean < LOGPROB_FLOOR);
        assert_eq!(accept_forced_prefix(prefix, mean), None);
        assert_eq!(
            rejoin("Please find the ", "report today", raw).as_deref(),
            Some("final ")
        );
    }

    #[test]
    fn a_likely_forced_suffix_keeps_the_prefix() {
        let tokens = vec![token(b"final", -0.2), token(b" report today", -0.5)];
        let mean =
            suffix_mean_logprob("final report today", " report today", &tokens).expect("mean");
        assert!(mean >= LOGPROB_FLOOR);
        assert_eq!(
            accept_forced_prefix("final", mean).as_deref(),
            Some("final")
        );
    }

    #[test]
    fn the_frame_has_no_personal_names() {
        assert_eq!(FRAME_VERSION, "everyday-english-2026-10-12.v1");
        assert!(FRAME.contains("plain everyday English"));
        assert!(!FRAME.to_ascii_lowercase().contains("maria"));
        assert!(FRAME.ends_with("\n\n"));
    }

    #[test]
    fn frame_examples_do_not_reuse_development_suite_content_words() {
        let examples = FRAME.split("\n\n").skip(1).collect::<Vec<_>>().join(" ");
        let example_words = content_words(&examples);
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../lab/examples");
        let mut suite_words = std::collections::BTreeSet::new();
        for name in [
            "quality-probes.json",
            "real-text-development.json",
            "typing-traces.json",
            "mid-line-development.json",
        ] {
            let text = std::fs::read_to_string(root.join(name)).expect(name);
            suite_words.extend(content_words(&text));
        }
        let overlap: Vec<_> = example_words.intersection(&suite_words).collect();
        assert!(overlap.is_empty(), "frame examples overlap {overlap:?}");
    }

    fn content_words(text: &str) -> std::collections::BTreeSet<String> {
        text.split(|character: char| !character.is_alphabetic())
            .map(str::to_lowercase)
            .filter(|word| word.chars().count() >= 4)
            .filter(|word| {
                !matches!(
                    word.as_str(),
                    "that" | "this" | "with" | "from" | "have" | "been" | "were" | "they"
                )
            })
            .collect()
    }

    fn token(bytes: &[u8], logprob: f64) -> TokenLogprob {
        TokenLogprob {
            bytes: bytes.to_vec(),
            logprob,
        }
    }
}
