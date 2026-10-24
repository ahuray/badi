//! The Lab's word completion: the broker's vocabulary rule
//! (`badi_broker::vocabulary`, ADR 0010) on one explicit, bounded snapshot,
//! with the case's context as window words and its style examples as learned
//! words. No model or dictionary.

use std::ffi::OsString;
use std::io::{BufRead, Read, Write};
use std::time::Instant;

use serde::{Deserialize, Serialize};

use badi_broker::vocabulary::{self, Vocabulary};
use badi_broker::writing::WritingLanguage;

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

pub const RESULT_SCHEMA: &str = "badi.context-lookup.result.v2";
pub const CONTRACT_ID: &str = "badi.vocabulary.v1";

/// Where the finished word was found, in the Lab's names for the broker's
/// window, learned and draft sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Context,
    Style,
    Draft,
}

impl From<vocabulary::Source> for Source {
    fn from(source: vocabulary::Source) -> Self {
        match source {
            vocabulary::Source::Window => Self::Context,
            vocabulary::Source::Learned => Self::Style,
            vocabulary::Source::Draft => Self::Draft,
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
    pub sources: Vec<Source>,
    pub latency_ms: f64,
    pub warnings: [&'static str; 1],
}

pub fn lookup(request: &Request) -> Result<ResultRecord, &'static str> {
    let started = Instant::now();
    let language = request.validate()?;
    let words = Vocabulary::from_texts(&request.context, &request.style_examples);
    let mut result = ResultRecord {
        schema: RESULT_SCHEMA,
        id: request.id.clone(),
        contract_id: CONTRACT_ID,
        outcome: "abstention",
        reason: "no_stem",
        text: None,
        matched_word: None,
        sources: Vec::new(),
        latency_ms: 0.0,
        warnings: ["context_match_does_not_prove_intent"],
    };
    match vocabulary::complete(&request.before, "", language, &words) {
        Ok(completion) => {
            result.outcome = "suggestion";
            result.reason = "vocabulary_word";
            result.text = Some(completion.suffix);
            result.matched_word = Some(completion.word);
            result.sources = completion.sources.into_iter().map(Source::from).collect();
        }
        Err(abstention) => result.reason = abstention.as_str(),
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
    fn the_brokers_rule_finishes_a_word_from_each_source() {
        let window = lookup(&request(
            "Please use the sched",
            "en",
            "The schedule is ready.",
        ))
        .unwrap();
        assert_eq!(
            (
                window.outcome,
                window.reason,
                window.text.as_deref(),
                window.matched_word.as_deref()
            ),
            (
                "suggestion",
                "vocabulary_word",
                Some("ule"),
                Some("schedule")
            )
        );
        assert_eq!(window.sources, vec![Source::Context]);
        let mut learned = request("Hi Okonk", "en", "");
        learned.style_examples.push("Okonkwo agreed.".to_owned());
        assert_eq!(lookup(&learned).unwrap().sources, vec![Source::Style]);
        let draft = lookup(&request("Die Tagesordnung und Tagesord", "de", "")).unwrap();
        assert_eq!(
            (draft.text.as_deref(), draft.sources),
            (Some("nung"), vec![Source::Draft])
        );
        let persian = lookup(&request("من هر روز می‌رو", "fa", "من هر روز می‌روم.")).unwrap();
        assert_eq!(persian.text.as_deref(), Some("م"));
    }
    #[test]
    fn abstentions_carry_the_brokers_reason() {
        for (before, context, reason) in [
            ("Please use the sch", "The schedule is ready.", "no_stem"),
            ("I told them about", "aboutness", "known_word"),
            ("Please use the sched", "Nothing fits.", "no_match"),
            ("Hi Marin", "Marinella and Marinette", "ambiguous"),
        ] {
            let result = lookup(&request(before, "en", context)).unwrap();
            assert_eq!(
                (result.outcome, result.reason),
                ("abstention", reason),
                "{before}"
            );
            assert!(
                result.text.is_none() && result.matched_word.is_none() && result.sources.is_empty()
            );
        }
        assert_eq!(
            lookup(&request("این دفترچه", "fa", "دفترچه‌ها."))
                .unwrap()
                .reason,
            "unsafe_suffix"
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
            ("language", serde_json::json!("it")),
        ] {
            let mut changed = valid.clone();
            changed[field] = replacement;
            assert!(lookup(&serde_json::from_value(changed).unwrap()).is_err());
        }
    }
}
