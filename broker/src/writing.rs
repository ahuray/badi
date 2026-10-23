//! Hardware-selected local writing inference. Artifact advice is never treated
//! as a release qualification; startup verifies the bytes actually executed.

use std::path::{Path, PathBuf};
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
pub use crate::settings::SuggestionLength;

pub const WRITING_CONTRACT: &str = "badi.writing.completion-and-spelling.en-de-fa.v2";
const MEMORY_RETRY_INITIAL: Duration = Duration::from_secs(2);
const MEMORY_RETRY_MAX: Duration = Duration::from_secs(30);

/// A language the writing provider completes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WritingLanguage {
    English,
    German,
    Persian,
    French,
    Spanish,
    Portuguese,
}

impl WritingLanguage {
    /// The writing language of a BCP 47 `tag`, by its primary subtag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.split('-').next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            "fa" => Some(Self::Persian),
            "fr" => Some(Self::French),
            "es" => Some(Self::Spanish),
            "pt" => Some(Self::Portuguese),
            _ => None,
        }
    }

    /// The primary language subtag.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
            Self::Persian => "fa",
            Self::French => "fr",
            Self::Spanish => "es",
            Self::Portuguese => "pt",
        }
    }

    /// Whether `value` uses only this language's scripts and punctuation.
    #[must_use]
    pub fn accepts_output(self, value: &str) -> bool {
        if self != Self::Persian {
            return valid_english_output(value);
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
                || allowed_common_scalar(character)
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

/// Whether `value` is Latin-script text with only the shared punctuation,
/// digits and combining accents on a Latin base: the English, German,
/// French, Spanish and Portuguese output rule.
fn valid_english_output(value: &str) -> bool {
    if value.is_empty() || value.ends_with(char::is_whitespace) {
        return false;
    }
    let mut saw_latin = false;
    let mut mark_has_latin_base = false;
    for character in value.chars() {
        match character.script() {
            Script::Latin => {
                saw_latin = true;
                mark_has_latin_base = true;
            }
            Script::Inherited
                if mark_has_latin_base && ('\u{0300}'..='\u{036f}').contains(&character) => {}
            Script::Common if allowed_common_scalar(character) => {
                mark_has_latin_base = false;
            }
            _ => return false,
        }
    }
    saw_latin
}

/// Space, digits and the punctuation every writing language may use.
const fn allowed_common_scalar(character: char) -> bool {
    matches!(
        character,
        ' ' | '0'
            ..='9'
                | '.'
                | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '\''
                | '\u{2019}'
                | '-'
                | '\u{2013}'
                | '\u{2014}'
                | '('
                | ')'
                | '['
                | ']'
                | '/'
                | '%'
                | '\u{2026}'
    )
}

/// Checks a completion's spacing, overlap and boundaries against the text
/// around the caret.
pub fn validate_suggestion_shape(
    before: &str,
    after: &str,
    suggestion: &str,
) -> Result<(), crate::segment::OutputError> {
    // Native prefix inference may continue the word at the caret. Permit only
    // same-script letters at that left boundary, and only where no word starts
    // right after the caret; all spacing, overlap and right boundary checks
    // still apply. No document mutation authority changes.
    let suffix = after
        .chars()
        .next()
        .is_none_or(|right| !right.is_alphanumeric())
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

/// Whether the caret splits a word, where nothing is suggested.
#[must_use]
pub fn caret_inside_word(before: &str, after: &str) -> bool {
    before
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
        && after.chars().next().is_some_and(char::is_alphanumeric)
}

/// The rest of the caret's line when it holds more than whitespace: a
/// suggestion there is inserted mid-line instead of continuing the line.
#[must_use]
pub fn line_rest(after: &str) -> Option<&str> {
    let line = after.split(['\n', '\r']).next().unwrap_or_default();
    (!line.trim().is_empty()).then_some(line)
}

/// A separate word follows the caret on its line. Punctuation there is not a
/// word: the word being typed may still be completed.
#[must_use]
pub fn word_follows_caret(after: &str) -> bool {
    line_rest(after).is_some_and(|line| line.trim_start().starts_with(char::is_alphanumeric))
}

/// What a continuation inserts before `line`, the text after the caret.
///
/// The model continues from the caret. Where its continuation reproduces the
/// line's first word or punctuation mark at a word boundary, the text before
/// that point fits between the two sides, and a phrase the model closes with a
/// clause mark fits before the line's own mark. Otherwise only the rest of a
/// word being typed is offered, since whole new words could not be shown to
/// fit.
#[must_use]
pub fn mid_line_insertion(before: &str, line: &str, continuation: &str) -> Option<String> {
    let clause_mark = |character: char| matches!(character, '.' | ',' | '!' | '?' | ';' | ':');
    let start = line.len() - line.trim_start().len();
    // Finishing the typed word into the line's own first word (`ha` + `d`
    // before `had a good weekend`) repeats it, so that insertion never fits.
    let stem = before
        .rsplit(|character: char| !character.is_alphabetic())
        .next()
        .unwrap_or_default();
    let fits = |insertion: &str| {
        let repeats = !stem.is_empty()
            && insertion.chars().all(char::is_alphabetic)
            && line[start..]
                .unicode_words()
                .next()
                .is_some_and(|next| next.eq_ignore_ascii_case(&format!("{stem}{insertion}")));
        (!repeats).then(|| insertion.to_owned())
    };
    let head_len = match line[start..].chars().next() {
        Some(first) if first.is_alphanumeric() => line[start..]
            .unicode_word_indices()
            .next()
            .map_or(first.len_utf8(), |(_, word)| word.len()),
        Some(first) => first.len_utf8(),
        None => 0,
    };
    // A short next word (`the`, `to`) comes back by chance: ` the right
    // place` does not rejoin ` the figures`. There the continuation must
    // also reproduce the word after it.
    let head_len = match line[start..]
        .unicode_word_indices()
        .take(2)
        .collect::<Vec<_>>()[..]
    {
        [(_, first), (second_start, second)]
            if head_len == first.len() && first.chars().count() <= 3 =>
        {
            second_start + second.len()
        }
        _ => head_len,
    };
    if head_len > 0 {
        let head = &line[..start + head_len];
        let rejoin = continuation
            .match_indices(head)
            .map(|(index, _)| index)
            .find(|&index| {
                // A word right after the caret needs a space before it; a space or
                // punctuation there needs none.
                let needs_space = head.starts_with(char::is_alphanumeric);
                index > 0
                    && continuation[..index].ends_with(char::is_whitespace) == needs_space
                    && !continuation[index + head.len()..].starts_with(char::is_alphanumeric)
            });
        if let Some(index) = rejoin {
            return fits(&continuation[..index]);
        }
        if line[start..].starts_with(clause_mark) {
            if let Some(phrase) = continuation.trim_end().strip_suffix(clause_mark) {
                if phrase.ends_with(|character: char| character.is_alphanumeric()) {
                    return Some(phrase.to_owned());
                }
            }
        }
    }
    let word_end = continuation
        .find(|character: char| !character.is_alphabetic())
        .unwrap_or(continuation.len());
    (word_end > 0 && before.ends_with(char::is_alphabetic))
        .then(|| fits(&continuation[..word_end]))
        .flatten()
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

/// The pinned English lexicon: lowercase ASCII words, one per line, each
/// ending in a newline, in byte order.
const ENGLISH_LEXICON: &str = include_str!("../data/writing-lexicon/en.txt");

/// The first lexicon word not less than `target` in byte order, found by
/// bisecting the embedded text in place: lookups need no index or heap.
fn first_english_word_from(target: &str) -> Option<&'static str> {
    let text = ENGLISH_LEXICON.as_bytes();
    // Every word starting before `low` is less than `target`; every word
    // starting at or after `high` is not. Both are always line starts.
    let (mut low, mut high) = (0, text.len());
    while low < high {
        let middle = low + (high - low) / 2;
        let start = text[low..middle]
            .iter()
            .rposition(|&byte| byte == b'\n')
            .map_or(low, |newline| low + newline + 1);
        let end = start + line_length(&text[start..]);
        if &text[start..end] < target.as_bytes() {
            low = end + 1;
        } else {
            high = start;
        }
    }
    (low < text.len()).then(|| &ENGLISH_LEXICON[low..low + line_length(&text[low..])])
}

fn line_length(text: &[u8]) -> usize {
    text.iter()
        .position(|&byte| byte == b'\n')
        .unwrap_or(text.len())
}

pub(crate) fn known_english_word(word: &str) -> bool {
    if !word.bytes().all(|byte| byte.is_ascii_alphabetic()) {
        return false;
    }
    let word = word.to_ascii_lowercase();
    first_english_word_from(&word) == Some(word.as_str())
}

fn english_word_prefix(word: &str) -> bool {
    first_english_word_from(word).is_some_and(|candidate| candidate.starts_with(word))
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
    chosen: Option<String>,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    start(directory, false, chosen).await
}

/// [`activate`] for a binary that dispatches [`EXEC_HELPER_FLAG`] before it
/// starts Tokio: the runtime then cannot outlive this process. The kernel
/// signals the runtime when the spawning thread exits, so await this on a
/// thread that lives as long as the process, such as `main`'s `block_on`.
#[cfg(target_os = "linux")]
pub async fn activate_contained(
    directory: PathBuf,
    chosen: Option<String>,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    start(directory, true, chosen).await
}

async fn start(
    directory: PathBuf,
    contained: bool,
    chosen: Option<String>,
) -> Result<(OwnedRuntime, ModelArtifact, WarmUpReport), WritingError> {
    let (model, threads) = installed_model(&directory, chosen.as_deref())?;
    wait_for_memory(model).await;
    let launch = verify_launch(directory, model, threads).await?;
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
/// thread count, from the models under `directory`: the user's `chosen`
/// model (settings `writing_model`) when it is installed and fits, else the
/// installed model the recommendation prefers.
pub fn installed_model(
    directory: &Path,
    chosen: Option<&str>,
) -> Result<(ModelArtifact, usize), WritingError> {
    let hardware = detect_cpu_inference_hardware();
    let threads = threads_for(&hardware)?;
    let installed: Vec<_> = catalog(ModelUseCase::Writing)
        .iter()
        .filter(|model| directory.join("models").join(model.filename).is_file())
        .map(|model| model.filename)
        .collect();
    let model = select_installed_writing_model(&hardware, &installed, chosen)
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

#[derive(Clone, Copy)]
pub(crate) struct CompletionPlan<'a> {
    pub(crate) prompt: &'a str,
    pub(crate) echo: Option<&'a str>,
    pub(crate) max_tokens: u16,
    /// Whether generation may write digits: only when the typed text has
    /// one, since [`introduces_unsupported_fact`] refuses any other number.
    pub(crate) digits: bool,
    /// Set when the echo is a typed lexicon word that may be half written
    /// (`rep`) or finished (`the`): the unhealed prompt, sent instead when the
    /// healed answer's first token does not extend that word
    /// ([`CompletionPlan::finished_word`]).
    pub(crate) finished_word_prompt: Option<&'a str>,
}

/// Languages whose trailing ASCII space is healed in production. A language is
/// listed only while blinded per-language review finds no increase in harmful
/// suggestions over the unhealed prompt; an unlisted language keeps that
/// prompt, trailing space included. Removing a language is its rollback.
/// German is unlisted: the sealed 2026-09-10 set rose from 1 to 9 harmful.
/// With Gemma 3 1B (2026-10-18, 67 German development and boundary requests)
/// healing German gave 15 useful against 7 and 3 harmful against 2.
/// English was kept by user decision at +1 harmful for +19 useful in 40.
pub(crate) const TRAILING_SPACE_HEALING: &[WritingLanguage] =
    &[WritingLanguage::English, WritingLanguage::Persian];

/// The languages healed by default for the catalog model whose weights hash
/// to `model_sha256` (its `healed_languages`); any other model gets
/// [`TRAILING_SPACE_HEALING`].
#[must_use]
pub fn trailing_space_healing(model_sha256: &str) -> Vec<WritingLanguage> {
    catalog(ModelUseCase::Writing)
        .iter()
        .find(|model| model.sha256 == model_sha256)
        .map_or_else(
            || TRAILING_SPACE_HEALING.to_vec(),
            |model| {
                model
                    .healed_languages
                    .iter()
                    .filter_map(|tag| WritingLanguage::from_tag(tag))
                    .collect()
            },
        )
}

/// The person's language choices from settings, as the writing provider
/// applies them: which languages get suggestions, and per-language healing
/// choices over the model's reviewed default.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LanguagePreferences {
    pub enabled: Vec<WritingLanguage>,
    pub healing: Vec<(WritingLanguage, bool)>,
}

impl Default for LanguagePreferences {
    /// What settings without either field mean: English and German, each
    /// healed as the model's review decided.
    fn default() -> Self {
        Self {
            enabled: vec![WritingLanguage::English, WritingLanguage::German],
            healing: Vec::new(),
        }
    }
}

impl LanguagePreferences {
    /// Every language the provider writes, each at the model default: what
    /// the Prediction Lab measures.
    #[must_use]
    pub fn all() -> Self {
        Self {
            enabled: vec![
                WritingLanguage::English,
                WritingLanguage::German,
                WritingLanguage::Persian,
                WritingLanguage::French,
                WritingLanguage::Spanish,
                WritingLanguage::Portuguese,
            ],
            healing: Vec::new(),
        }
    }

    /// Whether suggestions are on for `language`.
    #[must_use]
    pub fn enables(&self, language: WritingLanguage) -> bool {
        self.enabled.contains(&language)
    }

    /// `model_default` with each per-language choice applied.
    #[must_use]
    pub fn healed(&self, model_default: &[WritingLanguage]) -> Vec<WritingLanguage> {
        let mut healed: Vec<_> = model_default
            .iter()
            .copied()
            .filter(|language| !self.healing.contains(&(*language, false)))
            .collect();
        for (language, heal) in &self.healing {
            if *heal && !healed.contains(language) {
                healed.push(*language);
            }
        }
        healed
    }
}

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
    let (prompt, echo, max_tokens, checked) = healed_boundary(context, language, healed_languages);
    CompletionPlan {
        prompt,
        echo,
        max_tokens,
        digits: request.before.chars().any(|c| ascii_digit(c).is_some()),
        finished_word_prompt: checked.then_some(context),
    }
}

/// The production echo of `before`: separating spaces plus, for English, the
/// last healable word. `None` when this language is not healed, the prompt
/// would be empty, or the word is a lexicon word that production heals only
/// after checking the answer.
#[must_use]
pub fn production_healed_echo<'a>(before: &'a str, language_tag: &str) -> Option<&'a str> {
    let language = WritingLanguage::from_tag(language_tag);
    let (_, echo, _, checked) = healed_boundary(before, language, TRAILING_SPACE_HEALING);
    echo.filter(|_| !checked)
}

/// The prompt, the echo the grammar reproduces, the token budget, and
/// whether the echo is a lexicon word whose answer must extend it.
fn healed_boundary<'a>(
    context: &'a str,
    language: Option<WritingLanguage>,
    healed_languages: &[WritingLanguage],
) -> (&'a str, Option<&'a str>, u16, bool) {
    // A prompt that ends in a space tokenizes unlike the model's own " word"
    // tokens. The echo therefore owns every separating space and, for English,
    // the last healable word, whether or not the user already typed a space
    // after it.
    let trimmed = context.trim_end_matches(' ');
    let heal_english_words = language == Some(WritingLanguage::English)
        && healed_languages.contains(&WritingLanguage::English);
    if heal_english_words {
        if let Some(word) = healing_prefix(trimmed) {
            let (start, end) = healed_echo_span(context, word);
            if start > 0 {
                return (
                    &context[..start],
                    Some(&context[start..end]),
                    token_budget(language, 12),
                    false,
                );
            }
            // The whole field is that word. A trailing space falls through and
            // is healed with the word kept. With nothing else, an empty prompt
            // abstains instead of sending the bare stem.
            if context.len() == trimmed.len() {
                return ("", None, token_budget(language, 8), false);
            }
        }
        // A lexicon word at the caret may still be half typed: unhealed, the
        // model took `rep` and `sh` as finished and wrote ` to your email` or
        // `uck off`. Heal it as well, and keep the answer only when its first
        // token writes the whole word and more, or exactly the word.
        if context.len() == trimmed.len() {
            if let Some(word) = typed_lexicon_word(context) {
                let (start, end) = healed_echo_span(context, word);
                if start > 0 {
                    return (
                        &context[..start],
                        Some(&context[start..end]),
                        token_budget(language, 12),
                        true,
                    );
                }
            }
        }
    }
    if language.is_some_and(|language| healed_languages.contains(&language))
        && context.len() != trimmed.len()
    {
        return (
            trimmed,
            Some(&context[trimmed.len()..]),
            token_budget(language, 8),
            false,
        );
    }
    (context, None, token_budget(language, 8), false)
}

/// The echo of `word` inside `context`, including the ASCII spaces that
/// separate it from the prompt and any spaces the user already typed after it.
fn healed_echo_span(context: &str, word: &str) -> (usize, usize) {
    let word_start = context.trim_end_matches(' ').len() - word.len();
    let mut start = word_start;
    while start > 0 && context.as_bytes()[start - 1] == b' ' {
        start -= 1;
    }
    (start, context.len())
}

/// Tokens requested for one automatic continuation.
///
/// Persian was measured at 2.6× (8 → 21) on 2026-10-12, Lab
/// `production_boundary`, the 13 Persian development cases, pinned 1.7B.
/// All 13 missed 550 ms at 21 tokens and four returned no word. Eight tokens
/// missed twice and still returned a word for every case 21 had answered.
/// One of those words was shorter (`نظر من` became `نظر`). Neither arm
/// matched a reference first word. This is that development comparison, not
/// a held-out score.
fn token_budget(_language: Option<WritingLanguage>, english_tokens: u16) -> u16 {
    english_tokens
}

impl CompletionPlan<'_> {
    /// Whether this plan abstains before any model call: an empty prompt, or
    /// an unhealed one that ends in an ASCII space. Healed languages never
    /// leave that space in the prompt. After one, a model must start a word
    /// without its usual leading-space token: German showed 0 useful of 20
    /// such cases with Qwen3 1.7B (2026-09-09 boundary set) and 0 of 26 with
    /// Gemma 3 1B, which wrote markup (`<strong>Bezug`) or, with markup
    /// excluded, fragments such as `uralt.` (2026-10-18).
    pub(crate) fn sends_nothing(&self) -> bool {
        self.prompt.trim().is_empty() || self.prompt.ends_with(' ')
    }

    /// The plan that takes the typed lexicon word as finished, for a healed
    /// answer whose first token does not write the whole word: it spelled out
    /// something else from pieces (`and` became `andash`, `contains` became
    /// `containsKey`). `None` when the answer may stand, because its first
    /// token is the echo or extends it, or when the echo was no lexicon word.
    pub(crate) fn finished_word(&self, first_token: Option<&str>) -> Option<Self> {
        let prompt = self.finished_word_prompt?;
        let echo = self.echo?;
        if first_token.is_some_and(|token| token.starts_with(echo)) {
            return None;
        }
        Some(Self {
            prompt,
            echo: None,
            max_tokens: token_budget(Some(WritingLanguage::English), 8),
            digits: self.digits,
            finished_word_prompt: None,
        })
    }

    pub(crate) fn payload(&self) -> Value {
        // Per-token timings put the prompt/cache counts on the first token
        // already, so a call the writing budget cuts still reports how much
        // of its prompt came from the runtime's prefix cache.
        let mut payload = json!({"prompt":self.prompt,"n_predict":self.max_tokens,
            "temperature":0.0,"seed":42,"stop":[".","\n"],"stream":true,"cache_prompt":true,
            "timings_per_token":true});
        // Generate only what could be shown. Markup, backticks and line
        // breaks fail `complete_word_prefix`, and a number the typed text
        // lacks fails `introduces_unsupported_fact`; left to the model, the
        // whole suggestion was lost (Gemma 3 1B wrote `<strong>Bezug` and
        // `town of 1000`), so the grammar excludes them and lets the next
        // best token through.
        let class = if self.digits {
            "[^<>\\n\\r`]"
        } else {
            "[^<>\\n\\r`0-9\u{660}-\u{669}\u{6f0}-\u{6f9}]"
        };
        // With an echo, generate at the earlier boundary, reproduce the exact
        // removed bytes, then let the streaming reader remove only that echo.
        let echo = self.echo.map_or_else(String::new, |echo| {
            serde_json::to_string(echo).expect("echo is serializable") + " "
        });
        payload["grammar"] = Value::String(format!("root ::= {echo}{class}*"));
        // Before a typed lexicon word, a lone space token let the model meet
        // the echo without writing the word (` ` then `finger` for ` fin`).
        // Forbidden, the first token writes it (` final`): on the 134 stem
        // cases of 2026-10-23, 124 first tokens stood against 91.
        if self.finished_word_prompt.is_some() {
            payload["logit_bias"] = json!([[" ", false]]);
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

/// Scalars of context a prompt keeps before the cut starts moving.
const CONTEXT_WINDOW_SCALARS: usize = 160;
/// The cut advances in steps of this many scalars, never per keystroke:
/// while a sentence grows within one step, the window start stays fixed and
/// each request's prompt extends the previous one, so the runtime reuses
/// its prefix cache. A per-request sliding start instead changed the
/// prompt's first bytes every time and forced a full re-prefill (measured
/// 2026-10-17: cache reuse 0 of ~30 prompt tokens, 380–510 ms to the first
/// token, once a single sentence passed 160 scalars).
const CONTEXT_CUT_STEP_SCALARS: usize = 64;

/// Logit bias for the first token of each context word: enough to tip a
/// near tie toward a word already in the text, not to force it.
pub(crate) const CONTEXT_BIAS: f64 = 2.0;

/// A continuation is shown only when every token of the shown text has at
/// least this pre-sampling log-probability, so the whole suggestion must be
/// generated before it appears. Calibrated on Gemma 3 1B's blind-reviewed
/// suggestions; on held-out confirmation outputs it removed 5 of 15 harmful
/// and 8 of 18 unhelpful suggestions at the cost of 7 of 51 useful ones
/// (2026-10-20, user decision).
pub(crate) const CONFIDENCE_FLOOR: f64 = -2.75;
const BIAS_MIN_SCALARS: usize = 5;
const BIAS_MAX_WORDS: usize = 24;
/// The words just typed are left out, so the bias never invites a repeat.
const BIAS_RECENT_WORDS: usize = 6;
/// Common English function words long enough to pass the length floor.
const BIAS_STOP_WORDS: &[&str] = &[
    "about", "after", "again", "always", "another", "because", "before", "being", "could", "every",
    "first", "maybe", "never", "other", "really", "should", "since", "still", "their", "there",
    "these", "thing", "things", "think", "those", "through", "under", "until", "where", "which",
    "while", "would",
];

/// Distinctive words of `window`, the bounded text around the field, that a
/// continuation may reuse. Words are letters only and at least five scalars,
/// so names and topic words count and function words do not; words among
/// the last few typed are left out. English only: Persian words often begin
/// with a one-letter token, and biasing those wrote `ب ب ب` in the Lab
/// (2026-10-17). The field's own earlier text measured neutral as a source.
pub(crate) fn bias_words<'a>(
    before: &str,
    window: &'a str,
    language: Option<&str>,
) -> Vec<&'a str> {
    if language.and_then(WritingLanguage::from_tag) != Some(WritingLanguage::English) {
        return Vec::new();
    }
    let earlier: Vec<&str> = before.split_whitespace().collect();
    let recent_tail = &earlier[earlier.len().saturating_sub(BIAS_RECENT_WORDS)..];
    let words = window
        .split(|character: char| !character.is_alphabetic())
        .filter(|word| word.chars().count() >= BIAS_MIN_SCALARS)
        .filter(|word| !BIAS_STOP_WORDS.contains(&word.to_lowercase().as_str()));
    let mut chosen: Vec<&str> = Vec::new();
    for word in words {
        if chosen.len() == BIAS_MAX_WORDS {
            break;
        }
        if !chosen.contains(&word) && !recent_tail.iter().any(|tail| tail.contains(word)) {
            chosen.push(word);
        }
    }
    chosen
}

/// Limit cold prefill cost without changing the context used for edit authority.
pub(crate) fn inference_context(before: &str) -> &str {
    let excess = before
        .chars()
        .count()
        .saturating_sub(CONTEXT_WINDOW_SCALARS);
    let cut_scalars = excess / CONTEXT_CUT_STEP_SCALARS * CONTEXT_CUT_STEP_SCALARS;
    let start = before
        .char_indices()
        .nth(cut_scalars)
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

/// The trailing word of `before` that is a lexicon word but also begins a
/// longer one, so it may be half typed: two to 24 ASCII letters, such as
/// `rep`, `fin` or `the`.
fn typed_lexicon_word(before: &str) -> Option<&str> {
    let word = before.split_whitespace().next_back()?;
    (before.ends_with(word)
        && (2..=24).contains(&word.len())
        && known_english_word(word)
        && begins_longer_english_word(word))
    .then_some(word)
}

fn begins_longer_english_word(word: &str) -> bool {
    let word = word.to_ascii_lowercase();
    // The first lexicon word above `word` in byte order is one of its
    // extensions whenever one exists.
    first_english_word_from(&format!("{word}\0")).is_some_and(|next| next.starts_with(&word))
}

pub(crate) fn strip_healed_prefix<'a>(raw: &'a str, echo: Option<&str>) -> Option<&'a str> {
    match echo {
        Some(echo) => raw.strip_prefix(echo),
        None => Some(raw),
    }
}

/// The complete words of a stream cut short, at most `limit` of them.
pub(crate) fn available_complete_words(raw: &str, limit: usize) -> Option<String> {
    let separator = raw.rfind(' ').unwrap_or(0);
    let count = spaced_words(raw)
        .filter(|(run_end, _)| *run_end <= separator)
        .count()
        .min(limit);
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
    // The first character stays unchanged, as `valid_correction` requires.
    // At most 1,298 short candidates are checked for a 24-byte word.
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
mod tests;
