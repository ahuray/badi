//! Hardware-selected local writing inference. Artifact advice is never treated
//! as a release qualification; startup verifies the bytes actually executed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde_json::{Value, json};
use thiserror::Error;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use crate::model_selection::{
    HardwareProfile, ModelArtifact, ModelUseCase, catalog, detect_hardware, recommend_model,
    select_installed_writing_model,
};
use crate::provider::ProviderRequest;
use crate::semantic::candidate::{
    RUNTIME_ARCHIVE_BYTES, RUNTIME_ARCHIVE_FILENAME, RUNTIME_ARCHIVE_SHA256,
    RUNTIME_BUNDLE_MANIFEST_SHA256, RUNTIME_BYTES, RUNTIME_SHA256,
};
use crate::semantic::provenance::{
    DirectoryManifestExpectation, FileExpectation, ProvenanceError, VerifiedFile,
    verify_directory_manifest, verify_file,
};
use crate::semantic::runtime::{LlamaCppLaunch, OwnedRuntime, RuntimeError};

pub const WRITING_CONTRACT: &str = "badi.writing.completion-and-spelling.en-de-fa.v2";
pub(crate) const STREAM_BUDGET_MS: u64 = 550;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WritingLanguage {
    English,
    German,
    Persian,
}

impl WritingLanguage {
    pub(crate) fn from_tag(tag: &str) -> Option<Self> {
        match tag.split('-').next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            "fa" => Some(Self::Persian),
            _ => None,
        }
    }

    pub(crate) fn accepts_output(self, value: &str) -> bool {
        if self != Self::Persian {
            return crate::semantic::client::valid_english_output(value);
        }
        if !crate::segment::valid_orthographic_joiners(value) {
            return false;
        }
        let mut saw_letter = false;
        let mut arabic_base = false;
        for character in value.chars() {
            if character.script() == Script::Arabic && character.is_alphabetic() {
                saw_letter = true;
                arabic_base = true;
            } else if arabic_base && matches!(character, '\u{064b}'..='\u{065f}' | '\u{0670}') {
                // Arabic combining vowel marks require an actual preceding base.
            } else if character == '\u{200c}'
                || crate::semantic::client::allowed_common_scalar(character)
                || matches!(
                    character,
                    '\u{060c}' | '\u{061b}' | '\u{061f}' | '\u{06f0}'..='\u{06f9}'
                )
            {
                arabic_base = false;
            } else {
                return false;
            }
        }
        saw_letter
    }
}

pub(crate) fn validate_suggestion_shape(
    before: &str,
    after: &str,
    suggestion: &str,
) -> Result<(), crate::segment::OutputError> {
    // Native prefix inference may continue the word at the caret. Permit only
    // same-script letters at that left boundary; all spacing, overlap and right
    // boundary checks still apply. No document mutation authority changes.
    let suffix = after.is_empty()
        && before
            .chars()
            .next_back()
            .zip(suggestion.chars().next())
            .is_some_and(|(left, right)| {
                left.is_alphabetic()
                    && right.is_alphabetic()
                    && left.script() == right.script()
                    && matches!(left.script(), Script::Latin | Script::Arabic)
            });
    crate::segment::validate_completion_shape(before, after, suggestion, suffix)
}

pub(crate) fn validate_proposal(
    before: &str,
    after: &str,
    suggestion: &str,
    language: Option<&str>,
) -> Result<(), crate::segment::OutputError> {
    validate_suggestion_shape(before, after, suggestion)?;
    if before.chars().next_back().is_some_and(char::is_alphabetic)
        && suggestion.chars().next().is_some_and(char::is_alphabetic)
    {
        if language.and_then(WritingLanguage::from_tag) != Some(WritingLanguage::English) {
            return Err(crate::segment::OutputError::InvalidShape);
        }
        let stem = before.unicode_words().next_back().unwrap_or_default();
        let suffix = suggestion.unicode_words().next().unwrap_or_default();
        if !known_english_word(&format!("{stem}{suffix}")) {
            return Err(crate::segment::OutputError::InvalidShape);
        }
    }
    Ok(())
}

fn english_words() -> &'static [&'static str] {
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    WORDS.get_or_init(|| {
        include_str!("../data/writing-lexicon/en.txt")
            .lines()
            .collect()
    })
}

fn known_english_word(word: &str) -> bool {
    word.bytes().all(|byte| byte.is_ascii_alphabetic())
        && english_words()
            .binary_search(&word.to_ascii_lowercase().as_str())
            .is_ok()
}

fn english_word_prefix(word: &str) -> bool {
    let words = english_words();
    let index = words.partition_point(|candidate| *candidate < word);
    words
        .get(index)
        .is_some_and(|candidate| candidate.starts_with(word))
}

pub fn data_directory() -> Result<PathBuf, WritingError> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if !path.is_absolute() {
            return Err(WritingError::DataDirectory);
        }
        return Ok(path.join("badi"));
    }
    let home = std::env::var_os("HOME").ok_or(WritingError::DataDirectory)?;
    let home = PathBuf::from(home);
    if !home.is_absolute() {
        return Err(WritingError::DataDirectory);
    }
    Ok(home.join(".local/share/badi"))
}

pub async fn activate(directory: PathBuf) -> Result<(OwnedRuntime, ModelArtifact), WritingError> {
    let (launch, model) = prepare_launch(directory).await?;
    Ok((launch.for_writing().spawn().await?, model))
}

pub(crate) async fn prepare_launch(
    directory: PathBuf,
) -> Result<(LlamaCppLaunch, ModelArtifact), WritingError> {
    let hardware = detect_hardware();
    let threads = writing_threads(&hardware)?;
    let installed: Vec<_> = catalog(ModelUseCase::Writing)
        .iter()
        .filter(|model| directory.join("models").join(model.filename).is_file())
        .map(|model| model.filename)
        .collect();
    let model = select_installed_writing_model(&hardware, &installed)
        .map_err(|_| WritingError::NoFit)?
        .ok_or_else(|| {
            recommend_model(hardware, ModelUseCase::Writing)
                .recommended
                .map_or(WritingError::NoFit, |model| {
                    WritingError::NotInstalled(model.filename)
                })
        })?;
    let launch = tokio::task::spawn_blocking(move || verified_launch(&directory, model, threads))
        .await
        .map_err(|_| WritingError::VerificationTask)??;
    Ok((launch, model))
}

fn writing_threads(hardware: &HardwareProfile) -> Result<usize, WritingError> {
    if std::env::consts::OS != "linux" || hardware.architecture != "x86_64" || !hardware.cpu.avx2 {
        return Err(WritingError::UnsupportedRuntime);
    }
    // Small quantized models are bandwidth-bound on the tested hybrid CPU.
    // Leave capacity for the editor; twelve threads had worse tail latency.
    Ok((hardware.logical_cpus / 2).clamp(1, 4))
}

#[cfg(feature = "writing-lab")]
pub(crate) async fn prepare_lab_artifact(
    directory: PathBuf,
    artifact: crate::writing_lab::artifact::ModelArtifactOverride,
) -> Result<LlamaCppLaunch, WritingError> {
    let threads = writing_threads(&detect_hardware())?;
    tokio::task::spawn_blocking(move || {
        let weights = verify_file(&artifact.weights)?;
        verified_runtime_launch(&directory, weights, &artifact.alias, threads)
    })
    .await
    .map_err(|_| WritingError::VerificationTask)?
}

fn verified_launch(
    directory: &Path,
    model: ModelArtifact,
    threads: usize,
) -> Result<LlamaCppLaunch, WritingError> {
    let runtime = directory.join("runtimes/llama-b10726");
    let weights = directory.join("models").join(model.filename);
    if !weights.is_file() || !runtime.join("llama-server").is_file() {
        return Err(WritingError::NotInstalled(model.filename));
    }
    let weights = verify_file(&FileExpectation::new(
        weights,
        model.sha256,
        model.download_bytes,
    )?)?;
    verified_runtime_launch(directory, weights, model.filename, threads)
}

fn verified_runtime_launch(
    directory: &Path,
    weights: VerifiedFile,
    alias: &str,
    threads: usize,
) -> Result<LlamaCppLaunch, WritingError> {
    let runtime = directory.join("runtimes/llama-b10726");
    let bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
        &runtime,
        RUNTIME_BUNDLE_MANIFEST_SHA256,
    )?)?;
    verify_file(&FileExpectation::new(
        directory
            .join("runtimes/downloads")
            .join(RUNTIME_ARCHIVE_FILENAME),
        RUNTIME_ARCHIVE_SHA256,
        RUNTIME_ARCHIVE_BYTES,
    )?)?;
    let binary = verify_file(&FileExpectation::new(
        runtime.join("llama-server"),
        RUNTIME_SHA256,
        RUNTIME_BYTES,
    )?)?;
    Ok(LlamaCppLaunch::new(
        binary, bundle, weights, alias, threads,
    )?)
}

pub(crate) struct CompletionPlan<'a> {
    pub(crate) prompt: &'a str,
    pub(crate) echo: Option<&'a str>,
    pub(crate) max_tokens: u16,
}

pub(crate) fn completion_plan(
    request: &ProviderRequest,
    heal_trailing_ascii_space: bool,
) -> CompletionPlan<'_> {
    let context = inference_context(&request.before);
    let language = request
        .language
        .as_deref()
        .and_then(WritingLanguage::from_tag);
    if heal_trailing_ascii_space && language.is_some() && context.ends_with(' ') {
        let prompt = context.trim_end_matches(' ');
        return CompletionPlan {
            prompt,
            echo: Some(&context[prompt.len()..]),
            // Boundary echo is the only experiment. Do not silently expand
            // the ordinary eight-token allowance because an echo now exists.
            max_tokens: 8,
        };
    }
    let echo = (language == Some(WritingLanguage::English))
        .then(|| healing_prefix(context))
        .flatten();
    CompletionPlan {
        prompt: echo.map_or(context, |word| &context[..context.len() - word.len()]),
        echo,
        max_tokens: if echo.is_some() { 12 } else { 8 },
    }
}

impl CompletionPlan<'_> {
    pub(crate) fn payload(&self) -> Value {
        let mut payload = json!({"prompt":self.prompt,"n_predict":self.max_tokens,
            "temperature":0.0,"seed":42,"stop":[".","\n"],"stream":true,"cache_prompt":true});
        if let Some(echo) = self.echo {
            // Generate at the earlier boundary, reproduce the exact removed
            // bytes, then let the streaming reader remove only that echo.
            let literal = serde_json::to_string(echo).expect("echo is serializable");
            payload["grammar"] = Value::String(format!("root ::= {literal} [^<>\\n\\r`]*"));
        }
        payload
    }
}

#[cfg(test)]
pub(crate) fn completion_payload(request: &ProviderRequest) -> Value {
    completion_plan(request, false).payload()
}

/// Limit cold prefill cost without changing the context used for edit authority.
pub(crate) fn inference_context(before: &str) -> &str {
    let start = before
        .char_indices()
        .rev()
        .nth(159)
        .map_or(0, |(index, _)| index);
    let mut window = &before[start..];
    if start > 0 && !before[..start].ends_with(char::is_whitespace) {
        if let Some((index, whitespace)) = window
            .char_indices()
            .find(|(_, character)| character.is_whitespace())
        {
            window = &window[index + whitespace.len_utf8()..];
        }
    }
    // Cold prefill of the full window can exhaust the writing deadline before
    // the first token. Keep the current sentence; edit authority still binds
    // the unchanged full request.
    for (index, character) in window.char_indices().rev() {
        if matches!(character, '.' | '!' | '?' | '\u{061f}' | '\n') {
            let after = &window[index + character.len_utf8()..];
            if (character == '\n' || after.starts_with(char::is_whitespace))
                && !after.trim_start().is_empty()
            {
                return after.trim_start();
            }
        }
    }
    window
}

pub(crate) fn healing_prefix(before: &str) -> Option<&str> {
    let word = before.split_whitespace().next_back()?;
    (before.ends_with(word)
        && (2..=24).contains(&word.chars().count())
        && !known_english_word(word)
        && word
            .chars()
            .all(|character| character.is_alphabetic() || character == '\u{200c}')
        && crate::segment::valid_orthographic_joiners(word))
    .then_some(word)
}

pub(crate) fn strip_healed_prefix<'a>(raw: &'a str, echo: Option<&str>) -> Option<&'a str> {
    match echo {
        Some(echo) => raw.strip_prefix(echo),
        None => Some(raw),
    }
}

pub(crate) fn available_complete_words(raw: &str) -> Option<String> {
    let separator = raw.rfind(' ').unwrap_or(0);
    let count = raw
        .unicode_word_indices()
        .filter(|(start, word)| start + word.len() <= separator)
        .count()
        .min(4);
    (count > 0)
        .then(|| complete_word_prefix(raw, count, false))
        .flatten()
}

pub(crate) fn correction_word(before: &str) -> Option<&str> {
    let word = before.split_whitespace().next_back()?;
    // A paused partial word is not a spelling error. Preserve it for prefix
    // completion instead of spending the writing budget trying to replace it.
    if known_english_word(word) || english_word_prefix(word) {
        return None;
    }
    ((3..=24).contains(&word.len())
        && word.bytes().all(|byte| byte.is_ascii_lowercase())
        && before.ends_with(word))
    .then_some(word)
}

pub(crate) struct CorrectionTarget<'a> {
    pub(crate) word: &'a str,
    pub(crate) suffix: &'a str,
    pub(crate) delimiter: &'a str,
}

pub(crate) fn correction_target(before: &str) -> Option<CorrectionTarget<'_>> {
    let (word_context, delimiter) = before
        .strip_suffix(' ')
        .map_or((before, ""), |context| (context, " "));
    let word = correction_word(word_context)?;
    Some(CorrectionTarget {
        word,
        suffix: &before[word_context.len() - word.len()..],
        delimiter,
    })
}

/// Avoid model inference when the bounded dictionary edit neighborhood has
/// exactly one candidate. Ambiguity remains a model decision; dictionary order
/// is not a frequency signal. This path still only proposes an explicit edit.
pub(crate) fn unambiguous_correction(word: &str) -> Option<String> {
    if !(3..=24).contains(&word.len())
        || !word.bytes().all(|byte| byte.is_ascii_lowercase())
        || known_english_word(word)
        || english_word_prefix(word)
    {
        return None;
    }
    let mut candidates = std::collections::BTreeSet::new();
    let mut consider = |candidate: String| {
        if valid_correction(word, &candidate) {
            candidates.insert(candidate);
        }
    };
    // The first character stays unchanged, matching the existing spelling
    // contract. At most 1,298 short candidates are checked for a 24-byte word.
    for index in 1..word.len() {
        consider(format!("{}{}", &word[..index], &word[index + 1..]));
        for replacement in b'a'..=b'z' {
            consider(format!(
                "{}{}{}",
                &word[..index],
                char::from(replacement),
                &word[index + 1..]
            ));
        }
        if index + 1 < word.len() {
            consider(format!(
                "{}{}{}{}",
                &word[..index],
                char::from(word.as_bytes()[index + 1]),
                char::from(word.as_bytes()[index]),
                &word[index + 2..]
            ));
        }
    }
    for index in 1..=word.len() {
        for insertion in b'a'..=b'z' {
            consider(format!(
                "{}{}{}",
                &word[..index],
                char::from(insertion),
                &word[index..]
            ));
        }
    }
    (candidates.len() == 1)
        .then(|| candidates.pop_first())
        .flatten()
}

pub(crate) fn correction_payload(word: &str) -> Value {
    json!({"prompt":format!("Correct spelling. Return one word.\nrecieve -> receive\nhelo -> hello\nworld -> world\n{word} ->"),
        "n_predict":4,"temperature":0.0,"seed":42,"stop":["\n"],"stream":true,"cache_prompt":false})
}

/// Restrict spelling edits to a single insertion, deletion, substitution, or
/// adjacent transposition. Preserve initial letters to avoid guessing names.
pub(crate) fn valid_correction(original: &str, corrected: &str) -> bool {
    if !original.bytes().all(|byte| byte.is_ascii_lowercase())
        || original == corrected
        || known_english_word(original)
        || !known_english_word(corrected)
        || original.bytes().next() != corrected.bytes().next()
        || !corrected.bytes().all(|byte| byte.is_ascii_lowercase())
        || !(3..=24).contains(&corrected.len())
    {
        return false;
    }
    let a = original.as_bytes();
    let b = corrected.as_bytes();
    let mismatch = a
        .iter()
        .zip(b)
        .position(|(a, b)| a != b)
        .unwrap_or(a.len().min(b.len()));
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Equal => {
            a[mismatch + 1..] == b[mismatch + 1..]
                || (mismatch + 1 < a.len()
                    && a[mismatch] == b[mismatch + 1]
                    && a[mismatch + 1] == b[mismatch]
                    && a[mismatch + 2..] == b[mismatch + 2..])
        }
        std::cmp::Ordering::Less => b.len() == a.len() + 1 && a[mismatch..] == b[mismatch + 1..],
        std::cmp::Ordering::Greater => a.len() == b.len() + 1 && a[mismatch + 1..] == b[mismatch..],
    }
}

/// A streaming token is not necessarily a complete word. Wait for a following
/// separator (or a terminal model stop) before returning a bounded continuation.
pub(crate) fn complete_word_prefix(raw: &str, limit: usize, finished: bool) -> Option<String> {
    if raw.is_empty()
        || raw.contains(['\n', '\r', '<', '>', '`'])
        || raw.chars().any(char::is_control)
    {
        return None;
    }
    let words: Vec<_> = raw.unicode_word_indices().collect();
    let separator = raw.rfind(' ').unwrap_or(0);
    let complete: Vec<_> = words
        .into_iter()
        .filter(|(start, word)| finished || start + word.len() <= separator)
        .collect();
    if complete.is_empty() || (!finished && complete.len() < limit) {
        return None;
    }
    let count = complete.len().min(limit);
    let (start, word) = complete[count - 1];
    let mut end = start + word.len();
    if finished && count == complete.len() {
        end = raw.trim_end().len();
    }
    let output = raw[..end].to_owned();
    if output.chars().count() > crate::protocol::MAX_SUGGESTION_CHARS
        || crate::segment::sanitize_suggestion(&output).is_err()
    {
        return None;
    }
    Some(output)
}

#[derive(Debug, Error)]
pub enum WritingError {
    #[error(
        "the pinned writing runtime requires Linux x86_64 with AVX2; hardware advice alone does not supply a compatible runtime"
    )]
    UnsupportedRuntime,
    #[error("invalid model data directory")]
    DataDirectory,
    #[error("no writing model fits current hardware memory; run badictl hardware")]
    NoFit,
    #[error(
        "writing model/runtime not installed ({0}); run badictl models writing for the pinned download plan"
    )]
    NotInstalled(&'static str),
    #[error("model/runtime verification failed")]
    Provenance(#[from] ProvenanceError),
    #[error("local writing runtime failed")]
    Runtime(#[from] RuntimeError),
    #[error("model verification task failed")]
    VerificationTask,
}

#[cfg(test)]
mod tests {
    use super::{complete_word_prefix, correction_word, valid_correction};

    #[test]
    fn token_healing_binds_the_exact_typed_stem() {
        let request = crate::provider::ProviderRequest {
            before: "Please review the docum".to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
        };
        let payload = super::completion_payload(&request);
        assert_eq!(payload["prompt"], "Please review the ");
        assert_eq!(payload["grammar"], "root ::= \"docum\" [^<>\\n\\r`]*");
        assert_eq!(payload["cache_prompt"], true);
        assert_eq!(
            super::strip_healed_prefix("document", Some("docum")),
            Some("ent")
        );
        assert_eq!(super::strip_healed_prefix("doc", Some("docum")), None);
        assert_eq!(super::strip_healed_prefix("report", Some("docum")), None);
        assert_eq!(super::healing_prefix("Please review "), None);
        assert_eq!(super::healing_prefix("Please review."), None);
        assert_eq!(super::healing_prefix("We use version1"), None);
        assert_eq!(super::healing_prefix("Please find attached the"), None);
        assert_eq!(super::healing_prefix("او می‌تواند"), Some("می‌تواند"));
        assert_eq!(
            super::available_complete_words("ent and prov"),
            Some("ent and".to_owned())
        );
        assert_eq!(super::available_complete_words("partial"), None);
    }

    #[test]
    fn cold_inference_uses_a_bounded_sentence_without_changing_authority_context() {
        let original = format!(
            "{} Previous sentence. Please review the docum",
            "Earlier context. ".repeat(20)
        );
        let request = crate::provider::ProviderRequest {
            before: original.clone(),
            after: String::new(),
            language: Some("en".to_owned()),
        };
        let payload = super::completion_payload(&request);
        assert_eq!(payload["prompt"], "Please review the ");
        assert_eq!(request.before, original);
        for context in ["word ".repeat(100), "فارسی ".repeat(100), "x".repeat(300)] {
            assert!(super::inference_context(&context).chars().count() <= 160);
        }
        assert_eq!(
            super::inference_context("We use version 2.5 in production"),
            "We use version 2.5 in production"
        );
        for (context, expected) in [
            ("Thanks for sharing the draft. I will", "I will"),
            ("Vielen Dank für Ihre Nachricht. Ich werde", "Ich werde"),
            ("از پیام شما ممنونم. من فردا", "من فردا"),
            ("آماده هستید؟ من فردا", "من فردا"),
            ("The user chose blue.\nMake the heading", "Make the heading"),
            ("Previous line\nNext line", "Next line"),
            ("A complete sentence. ", "A complete sentence. "),
            (
                "domain.example remains intact",
                "domain.example remains intact",
            ),
        ] {
            assert_eq!(super::inference_context(context), expected);
        }
        let german = crate::provider::ProviderRequest {
            before: "Vielen Dank für Ihre".to_owned(),
            after: String::new(),
            language: Some("de".to_owned()),
        };
        assert_eq!(super::completion_payload(&german)["prompt"], german.before);
        assert!(super::completion_plan(&german, false).echo.is_none());
    }

    #[test]
    fn opt_in_space_boundary_preserves_exact_bytes_and_the_eight_token_budget() {
        for (language, before) in [
            ("en-US", "Please review the "),
            ("de-DE", "Bitte lies das "),
            ("fa", "لطفا این گزارش را "),
            ("en", "Please review the  "),
        ] {
            let request = crate::provider::ProviderRequest {
                before: before.to_owned(),
                after: String::new(),
                language: Some(language.to_owned()),
            };
            let plan = super::completion_plan(&request, true);
            let original = super::inference_context(before);
            assert_eq!(
                format!("{}{}", plan.prompt, plan.echo.expect("ASCII echo")),
                original
            );
            assert_eq!(
                plan.echo,
                Some(&original[original.trim_end_matches(' ').len()..])
            );
            assert_eq!(plan.max_tokens, 8);
            let payload = plan.payload();
            assert_eq!(payload["stop"], serde_json::json!([".", "\n"]));
            assert_eq!(payload["seed"], 42);
            assert_eq!(payload["cache_prompt"], true);
            assert_eq!(request.before, before, "editing context must not change");
            let legacy = super::completion_plan(&request, false);
            assert_eq!(legacy.prompt, original);
            assert_eq!(legacy.echo, None);
            assert_eq!(legacy.max_tokens, 8);
        }
    }

    #[test]
    fn boundary_candidate_keeps_legacy_stems_and_never_trims_other_whitespace() {
        for (language, before, tokens) in [
            ("en", "Please review the docum", 12),
            ("en", "Please review the", 8),
            ("de", "Bitte lies die Dokum", 8),
            ("fa", "لطفا این گزار", 8),
            ("en", "Please review the\u{a0}", 8),
            ("en", "Please review the\t", 8),
            ("en", "Please review the\n", 8),
            ("en", "Please review the \n", 8),
        ] {
            let request = crate::provider::ProviderRequest {
                before: before.to_owned(),
                after: String::new(),
                language: Some(language.to_owned()),
            };
            let legacy = super::completion_plan(&request, false);
            let candidate = super::completion_plan(&request, true);
            assert_eq!(legacy.payload(), candidate.payload());
            assert_eq!(candidate.max_tokens, tokens);
            assert_eq!(
                format!("{}{}", candidate.prompt, candidate.echo.unwrap_or("")),
                super::inference_context(before)
            );
            assert_eq!(request.before, before);
        }
        let before = format!(
            "{} Earlier sentence. Please review the  ",
            "Background. ".repeat(30)
        );
        let request = crate::provider::ProviderRequest {
            before: before.clone(),
            after: String::new(),
            language: Some("en".to_owned()),
        };
        let plan = super::completion_plan(&request, true);
        assert_eq!(plan.prompt, "Please review the");
        assert_eq!(plan.echo, Some("  "));
        assert_eq!(request.before, before);
    }

    #[test]
    fn writing_continues_same_script_words_without_relaxing_other_boundaries() {
        for (before, suggestion) in [
            ("Please review the docum", "entation before the meeting"),
            ("The software should autom", "atically save changes"),
            ("Die neue Dokum", "entation ist verfügbar"),
            ("این گزار", "ش را بررسی کنید"),
        ] {
            assert!(super::validate_suggestion_shape(before, "", suggestion).is_ok());
            assert!(crate::segment::validate_suggestion_shape(before, "", suggestion).is_err());
        }
        for (before, after, suggestion) in [
            ("Please review ", "", " the document"),
            ("Please review", "", " review"),
            ("Please review", "", "  the document"),
            ("Please rev", " existing", "iew existing"),
            ("Please rev", "iew", "iew"),
            ("Please rev", "", "بررسی"),
            ("version 12", "", "34"),
        ] {
            assert!(super::validate_suggestion_shape(before, after, suggestion).is_err());
        }
    }

    #[test]
    fn partial_word_proposals_require_a_real_english_word() {
        for (before, suggestion) in [
            ("Please review the docum", "ent and provide a"),
            ("The software should autom", "atically detect changes"),
        ] {
            assert!(super::validate_proposal(before, "", suggestion, Some("en")).is_ok());
        }
        for (before, suggestion, language) in [
            ("Please review the docum", "net and provide a", "en"),
            ("I use omarchy", "thm's method for the", "en"),
            ("I use Badi", "ou's concept", "en"),
            ("Neue Dokum", "entation ist verfügbar", "de"),
            ("این گزار", "ش را بررسی کنید", "fa"),
        ] {
            assert!(super::validate_proposal(before, "", suggestion, Some(language)).is_err());
        }
        for (before, suggestion, language) in [
            ("I use Badi", " for writing", "en"),
            ("Vielen Dank für Ihre", " Nachricht.", "de"),
            ("گزارش را", " بررسی کنید", "fa"),
        ] {
            assert!(super::validate_proposal(before, "", suggestion, Some(language)).is_ok());
        }
    }

    #[test]
    fn writing_languages_have_explicit_script_and_format_boundaries() {
        use super::WritingLanguage::{English, German, Persian};
        assert_eq!(super::WritingLanguage::from_tag("DE-de"), Some(German));
        assert_eq!(super::WritingLanguage::from_tag("fa-IR"), Some(Persian));
        assert_eq!(super::WritingLanguage::from_tag("ar"), None);
        assert!(English.accepts_output(" the next step"));
        assert!(German.accepts_output(" für Ihre Unterstützung"));
        assert!(Persian.accepts_output(" بررسی کنید"));
        assert!(Persian.accepts_output(" می\u{200c}شود"));
        for language in [English, German] {
            assert!(!language.accepts_output(" بررسی کنید"));
        }
        for output in [
            " next step",
            " 世界",
            " 👍",
            "\u{202e}بررسی",
            " می\u{200c} شود",
        ] {
            assert!(!Persian.accepts_output(output));
        }
        assert_eq!(
            complete_word_prefix(" بررسی کنید", 4, true).as_deref(),
            Some(" بررسی کنید")
        );
        assert_eq!(complete_word_prefix(" \u{202e}بررسی", 4, true), None);
    }

    #[test]
    fn an_installed_filename_does_not_bypass_model_verification() {
        let directory = tempfile::tempdir().expect("model directory");
        let model =
            crate::model_selection::catalog(crate::model_selection::ModelUseCase::Writing)[1];
        std::fs::create_dir(directory.path().join("models")).expect("models");
        std::fs::create_dir_all(directory.path().join("runtimes/llama-b10726")).expect("runtime");
        std::fs::write(
            directory.path().join("models").join(model.filename),
            b"corrupt",
        )
        .expect("invalid weights");
        std::fs::write(
            directory.path().join("runtimes/llama-b10726/llama-server"),
            b"unused",
        )
        .expect("unused runtime");
        assert!(matches!(
            super::verified_launch(directory.path(), model, 4),
            Err(super::WritingError::Provenance(
                crate::semantic::provenance::ProvenanceError::SizeMismatch
            ))
        ));
    }

    #[test]
    fn spelling_proposals_are_conservative_and_leave_names_alone() {
        for (before, after) in [
            ("teh", "the"),
            ("recieve", "receive"),
            ("adress", "address"),
            ("definately", "definitely"),
        ] {
            assert!(valid_correction(before, after));
        }
        for (before, after) in [
            ("omarchy", "monarchy"),
            ("their", "their"),
            ("cat", "elephant"),
            ("color", "colon"),
            ("badi", "badix"),
        ] {
            assert!(!valid_correction(before, after));
        }
        assert_eq!(correction_word("I use Badi"), None);
        assert_eq!(correction_word("Please find attached the"), None);
        assert_eq!(correction_word("This is teh "), None);
        assert_eq!(correction_word("This is teh"), Some("teh"));
        for before in [
            "Please review this project",
            "Please check the documents",
            "Use the latest",
            "I use Obsidian",
        ] {
            assert_eq!(correction_word(before), None, "{before}");
        }
    }

    #[test]
    fn dictionary_spelling_returns_only_unambiguous_single_edits() {
        for (original, corrected) in [
            ("adress", "address"),
            ("definately", "definitely"),
            ("exampel", "example"),
            ("langauge", "language"),
            ("seperate", "separate"),
            ("occured", "occurred"),
            ("acheive", "achieve"),
        ] {
            assert_eq!(
                super::unambiguous_correction(original).as_deref(),
                Some(corrected),
                "{original}"
            );
        }
        // Frequency and context are not present in a word set. Never choose
        // the first alphabetic candidate for an ambiguous misspelling.
        for original in [
            "teh", "recieve", "wierd", "realy", "Badi", "omarchy", "badi", "color", "thier",
            "tommorow", "ab", "naïve", "миp", "the",
        ] {
            assert_eq!(super::unambiguous_correction(original), None, "{original}");
        }
        for partial in ["expl", "docum", "autom", "attache", "programmi"] {
            assert_eq!(correction_word(partial), None, "{partial}");
            assert_eq!(super::unambiguous_correction(partial), None, "{partial}");
        }
    }

    #[test]
    fn spelling_target_owns_only_the_word_and_a_single_typed_space() {
        for (before, suffix, delimiter) in [
            ("This is teh", "teh", ""),
            ("This is teh ", "teh ", " "),
            ("teh ", "teh ", " "),
        ] {
            let target = super::correction_target(before).expect("correction target");
            assert_eq!(target.word, "teh");
            assert_eq!(target.suffix, suffix);
            assert_eq!(target.delimiter, delimiter);
        }
        for before in [
            "This is teh  ",
            "This is teh\t",
            "This is teh\n",
            "This is teh\u{00a0}",
            "This is teh.",
            "This is the ",
            "This is Teh ",
            "This is docum ",
        ] {
            assert!(super::correction_target(before).is_none(), "{before:?}");
        }
    }

    #[test]
    fn streaming_never_returns_a_partial_last_word() {
        for apostrophe in ['\'', '\u{2019}'] {
            assert_eq!(
                super::available_complete_words(&format!(" don{apostrophe}")),
                None
            );
            assert_eq!(
                complete_word_prefix(&format!(" one two three don{apostrophe}"), 4, false),
                None
            );
            assert_eq!(
                super::available_complete_words(&format!(" one two three don{apostrophe}")),
                Some(" one two three".to_owned())
            );
            assert_eq!(
                complete_word_prefix(&format!(" don{apostrophe}t "), 1, false),
                Some(format!(" don{apostrophe}t"))
            );
        }
        assert_eq!(
            complete_word_prefix(" the updated project rep", 4, false),
            None
        );
        assert_eq!(
            complete_word_prefix(" the updated project report for", 4, false).as_deref(),
            Some(" the updated project report")
        );
        assert_eq!(
            complete_word_prefix(" tomorrow.", 4, true).as_deref(),
            Some(" tomorrow.")
        );
        assert_eq!(complete_word_prefix(" the next step", 4, false), None);
        assert_eq!(
            complete_word_prefix("<think>private reasoning", 4, true),
            None
        );
    }
}
