//! Exact word reuse from one explicit, bounded snapshot. No model or dictionary.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::time::Instant;

use serde::{Deserialize, Serialize};
use unicode_segmentation::UnicodeSegmentation;

use super::attestation::{lexical_separator, word_scalars};
use crate::writing::{self, WritingLanguage};

pub const REQUEST_SCHEMA: &str = "badi.context-lookup.request.v1";
pub const ERROR_SCHEMA: &str = "badi.context-lookup.error.v1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub id: String,
    pub before: String,
    pub language: String,
    pub context: String,
    pub style_examples: Vec<String>,
}

impl Request {
    fn validate(&self) -> Result<WritingLanguage, &'static str> {
        if self.schema != REQUEST_SCHEMA
            || self.id.is_empty()
            || self.id.len() > 128
            || self.id.chars().any(char::is_control)
            || self.before.trim().is_empty()
            || !super::valid_text(&self.before, 2048)
            || !super::valid_text(&self.context, 4096)
            || self.style_examples.len() > 8
            || self
                .style_examples
                .iter()
                .any(|text| text.trim().is_empty() || !super::valid_text(text, 1024))
            || !(2..=35).contains(&self.language.len())
            || !self
                .language
                .split('-')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err("invalid_request");
        }
        WritingLanguage::from_tag(&self.language).ok_or("invalid_request")
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Context,
    Style,
    Draft,
}

impl Source {
    const fn bit(self) -> u8 {
        match self {
            Self::Context => 1,
            Self::Style => 2,
            Self::Draft => 4,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ResultRecord {
    pub schema: &'static str,
    pub id: String,
    pub contract_id: &'static str,
    pub outcome: &'static str,
    pub reason: &'static str,
    pub text: Option<String>,
    pub matched_word: Option<String>,
    pub matched_candidate_count: usize,
    pub sources: Vec<Source>,
    pub latency_ms: f64,
    pub warnings: [&'static str; 1],
}

fn stem(before: &str, language: WritingLanguage) -> Option<(usize, &str)> {
    let (start, word) = before.unicode_word_indices().next_back()?;
    (start != 0
        && start + word.len() == before.len()
        && (3..=24).contains(&word.chars().count())
        && before[..start]
            .chars()
            .next_back()
            .is_some_and(lexical_separator)
        && word.chars().next_back().is_some_and(char::is_alphabetic)
        && word_scalars(word)
        && language.accepts_output(word))
    .then_some((start, word))
}

fn collect<'a>(
    candidates: &mut BTreeMap<&'a str, u8>,
    text: &'a str,
    stem: &str,
    language: WritingLanguage,
    source: Source,
) {
    for (start, word) in text.unicode_word_indices() {
        let end = start + word.len();
        if word.len() <= stem.len()
            || !word.starts_with(stem)
            || !word_scalars(word)
            || !language.accepts_output(word)
            || (start == 0 && source == Source::Draft)
            || (start > 0
                && !text[..start]
                    .chars()
                    .next_back()
                    .is_some_and(lexical_separator))
            || !text[end..].chars().next().is_some_and(lexical_separator)
        {
            continue;
        }
        *candidates.entry(word).or_default() |= source.bit();
    }
}

pub fn lookup(request: &Request) -> Result<ResultRecord, &'static str> {
    let started = Instant::now();
    let language = request.validate()?;
    let mut result = ResultRecord {
        schema: "badi.context-lookup.result.v1",
        id: request.id.clone(),
        contract_id: "badi.context-lookup.exact.v1",
        outcome: "abstention",
        reason: "no_eligible_stem",
        text: None,
        matched_word: None,
        matched_candidate_count: 0,
        sources: Vec::new(),
        latency_ms: 0.0,
        warnings: ["context_match_does_not_prove_intent"],
    };
    if let Some((start, stem)) = stem(&request.before, language) {
        let mut candidates = BTreeMap::new();
        collect(
            &mut candidates,
            &request.context,
            stem,
            language,
            Source::Context,
        );
        for example in &request.style_examples {
            collect(&mut candidates, example, stem, language, Source::Style);
        }
        // The current stem is never evidence, and a clipped leading fragment
        // of the authorized draft never gains an invented left boundary.
        collect(
            &mut candidates,
            &request.before[..start],
            stem,
            language,
            Source::Draft,
        );
        result.matched_candidate_count = candidates.len();
        result.reason = match candidates.len() {
            0 => "no_matching_word",
            1 => "unsupported_suffix",
            _ => "ambiguous_matches",
        };
        if candidates.len() == 1 {
            let (word, sources) = candidates.first_key_value().expect("one candidate");
            let suffix = &word[stem.len()..];
            // Count all competing whole words first. An unsafe display seam
            // must not turn two plausible source words into a unique choice.
            if suffix.chars().next().is_some_and(char::is_alphabetic)
                && language.accepts_output(suffix)
                && crate::segment::sanitize_suggestion(suffix).is_ok()
                && writing::validate_suggestion_shape(&request.before, "", suffix).is_ok()
            {
                result.outcome = "suggestion";
                result.reason = "unique_source_word";
                result.text = Some(suffix.to_owned());
                result.matched_word = Some((*word).to_owned());
                result.sources = [Source::Context, Source::Style, Source::Draft]
                    .into_iter()
                    .filter(|source| sources & source.bit() != 0)
                    .collect();
            }
        }
    }
    result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(result)
}

fn read_request(input: &mut impl BufRead) -> Result<Request, &'static str> {
    let mut bytes = Vec::new();
    input
        .by_ref()
        .take((super::MAX_FRAME_BYTES + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "invalid_frame")?;
    if bytes.len() > super::MAX_FRAME_BYTES || bytes.last() != Some(&b'\n') {
        return Err("invalid_frame");
    }
    let mut extra = [0_u8; 1];
    if input.read(&mut extra).map_err(|_| "invalid_frame")? != 0 {
        return Err("invalid_frame");
    }
    serde_json::from_slice(&bytes).map_err(|_| "invalid_request")
}

/// Called synchronously before Tokio is constructed. EOF closes the one-shot
/// input snapshot; no runtime or other child process is ever needed.
pub fn run_stdio(args: &[OsString]) -> Result<(), &'static str> {
    if !args.is_empty() {
        return Err("invalid_arguments");
    }
    let request = read_request(&mut std::io::stdin().lock())?;
    let result = lookup(&request)?;
    let mut output = std::io::stdout().lock();
    serde_json::to_writer(&mut output, &result).map_err(|_| "output_closed")?;
    output
        .write_all(b"\n")
        .and_then(|()| output.flush())
        .map_err(|_| "output_closed")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(before: &str, language: &str, context: &str) -> Request {
        Request {
            schema: REQUEST_SCHEMA.to_owned(),
            id: "fixture".to_owned(),
            before: before.to_owned(),
            language: language.to_owned(),
            context: context.to_owned(),
            style_examples: Vec::new(),
        }
    }
    #[test]
    fn exact_multilingual_lookup_returns_only_the_untyped_suffix() {
        for (before, language, context, suffix, word) in [
            (
                "Please use the sched",
                "en",
                "The schedule is ready.",
                "ule",
                "schedule",
            ),
            (
                "Die Tagesord",
                "de-DE",
                "Tagesordnung liegt vor.",
                "nung",
                "Tagesordnung",
            ),
            (
                "Die Tagesordnung",
                "de",
                "Tagesordnungsentwurf liegt vor.",
                "sentwurf",
                "Tagesordnungsentwurf",
            ),
            ("من هر روز می‌ر", "FA-ir", "من هر روز می‌روم.", "وم", "می‌روم"),
        ] {
            let result = lookup(&request(before, language, context)).unwrap();
            assert_eq!(result.outcome, "suggestion");
            assert_eq!(result.text.as_deref(), Some(suffix));
            assert_eq!(result.matched_word.as_deref(), Some(word));
            assert_eq!(result.sources, vec![Source::Context]);
            assert_eq!(result.matched_candidate_count, 1);
        }
    }
    #[test]
    fn repeated_words_dedupe_and_retain_original_field_provenance() {
        let mut value = request(
            "Die Tagesordnung und Tagesord",
            "de",
            "Tagesordnung. Tagesordnung.",
        );
        value
            .style_examples
            .push("Tagesordnung liegt vor.".to_owned());
        let result = lookup(&value).unwrap();
        assert_eq!(result.matched_candidate_count, 1);
        assert_eq!(
            result.sources,
            vec![Source::Context, Source::Style, Source::Draft]
        );
        value.context.clear();
        value.style_examples.clear();
        assert_eq!(lookup(&value).unwrap().sources, vec![Source::Draft]);
    }
    #[test]
    fn unknown_draft_start_and_unclosed_or_connected_source_words_are_not_evidence() {
        assert_eq!(
            lookup(&request("Tagesord", "de", "Tagesordnung."))
                .unwrap()
                .reason,
            "no_eligible_stem"
        );
        assert_eq!(
            lookup(&request("Tagesordnung und Tagesord", "de", ""))
                .unwrap()
                .reason,
            "no_matching_word"
        );
        for context in [
            "Tagesordnung",
            "XTagesordnung.",
            "Tagesordnung-Entwurf.",
            "Tagesordnung's",
            "Tagesordnung/Entwurf.",
            "Tagesordnung_Entwurf.",
            "Tagesordnung1.",
            "1Tagesordnung.",
        ] {
            let result = lookup(&request("Die Tagesord", "de", context)).unwrap();
            assert!(result.text.is_none(), "{context}");
        }
        let mut value = request("Die Tagesord", "de", "Tagesordnung");
        value.style_examples = vec![".".to_owned(), "Tagesordnung".to_owned(), ".".to_owned()];
        assert_eq!(lookup(&value).unwrap().matched_candidate_count, 0);
    }
    #[test]
    fn ambiguity_precedes_unsafe_suffix_filtering() {
        for (before, language, context) in [
            ("Die Fran", "de", "Frankfurt. Frankreich."),
            ("Das Caf", "de", "Caf\u{301}e. Cafe."),
            ("این میخ", "fa", "میخ‌ها. میخک."),
        ] {
            let result = lookup(&request(before, language, context)).unwrap();
            assert_eq!(result.reason, "ambiguous_matches");
            assert_eq!(result.matched_candidate_count, 2);
            assert!(
                result.text.is_none() && result.matched_word.is_none() && result.sources.is_empty()
            );
        }
        assert_eq!(
            lookup(&request("این میخ", "fa", "میخ‌ها.")).unwrap().reason,
            "unsupported_suffix"
        );
        let long = lookup(&request(
            "The abc",
            "en",
            &format!("abc{}.", "d".repeat(65)),
        ))
        .unwrap();
        assert_eq!(
            (long.reason, long.matched_candidate_count),
            ("unsupported_suffix", 1)
        );
    }
    #[test]
    fn case_diacritics_and_joiners_are_never_normalized() {
        for (before, language, context) in [
            ("Die Tagesord", "de", "tagesordnung."),
            ("Die Übertr", "de", "U\u{308}berträge."),
            ("برای کیا", "fa", "كيان آماده است."),
            ("من می‌ر", "fa", "من میروم."),
        ] {
            assert_eq!(
                lookup(&request(before, language, context))
                    .unwrap()
                    .matched_candidate_count,
                0
            );
        }
        for before in [
            "Die Ta",
            "Die Tagesord ",
            "Die Tagesord-",
            "Die/Tagesord",
            "Die Tagesord\n",
        ] {
            assert_eq!(
                lookup(&request(before, "de", "Tagesordnung."))
                    .unwrap()
                    .reason,
                "no_eligible_stem"
            );
        }
        assert_eq!(
            lookup(&request(
                &format!("Die {}", "T".repeat(25)),
                "de",
                "Tagesordnung."
            ))
            .unwrap()
            .reason,
            "no_eligible_stem"
        );
    }
    #[test]
    fn frames_and_schema_reject_expected_data_extra_input_and_excessive_text() {
        let valid = serde_json::json!({"schema":REQUEST_SCHEMA,"id":"fixture","before":"Die Tagesord","language":"de","context":"Tagesordnung.","style_examples":[]});
        let frame = format!("{valid}\n");
        read_request(&mut std::io::Cursor::new(frame.as_bytes())).unwrap();
        for bytes in [
            Vec::new(),
            valid.to_string().into_bytes(),
            format!("{frame}\n").into_bytes(),
            vec![b'x'; super::super::MAX_FRAME_BYTES + 1],
            b"\xff\n".to_vec(),
        ] {
            assert!(read_request(&mut std::io::Cursor::new(bytes)).is_err());
        }
        for field in ["expected", "matched_word", "after", "config"] {
            let mut changed = valid.clone();
            changed[field] = serde_json::json!("private");
            assert!(serde_json::from_value::<Request>(changed).is_err());
        }
        for (field, replacement) in [
            ("before", serde_json::json!("x".repeat(2049))),
            ("context", serde_json::json!("x".repeat(4097))),
            ("style_examples", serde_json::json!(vec!["example"; 9])),
            ("style_examples", serde_json::json!(["x".repeat(1025)])),
            ("before", serde_json::json!("این می‌")),
            ("language", serde_json::json!("fr")),
        ] {
            let mut changed = valid.clone();
            changed[field] = replacement;
            assert!(lookup(&serde_json::from_value(changed).unwrap()).is_err());
        }
    }
}
