//! Hardware-selected local writing inference. Artifact advice is never treated
//! as a release qualification; startup verifies the bytes actually executed.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use thiserror::Error;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

use crate::model_selection::{
    HardwareProfile, ModelArtifact, ModelUseCase, catalog, current_memory,
    detect_cpu_inference_hardware, fits_available_memory, recommend_model,
    select_installed_writing_model,
};
use crate::provider::ProviderRequest;
use crate::semantic::pinned_runtime::{
    RUNTIME_ARCHIVE_BYTES, RUNTIME_ARCHIVE_FILENAME, RUNTIME_ARCHIVE_SHA256,
    RUNTIME_BUNDLE_MANIFEST_SHA256, RUNTIME_BYTES, RUNTIME_SHA256,
};
#[cfg(target_os = "linux")]
pub use crate::semantic::process::{EXEC_HELPER_FLAG, exec_runtime_helper};
use crate::semantic::provenance::{
    DirectoryManifestExpectation, FileExpectation, ProvenanceError, VerifiedFile,
    verify_directory_manifest, verify_file,
};
use crate::semantic::runtime::{LlamaCppLaunch, OwnedRuntime, RuntimeError, WarmUpReport};

pub const WRITING_CONTRACT: &str = "badi.writing.completion-and-spelling.en-de-fa.v2";
const MEMORY_RETRY_INITIAL: Duration = Duration::from_secs(2);
const MEMORY_RETRY_MAX: Duration = Duration::from_secs(30);

/// A language the writing provider completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritingLanguage {
    English,
    German,
    Persian,
}

impl WritingLanguage {
    /// The writing language of a BCP 47 `tag`, by its primary subtag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.split('-').next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            "fa" => Some(Self::Persian),
            _ => None,
        }
    }

    /// Whether `value` uses only this language's scripts and punctuation.
    #[must_use]
    pub fn accepts_output(self, value: &str) -> bool {
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

/// Checks a completion's spacing, overlap and boundaries against the text
/// around the caret.
pub fn validate_suggestion_shape(
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

/// [`validate_suggestion_shape`], plus: a completion that continues a word
/// must be English and form a known English word.
pub fn validate_proposal(
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

/// Normalize ASCII, Arabic-Indic and Persian digits to ASCII.
#[must_use]
pub fn ascii_digit(character: char) -> Option<char> {
    let value = match character {
        '0'..='9' => character as u32 - '0' as u32,
        '\u{0660}'..='\u{0669}' => character as u32 - '\u{0660}' as u32,
        '\u{06f0}'..='\u{06f9}' => character as u32 - '\u{06f0}' as u32,
        _ => return None,
    };
    char::from_u32('0' as u32 + value)
}

fn digit_runs(text: &str) -> std::collections::BTreeSet<String> {
    let mut runs = std::collections::BTreeSet::new();
    let mut run = String::new();
    for character in text.chars() {
        if let Some(digit) = ascii_digit(character) {
            run.push(digit);
        } else if !run.is_empty() {
            runs.insert(std::mem::take(&mut run));
        }
    }
    if !run.is_empty() {
        runs.insert(run);
    }
    runs
}

/// A small model readily invents dates, times, amounts and ordinals. Reject a
/// proposal unless each of its digit runs already occurs in the typed context,
/// and reject a number ending at the `.` stop word, which may be an ordinal or
/// a cut decimal.
pub(crate) fn introduces_unsupported_fact(context: &str, proposal: &str) -> bool {
    if proposal
        .strip_suffix('.')
        .and_then(|value| value.chars().next_back())
        .is_some_and(|character| ascii_digit(character).is_some())
    {
        return true;
    }
    let proposed = digit_runs(proposal);
    !proposed.is_empty() && !proposed.is_subset(&digit_runs(context))
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

/// Starts the installed writing model, first waiting while available memory
/// is temporarily too short for it.
pub async fn activate(
    directory: PathBuf,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    start(directory, false).await
}

/// [`activate`] for a binary that dispatches [`EXEC_HELPER_FLAG`] before it
/// starts Tokio: the runtime then cannot outlive this process. The kernel
/// signals the runtime when the spawning thread exits, so await this on a
/// thread that lives as long as the process, such as `main`'s `block_on`.
#[cfg(target_os = "linux")]
pub async fn activate_contained(
    directory: PathBuf,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    start(directory, true).await
}

async fn start(
    directory: PathBuf,
    contained: bool,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    let (model, threads) = installed_model(&directory)?;
    wait_for_memory(model).await;
    let launch = verify_launch(directory, model, threads)
        .await?
        .for_writing();
    let launch = if contained {
        launch.contained()
    } else {
        launch
    };
    let runtime = launch.spawn().await?;
    // Pay the first-inference cost before the broker binds its socket. No real
    // request can queue behind this bounded job, and a failure only leaves the
    // first request cold.
    let warm_up = runtime.warm_up().await;
    Ok((runtime, model, warm_up))
}

async fn wait_for_memory(model: ModelArtifact) {
    let started = Instant::now();
    let mut delay = None;
    loop {
        let available_mib = current_memory().available_mib;
        let Some(next) = memory_retry_delay(model, available_mib, delay) else {
            break;
        };
        if delay.is_none() {
            eprintln!(
                "badi-broker: waiting for available memory before starting the writing model available_mib={}",
                available_mib.unwrap_or_default()
            );
        }
        delay = Some(next);
        tokio::time::sleep(next).await;
    }
    if delay.is_some() {
        eprintln!(
            "badi-broker: memory available after waiting elapsed_s={}",
            started.elapsed().as_secs()
        );
    }
}

/// Whether `model` can start with `available_mib` of available memory.
/// Unknown availability does not block: total memory already passed the hard
/// floor during selection.
#[must_use]
pub fn memory_fits(model: ModelArtifact, available_mib: Option<u64>) -> bool {
    available_mib.is_none_or(|available| fits_available_memory(model, available))
}

/// The pause before checking available memory again, or `None` once `model`
/// can start.
fn memory_retry_delay(
    model: ModelArtifact,
    available_mib: Option<u64>,
    previous: Option<Duration>,
) -> Option<Duration> {
    if memory_fits(model, available_mib) {
        return None;
    }
    Some(previous.map_or(MEMORY_RETRY_INITIAL, |delay| {
        delay.saturating_mul(2).min(MEMORY_RETRY_MAX)
    }))
}

/// Content-free startup lines, provider line first: log readers such as the
/// Fcitx broker smoke lane require the log to begin with it. Only the memory
/// wait lines can precede it, and only when startup had to wait.
#[must_use]
pub fn activation_report(model: &ModelArtifact, warm_up: WarmUpReport) -> String {
    format!(
        "provider=local_model model={} quantization={}\nbadi-broker: writing runtime warm-up {warm_up}",
        model.filename, model.quantization
    )
}

/// The installed catalog model that fits this host, with the inference
/// thread count, from the models under `directory`.
pub fn installed_model(directory: &Path) -> Result<(ModelArtifact, usize), WritingError> {
    let hardware = detect_cpu_inference_hardware();
    let threads = threads_for(&hardware)?;
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
    Ok((model, threads))
}

/// Verifies the installed `model` and the pinned runtime under `directory`
/// off the async executor, and returns their launch.
pub async fn verify_launch(
    directory: PathBuf,
    model: ModelArtifact,
    threads: usize,
) -> Result<LlamaCppLaunch, WritingError> {
    tokio::task::spawn_blocking(move || verified_launch(&directory, model, threads))
        .await
        .map_err(|_| WritingError::VerificationTask)?
}

/// The inference thread count for this host's CPU.
pub fn writing_threads() -> Result<usize, WritingError> {
    threads_for(&detect_cpu_inference_hardware())
}

fn threads_for(hardware: &HardwareProfile) -> Result<usize, WritingError> {
    if std::env::consts::OS != "linux" || hardware.architecture != "x86_64" || !hardware.cpu.avx2 {
        return Err(WritingError::UnsupportedRuntime);
    }
    // Small quantized models are bandwidth-bound on the tested hybrid CPU.
    // Leave capacity for the editor; twelve threads had worse tail latency.
    Ok((hardware.logical_cpus / 2).clamp(1, 4))
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
    pinned_runtime_launch(directory, weights, model.filename, threads)
}

/// Verifies the pinned runtime release under `directory` and returns its
/// launch of the already verified `weights`. Blocking: it hashes the runtime.
pub fn pinned_runtime_launch(
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

/// Languages whose trailing ASCII space is healed in production. A language is
/// listed only while blinded per-language review finds no increase in harmful
/// suggestions over the unhealed prompt; an unlisted language keeps that
/// prompt, trailing space included. Removing a language is its rollback.
/// German is unlisted: the sealed 2026-09-10 set rose from 1 to 9 harmful.
/// English was kept by user decision at +1 harmful for +19 useful in 40.
pub(crate) const TRAILING_SPACE_HEALING: &[WritingLanguage] =
    &[WritingLanguage::English, WritingLanguage::Persian];

/// Whether production heals a trailing ASCII space for this language tag.
#[must_use]
pub fn heals_trailing_space(language: &str) -> bool {
    WritingLanguage::from_tag(language)
        .is_some_and(|language| TRAILING_SPACE_HEALING.contains(&language))
}

/// Production passes [`TRAILING_SPACE_HEALING`]; tests pass other sets to
/// check each language's promotion and rollback on its own.
pub(crate) fn completion_plan<'a>(
    request: &'a ProviderRequest,
    healed_languages: &[WritingLanguage],
) -> CompletionPlan<'a> {
    let context = inference_context(&request.before);
    let language = request
        .language
        .as_deref()
        .and_then(WritingLanguage::from_tag);
    if language.is_some_and(|language| healed_languages.contains(&language))
        && context.ends_with(' ')
    {
        // A prompt ending in a space tokenizes unlike the model's own
        // " word" tokens. Generate at the earlier boundary and require the
        // exact spaces back; the echo keeps the ordinary eight-token budget.
        let prompt = context.trim_end_matches(' ');
        return CompletionPlan {
            prompt,
            echo: Some(&context[prompt.len()..]),
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
    completion_plan(request, TRAILING_SPACE_HEALING).payload()
}

/// A sentence start with fewer words than this also keeps the sentence(s)
/// before it, within the same 160-scalar window.
const MIN_SENTENCE_CONTEXT_WORDS: usize = 4;

/// Words for the sentence-context rule: whitespace-separated runs containing a
/// letter or digit. ZWNJ is not whitespace, so `می‌شود` is one word, and a
/// lone dash is not a word.
fn context_words(text: &str) -> usize {
    text.split_whitespace()
        .filter(|run| run.chars().any(char::is_alphanumeric))
        .count()
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
    // the first token, so keep the current sentence. At a sentence start that
    // leaves the model almost nothing (`Please`), so extend back one sentence
    // at a time until the prompt has four words; when the text is typed in
    // order, the previous request's prompt is then a cached prefix. A sentence
    // end is `.`, `!`, `?` or `؟` before whitespace, or a newline, so
    // abbreviations such as `z. B.` and `e.g.` also end one. Edit authority
    // still binds the unchanged full request.
    for (index, character) in window.char_indices().rev() {
        if matches!(character, '.' | '!' | '?' | '\u{061f}' | '\n') {
            let after = &window[index + character.len_utf8()..];
            if (character == '\n' || after.starts_with(char::is_whitespace))
                && context_words(after) >= MIN_SENTENCE_CONTEXT_WORDS
            {
                return after.trim_start();
            }
        }
    }
    // Too few words after every sentence start in the window: keep the whole
    // window, which still begins at a whitespace boundary.
    window
}

/// The trailing word of `before` that token healing regenerates: two to 24
/// letters that are not a known English word.
#[must_use]
pub fn healing_prefix(before: &str) -> Option<&str> {
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
    let count = spaced_words(raw)
        .filter(|(run_end, _)| *run_end <= separator)
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

/// Visible words for the suggestion limit: runs between ASCII spaces, the only
/// word separator a suggestion may contain. UAX #29 splits `re-try`, `E-Mail`
/// and `state-of-the-art`, so its segments cannot decide where a displayed word
/// ends. Each item is (end of the run, end of the run's last UAX #29 word); the
/// latter drops trailing punctuation such as a comma. A run without a word,
/// such as a dash, is not counted.
fn spaced_words(raw: &str) -> impl Iterator<Item = (usize, usize)> + '_ {
    let mut next = 0;
    raw.split(' ').filter_map(move |run| {
        let start = next;
        next += run.len() + 1;
        let (offset, word) = run.unicode_word_indices().next_back()?;
        Some((start + run.len(), start + offset + word.len()))
    })
}

/// A streaming token is not necessarily a complete word. Wait for a following
/// separator (or a terminal model stop) before returning a bounded continuation.
#[must_use]
pub fn complete_word_prefix(raw: &str, limit: usize, finished: bool) -> Option<String> {
    if raw.is_empty()
        || raw.contains(['\n', '\r', '<', '>', '`'])
        || raw.chars().any(char::is_control)
    {
        return None;
    }
    let separator = raw.rfind(' ').unwrap_or(0);
    let complete: Vec<_> = spaced_words(raw)
        .filter(|(run_end, _)| finished || *run_end <= separator)
        .map(|(_, word_end)| word_end)
        .collect();
    if complete.is_empty() || (!finished && complete.len() < limit) {
        return None;
    }
    let count = complete.len().min(limit);
    let mut end = complete[count - 1];
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

impl WritingError {
    /// Whether retrying cannot help until the installation or host changes.
    #[must_use]
    pub const fn is_configuration(&self) -> bool {
        matches!(
            self,
            Self::UnsupportedRuntime | Self::DataDirectory | Self::NoFit | Self::NotInstalled(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{complete_word_prefix, correction_word, memory_retry_delay, valid_correction};

    #[test]
    fn short_memory_delays_start_with_bounded_backoff() {
        let model =
            crate::model_selection::catalog(crate::model_selection::ModelUseCase::Writing)[1];
        assert_eq!(memory_retry_delay(model, Some(16_000), None), None);
        assert_eq!(memory_retry_delay(model, None, None), None);
        let mut delays = Vec::new();
        let mut previous = None;
        for _ in 0..7 {
            previous = memory_retry_delay(model, Some(4_000), previous);
            delays.push(previous.expect("short memory waits").as_secs());
        }
        assert_eq!(delays, [2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(memory_retry_delay(model, Some(8_000), previous), None);
    }

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
            (
                "Thanks for sharing the draft. I will read it",
                "I will read it",
            ),
            (
                "Vielen Dank für Ihre Nachricht. Ich werde sie morgen",
                "Ich werde sie morgen",
            ),
            ("از پیام شما ممنونم. من فردا گزارش را", "من فردا گزارش را"),
            ("آماده هستید؟ من فردا گزارش را", "من فردا گزارش را"),
            (
                "The user chose blue.\nMake the heading bold",
                "Make the heading bold",
            ),
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
        assert!(super::completion_plan(&german, &[]).echo.is_none());
    }

    #[test]
    fn a_short_sentence_start_keeps_the_previous_sentences_in_the_window() {
        for context in [
            // Fewer than four words after the last sentence end.
            "Thanks for sharing the draft. I will",
            "Thanks for sharing the draft. I will send",
            "Vielen Dank für Ihre Nachricht. Ich werde",
            "Aus dem Protokoll ist der Grund nicht ersichtlich. Deshalb ",
            "از پیام شما ممنونم. من فردا",
            "آماده هستید؟ من فردا",
            "Great news! We",
            "The user chose blue.\nMake the heading",
            "Previous line\nNext line",
            "Hi Anna,\n\nThanks",
            "Hi. Thanks. See you",
            "First sentence. Second one. ",
            // ZWNJ joins one word: three words, not four.
            "پیام شما رسید. من می\u{200c}خواهم فردا",
            // A lone dash is not a word: three words, not four.
            "Thanks for the draft. Yes – I will",
            // Abbreviations end a sentence; each piece is still short.
            "Wir brauchen Obst, z. B. Äpfel",
        ] {
            assert_eq!(super::inference_context(context), context, "{context:?}");
        }
        for (context, expected) in [
            // Extension stops once the kept sentences have four words.
            (
                "I read your notes. The draft looks good to me. Thanks. Please",
                "The draft looks good to me. Thanks. Please",
            ),
            (
                "Thanks for sharing the draft. I will send it",
                "I will send it",
            ),
            (
                "پیام شما رسید. من می\u{200c}خواهم فردا صبح",
                "من می\u{200c}خواهم فردا صبح",
            ),
            // `z. B.` is treated as a sentence end (documented limitation).
            (
                "Ich brauche Zutaten, z. B. Mehl, Zucker, Eier und Butter",
                "Mehl, Zucker, Eier und Butter",
            ),
        ] {
            assert_eq!(super::inference_context(context), expected, "{context:?}");
        }
        // The 160-scalar window and its whitespace cut still bound the prompt,
        // even when the previous sentence begins before the window.
        for (sentence, start) in [
            ("word ".repeat(40), "Please"),
            ("Wort ".repeat(40), "Bitte"),
            ("کلمه ".repeat(40), "لطفا"),
        ] {
            let before = format!("{}. {start} ", sentence.trim_end());
            let context = super::inference_context(&before);
            assert!(context.chars().count() <= 160, "{context:?}");
            assert!(context.ends_with(&format!(". {start} ")), "{context:?}");
            assert!(before[..before.len() - context.len()].ends_with(' '));
            assert!(context.starts_with(sentence.split(' ').next().expect("word")));
        }
        // Typed in order, the previous request's prompt is a prefix of the
        // prompt at the next sentence start, so the runtime reuses its cache.
        let previous = super::inference_context("Thanks for sharing the draft");
        let next = super::inference_context("Thanks for sharing the draft. I");
        assert_eq!(next, "Thanks for sharing the draft. I");
        assert!(next.starts_with(previous));
        let request = crate::provider::ProviderRequest {
            before: "Previous notes are here. Thanks for sharing the draft. Please ".to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
        };
        let plan = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
        assert_eq!(plan.prompt, "Thanks for sharing the draft. Please");
        assert_eq!(plan.echo, Some(" "));
    }

    #[test]
    fn trailing_space_healing_is_promoted_per_language() {
        use super::WritingLanguage::{English, German, Persian};
        // A reviewed per-language promotion: a blinded review cleared English
        // and Persian, and German failed it. Change it only after a new review.
        assert_eq!(super::TRAILING_SPACE_HEALING, [English, Persian]);
        for (tag, language, before) in [
            ("en-US", English, "Please review the "),
            ("de", German, "Bitte lies das "),
            ("fa-IR", Persian, "لطفا این گزارش را "),
        ] {
            let request = crate::provider::ProviderRequest {
                before: before.to_owned(),
                after: String::new(),
                language: Some(tag.to_owned()),
            };
            let legacy = super::completion_plan(&request, &[]).payload();
            assert_eq!(legacy["prompt"], before);
            assert!(legacy.get("grammar").is_none());
            let promoted = super::completion_plan(&request, &[language]);
            assert_eq!(promoted.prompt, before.trim_end_matches(' '));
            assert_eq!(promoted.echo, Some(" "));
            // Promoting the other languages leaves this one unhealed.
            let others: Vec<_> = [English, German, Persian]
                .into_iter()
                .filter(|other| *other != language)
                .collect();
            assert_eq!(super::completion_plan(&request, &others).payload(), legacy);
            let production = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
            assert_eq!(
                production.echo.is_some(),
                super::TRAILING_SPACE_HEALING.contains(&language)
            );
            assert_eq!(
                super::heals_trailing_space(tag),
                super::TRAILING_SPACE_HEALING.contains(&language)
            );
        }
        assert!(!super::heals_trailing_space("ar"));
    }

    #[test]
    fn trailing_space_boundary_preserves_exact_bytes_and_the_eight_token_budget() {
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
            // The mechanism, independent of which languages production promotes.
            let all = [
                super::WritingLanguage::English,
                super::WritingLanguage::German,
                super::WritingLanguage::Persian,
            ];
            let plan = super::completion_plan(&request, &all);
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
            let legacy = super::completion_plan(&request, &[]);
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
            let legacy = super::completion_plan(&request, &[]);
            let candidate = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
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
        let plan = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
        // Three words keep the previous sentence, but not the whole window.
        assert_eq!(plan.prompt, "Earlier sentence. Please review the");
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
    fn fact_fence_rejects_invented_numbers_and_stop_word_ordinals() {
        use super::introduces_unsupported_fact as rejects;
        for (context, proposal) in [
            ("Ich freue mich auf", " die Teilnahme am 1."),
            ("Ich freue mich auf", " die Teilnahme am 1"),
            ("The survey ran in ", "2024, but"),
            ("This essay is ", "100% original and"),
            ("Our office is open from ", "10:00 AM to"),
            ("Meet me at room 42, then room", " 4"),
            ("Meet me at room 42, then room", " 420"),
            ("Meet me at room 42, then room", " 42 or 43"),
            ("We use version 2.5 in production and", " 2.6 later"),
            // The number is known, but a stop-word period may be an ordinal
            // or a decimal cut at the stop word.
            ("Die Frist ist der 1. Mai. Wir treffen uns am", " 1."),
            ("Meet me at room 42, then room", " 42."),
            ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۳ برویم"),
            ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۲."),
            ("پیام شما رسید. من فردا", " ساعت ۱۰ می‌آیم"),
            ("پیام شما رسید. من فردا", " ساعت \u{0661}\u{0660} می‌آیم"),
        ] {
            assert!(rejects(context, proposal), "{context:?} + {proposal:?}");
        }
        for (context, proposal) in [
            ("Meet me at room 42, then room", " 42"),
            ("Meet me at room 42, then room", " 42 again"),
            ("We use version 2.5 in production and", " 2.5 remains"),
            ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۲ برویم"),
            // Digit scripts normalize before comparison.
            ("جلسه در اتاق ۴۲ است. بعد به اتاق", " 42 برویم"),
            ("Meet me at room 42, then room", " \u{0664}\u{0662}"),
            ("Ich freue mich auf", " die Teilnahme."),
            ("Thank you", " for your time."),
            ("Please find attached the", " latest version of the"),
        ] {
            assert!(!rejects(context, proposal), "{context:?} + {proposal:?}");
        }
    }

    #[test]
    fn activation_report_begins_with_the_provider_line() {
        use crate::model_selection::{ModelUseCase, catalog};
        use crate::semantic::runtime::WarmUpReport;
        use std::time::Duration;

        let model = catalog(ModelUseCase::Writing)[0];
        let provider = format!(
            "provider=local_model model={} quantization={}",
            model.filename, model.quantization
        );
        assert_eq!(
            super::activation_report(&model, WarmUpReport::new(Duration::from_millis(412), None)),
            format!("{provider}\nbadi-broker: writing runtime warm-up completed elapsed_ms=412")
        );
        assert_eq!(
            super::activation_report(
                &model,
                WarmUpReport::new(Duration::from_millis(2000), Some("timeout"))
            ),
            format!(
                "{provider}\nbadi-broker: writing runtime warm-up failed class=timeout elapsed_ms=2000"
            )
        );
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

    /// Observed with the pinned model on 2026-09-26: after the healed echo,
    /// `the client should ` streamed ` be able to re-try the request`. UAX #29
    /// splits `re-try` into two words, so the four-word limit ended inside it
    /// and displayed `be able to re`. The limit counts space-separated words.
    #[test]
    fn the_word_limit_never_cuts_inside_a_joined_word() {
        for (raw, finished, expected) in [
            ("be able to re-try the", false, Some("be able to re-try")),
            (" be able to re-try the", false, Some(" be able to re-try")),
            (
                "be able to re-try the request",
                true,
                Some("be able to re-try"),
            ),
            ("be able to re-try", false, None),
            (" a state-of-the-art tool is", false, None),
            (
                " a state-of-the-art tool is here",
                false,
                Some(" a state-of-the-art tool is"),
            ),
            (" die E-Mail an Herrn", false, None),
            (
                " die E-Mail an Herrn Meyer",
                false,
                Some(" die E-Mail an Herrn"),
            ),
            (" را به‌روز کنیم و", false, None),
            (" را به‌روز کنیم و بعد", false, Some(" را به‌روز کنیم و")),
            // Trailing punctuation of an intermediate word is still dropped.
            (
                " we agreed, then left it",
                false,
                Some(" we agreed, then left"),
            ),
            (
                " one two three four, five",
                false,
                Some(" one two three four"),
            ),
            (
                " one - two three four five",
                false,
                Some(" one - two three four"),
            ),
            (" don't re-send it now", false, None),
            (
                " don't re-send it now ",
                false,
                Some(" don't re-send it now"),
            ),
        ] {
            assert_eq!(
                complete_word_prefix(raw, 4, finished).as_deref(),
                expected,
                "{raw:?}"
            );
        }
        // Deadline salvage counts the same words.
        for (raw, expected) in [
            ("be able to re-try the", Some("be able to re-try")),
            (" a b c re-try d", Some(" a b c re-try")),
            (" re-", None),
            (" able to re-", Some(" able to")),
        ] {
            assert_eq!(
                super::available_complete_words(raw).as_deref(),
                expected,
                "{raw:?}"
            );
        }
    }
}
