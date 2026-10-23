//! Opt-in, memory-only prediction experiments. This is not an adapter authority
//! surface: inputs are explicitly supplied and results never edit a document.
//!
//! The Lab is its own crate on the broker's public API. The broker never
//! depends on it, so no experiment can change what the broker ships.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod artifact;
pub mod context_lookup;
mod field_context;
mod midline;
pub mod paced_probe;
pub mod prefill_probe;
pub mod qualification;
pub mod revision;
pub mod spelling;
pub mod stop_token_probe;
mod transport;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use badi_broker::model_selection::current_memory;
use badi_broker::provider::{CompletionProvider, ProviderRequest};
use badi_broker::semantic::client::ClientError;
use badi_broker::semantic::provenance::verify_file;
use badi_broker::semantic::runtime::{
    LlamaCppLaunch, OwnedRuntime, RuntimeError, StableRuntimeIdentity, WritingProfile,
};
use badi_broker::writing::{self, SuggestionLength, WritingError, WritingLanguage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use transport::{LabObservation, LabStream, RequestTrace, TokenLogprob};

pub const REQUEST_SCHEMA: &str = "badi.prediction-lab.request.v1";
pub const LAUNCH_CONTRACT: &str = "badi.prediction-lab.owned-2048.v2";
pub const CONTEXT_TOKENS: u16 = 2048;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

/// Languages admitted for experiments, independently of production support.
#[must_use]
pub fn supported_language(tag: &str) -> bool {
    matches!(
        tag.split('-')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "en" | "de" | "fr" | "es" | "fa" | "pt"
    )
}

fn accepts_language_output(tag: &str, text: &str) -> bool {
    let language = WritingLanguage::from_tag(tag).or_else(|| {
        // French and Spanish experiments use the same Latin script boundary.
        // This is a shape check, not evidence of correct language or quality.
        supported_language(tag).then_some(WritingLanguage::English)
    });
    language.is_some_and(|language| language.accepts_output(text))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let mut hex = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        write!(hex, "{byte:02x}").expect("writing to a String cannot fail");
    }
    hex
}

/// Explicit worker-launch experiment. Browser requests cannot select runtime
/// settings; omission preserves the installed writing batch configuration.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum PrefillBatch {
    #[default]
    Tokens16,
    Tokens64,
}

impl PrefillBatch {
    #[must_use]
    pub const fn tokens(self) -> u16 {
        match self {
            Self::Tokens16 => 16,
            Self::Tokens64 => 64,
        }
    }

    pub fn parse(value: &std::ffi::OsStr) -> Result<Self, &'static str> {
        match value.to_str() {
            Some("16") => Ok(Self::Tokens16),
            Some("64") => Ok(Self::Tokens64),
            _ => Err("invalid_arguments"),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The installed provider, exactly as production runs it.
    ProductionBoundary,
    Context,
    ContextConfidence,
    Instructed,
    Healed,
    InstructedHealed,
    NativeInstructed,
    /// Lab-only: production healing and rejoin, with the constant frame in front.
    Framed,
    /// Lab-only: earlier text of the same field, cut on a stable character grid.
    FieldContext,
    /// Lab-only: force the next one or two words, then keep a prefix only when
    /// those words are not unlikely. The floor is uncalibrated.
    ProbabilityRejoin,
    /// Lab-only: name the line after the caret, then continue from the text
    /// before it. Production prompts do not include that line.
    FollowingText,
    /// The production provider with the case's context as window text: its
    /// words bias the continuation's first tokens; the prompt is unchanged.
    ContextBias,
    /// The production provider with context on: the case's context gives the
    /// window's words and its style examples the earlier writing's, and a
    /// unique word starting with the typed letters is finished before any
    /// model runs.
    Vocabulary,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub budget_ms: u64,
    pub max_tokens: u16,
    pub cache_prompt: bool,
    pub temperature: f32,
    pub seed: u32,
    /// The production suggestion length, for `production_boundary`; other
    /// modes keep the normal one. Omitted when normal.
    #[serde(default, skip_serializing_if = "SuggestionLength::is_normal")]
    pub length: SuggestionLength,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub id: String,
    pub before: String,
    /// Text after the caret. Empty for end-of-text cases. Production boundary
    /// passes it to the provider so mid-line insertion can be measured.
    #[serde(default)]
    pub after: String,
    pub language: String,
    pub context: String,
    pub style_examples: Vec<String>,
    pub config: Config,
}

#[derive(Debug, thiserror::Error)]
pub enum LabError {
    #[error("invalid_request")]
    InvalidRequest,
    #[error("unsupported_language")]
    UnsupportedLanguage,
    #[error("context_overflow")]
    ContextOverflow,
    #[error("runtime_unavailable")]
    RuntimeUnavailable,
    #[error("cancelled")]
    Cancelled,
    #[error("malformed_runtime_response")]
    MalformedResponse,
}

impl From<ClientError> for LabError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Cancelled => Self::Cancelled,
            ClientError::MalformedStream
            | ClientError::UnexpectedContentType
            | ClientError::ResponseTooLarge => Self::MalformedResponse,
            _ => Self::RuntimeUnavailable,
        }
    }
}

#[derive(Debug, Serialize)]
// These are independent measured facts in the public JSON record, not states.
#[allow(clippy::struct_excessive_bools)]
pub struct ResultRecord {
    pub r#type: &'static str,
    pub id: String,
    pub outcome: &'static str,
    pub reason: &'static str,
    pub text: Option<String>,
    pub raw: Option<String>,
    pub effective_prompt: Option<String>,
    pub model_requests: Vec<Value>,
    pub latency_ms: f64,
    pub inference_ms: Option<f64>,
    pub preflight_ms: Option<f64>,
    pub ttft_ms: Option<f64>,
    pub first_word_ms: Option<f64>,
    pub first_four_words_ms: Option<f64>,
    pub candidate_mean_token_logprob: Option<f64>,
    pub candidate_logprob_token_count: Option<usize>,
    pub shape_valid: bool,
    pub within_production_budget: bool,
    pub word_complete: bool,
    pub replace_before: Option<String>,
    pub config: Config,
    pub identity: StableRuntimeIdentity,
    pub token_count: Option<usize>,
    pub tokens_predicted: Option<u64>,
    pub tokens_evaluated: Option<u64>,
    pub slot_tokens_cached: Option<u64>,
    pub reused_prompt_tokens: Option<u64>,
    pub newly_evaluated_prompt_tokens: Option<u64>,
    /// Transport completion, distinct from a naturally completed final word.
    pub terminal_received: Option<bool>,
    pub used_context: bool,
    pub used_style: bool,
    pub warnings: Vec<&'static str>,
}

pub async fn activate(directory: PathBuf) -> Result<OwnedRuntime, LabError> {
    activate_with_artifact(directory, None).await
}

pub async fn activate_with_artifact(
    directory: PathBuf,
    artifact: Option<artifact::ModelArtifactOverride>,
) -> Result<OwnedRuntime, LabError> {
    activate_with_options(directory, artifact, PrefillBatch::default(), true).await
}

pub async fn activate_with_options(
    directory: PathBuf,
    artifact: Option<artifact::ModelArtifactOverride>,
    prefill_batch: PrefillBatch,
    memory_map: bool,
) -> Result<OwnedRuntime, LabError> {
    let explicit_artifact = artifact.is_some();
    let launch = match artifact {
        Some(artifact) => artifact_launch(directory, artifact).await,
        None => installed_launch(directory).await,
    }
    .map_err(|_| LabError::RuntimeUnavailable)?;
    let runtime = configure_launch(launch, prefill_batch, explicit_artifact, memory_map)
        .map_err(|_| LabError::RuntimeUnavailable)?
        .spawn()
        .await
        .map_err(|_| LabError::RuntimeUnavailable)?;
    // The Lab measures every language the provider writes, each healed as
    // the model's review decided; the person's settings never reach it.
    // `BADI_LAB_HEALING=all` heals every language instead, so models outside
    // the catalog are screened on equal terms.
    let mut preferences = writing::LanguagePreferences::all();
    if std::env::var("BADI_LAB_HEALING").is_ok_and(|value| value == "all") {
        preferences.healing = preferences
            .enabled
            .iter()
            .map(|language| (*language, true))
            .collect();
    }
    runtime.set_language_preferences(&preferences);
    Ok(runtime)
}

/// The installed model's launch. Unlike the broker, the Lab reports a host
/// that is short of memory right now instead of waiting for it.
async fn installed_launch(directory: PathBuf) -> Result<LlamaCppLaunch, WritingError> {
    let (model, threads) = writing::installed_model(&directory, None)?;
    if !writing::memory_fits(model, current_memory().available_mib) {
        return Err(WritingError::NoFit);
    }
    writing::verify_launch(directory, model, threads).await
}

/// An explicitly selected model artifact on the pinned runtime.
async fn artifact_launch(
    directory: PathBuf,
    artifact: artifact::ModelArtifactOverride,
) -> Result<LlamaCppLaunch, WritingError> {
    let threads = writing::writing_threads()?;
    tokio::task::spawn_blocking(move || {
        let weights = verify_file(&artifact.weights)?;
        writing::pinned_runtime_launch(&directory, weights, &artifact.alias, threads)
    })
    .await
    .map_err(|_| WritingError::VerificationTask)?
}

/// Applies the Lab's runtime settings to a verified launch: its own launch
/// contract, context window and prompt batch, and containment, so the runtime
/// cannot outlive this worker.
fn configure_launch(
    launch: LlamaCppLaunch,
    prefill_batch: PrefillBatch,
    explicit_artifact: bool,
    memory_map: bool,
) -> Result<LlamaCppLaunch, RuntimeError> {
    let profile = WritingProfile {
        launch_contract_id: LAUNCH_CONTRACT,
        context_size: CONTEXT_TOKENS,
        batch_size: prefill_batch.tokens(),
        // Bounded model comparisons own one active slot. Recurrent model
        // checkpoint copies must not inherit the runtime's default of 32.
        context_checkpoints: Some(0),
        memory_map,
        // As production, so Lab timings of sliding-window models (Gemma 3)
        // include the prompt reuse that ships.
        full_swa_cache: WritingProfile::PRODUCTION.full_swa_cache,
        model_origin: explicit_artifact.then_some("explicit_lab_artifact"),
    };
    Ok(launch.with_writing_profile(profile)?.contained())
}

fn valid_text(text: &str, limit: usize) -> bool {
    text.chars().count() <= limit && !text.contains("<|")
        && !text.chars().any(|ch| {
            (ch.is_control() && !matches!(ch, '\n' | '\t'))
                || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}' | '\u{feff}')
        })
        && badi_broker::segment::valid_orthographic_joiners(text)
}

impl Request {
    pub fn validate(&self) -> Result<(), LabError> {
        let config = &self.config;
        if self.schema != REQUEST_SCHEMA
            || self.id.is_empty()
            || self.id.len() > 128
            || self.id.chars().any(char::is_control)
            || self.before.trim().is_empty()
            || !valid_text(&self.before, 2048)
            || !valid_text(&self.after, 128)
            || !valid_text(&self.context, 4096)
            || self.style_examples.len() > 8
            || self
                .style_examples
                .iter()
                .any(|text| text.trim().is_empty() || !valid_text(text, 1024))
            || !(550..=10_000).contains(&config.budget_ms)
            || !(8..=64).contains(&config.max_tokens)
            || !config.temperature.is_finite()
            || !(0.0..=1.0).contains(&config.temperature)
            || config.seed > 2_147_483_647
            || !(2..=35).contains(&self.language.len())
            || !self
                .language
                .split('-')
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric()))
        {
            return Err(LabError::InvalidRequest);
        }
        if !supported_language(&self.language) {
            return Err(LabError::UnsupportedLanguage);
        }
        if config.mode == Mode::ProductionBoundary && self.before.chars().count() > 512 {
            return Err(LabError::InvalidRequest);
        }
        if config.mode != Mode::ProductionBoundary && !config.length.is_normal() {
            return Err(LabError::InvalidRequest);
        }
        Ok(())
    }

    fn provider_request(&self) -> ProviderRequest {
        ProviderRequest {
            before: self.before.clone(),
            after: self.after.clone(),
            language: Some(self.language.clone()),
            vocabulary: None,
        }
    }

    fn result(&self, identity: &StableRuntimeIdentity) -> ResultRecord {
        let warnings = vec![
            "lab_runtime_2048_differs_from_installed_512",
            "no_application_edit_or_user_acceptance_measured",
        ];
        ResultRecord {
            r#type: "result",
            id: self.id.clone(),
            outcome: "abstention",
            reason: "no_complete_word",
            text: None,
            raw: None,
            effective_prompt: None,
            model_requests: Vec::new(),
            latency_ms: 0.0,
            inference_ms: None,
            preflight_ms: None,
            ttft_ms: None,
            first_word_ms: None,
            first_four_words_ms: None,
            candidate_mean_token_logprob: None,
            candidate_logprob_token_count: None,
            shape_valid: false,
            within_production_budget: false,
            word_complete: false,
            replace_before: None,
            config: self.config.clone(),
            identity: identity.clone(),
            token_count: None,
            tokens_predicted: None,
            tokens_evaluated: None,
            slot_tokens_cached: None,
            reused_prompt_tokens: None,
            newly_evaluated_prompt_tokens: None,
            terminal_received: None,
            used_context: false,
            used_style: false,
            warnings,
        }
    }
}

#[derive(Debug)]
pub struct PreparedPrompt {
    pub payload: Value,
    /// Bytes generated to reproduce a boundary, excluded from the continuation.
    pub echo: String,
}

/// Preserve all supplied text. Token preflight rejects overflow rather than
/// silently dropping contextual cues or examples to fit the runtime.
#[must_use]
pub fn prepare_prompt(request: &Request) -> PreparedPrompt {
    let mut before = request.before.as_str();
    let mut echo = String::new();
    if matches!(
        request.config.mode,
        Mode::Healed | Mode::InstructedHealed | Mode::NativeInstructed
    ) {
        if let Some(boundary) = writing::production_healed_echo(before, &request.language) {
            boundary.clone_into(&mut echo);
            before = &before[..before.len() - boundary.len()];
        }
    }
    let context = context_prefix(&request.context, &request.style_examples);
    let prompt = if matches!(
        request.config.mode,
        Mode::Instructed | Mode::InstructedHealed
    ) {
        // Qwen3 non-thinking ChatML template, with the exact draft prefilled as
        // assistant text so generation continues it instead of answering it.
        format!(
            "<|im_start|>system\nContinue the user's draft in {}. Preserve its language and style. Use the supplied context only when relevant. Do not invent facts. Write only the continuation.\n<|im_end|>\n<|im_start|>user\nContext and writing examples:\n{context}Continue the draft in the assistant message.<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n{before}",
            request.language
        )
    } else {
        format!("{context}{before}")
    };
    let mut payload = json!({"prompt":prompt,"n_predict":request.config.max_tokens,
        "temperature":request.config.temperature,"seed":request.config.seed,
        "stop":["\n","<|im_end|>","<|endoftext|>"],"stream":true,
        "cache_prompt":request.config.cache_prompt});
    if request.config.mode == Mode::ContextConfidence {
        payload["n_probs"] = json!(1);
        payload["post_sampling_probs"] = json!(false);
    }
    if !echo.is_empty() {
        let literal = serde_json::to_string(&echo).expect("string serialization");
        payload["grammar"] = json!(format!("root ::= {literal} [^<>\\n\\r`]*"));
    }
    PreparedPrompt { payload, echo }
}

// The paced primer receives only this stable prefix. Accepting draft text here
// would make future keystrokes available before their scheduled event.
fn context_prefix(context: &str, styles: &[String]) -> String {
    let mut prefix = String::new();
    if !context.is_empty() {
        prefix.push_str(context);
        prefix.push_str("\n\n");
    }
    for example in styles {
        prefix.push_str(example);
        prefix.push_str("\n\n");
    }
    prefix
}

fn native_instruction_messages(request: &Request) -> Value {
    json!([
        {"role":"system","content":format!("Continue the user's unfinished draft in {}. Return only the continuation. Current context facts take precedence over facts in writing examples. Examples describe style only. Do not invent facts.", request.language)},
        {"role":"user","content":format!("Current context:\n{}\n\nWriting examples (style only):\n{}\n\nContinue the draft prefilled in the assistant message.", request.context, request.style_examples.join("\n\n"))}
    ])
}

#[allow(clippy::too_many_lines)]
pub async fn run(
    runtime: &OwnedRuntime,
    request: Request,
    cancellation: CancellationToken,
) -> Result<ResultRecord, LabError> {
    request.validate()?;
    if !runtime.is_alive() {
        return Err(LabError::RuntimeUnavailable);
    }
    let started = Instant::now();
    if matches!(
        request.config.mode,
        Mode::ProductionBoundary | Mode::ContextBias | Mode::Vocabulary
    ) {
        return baseline(runtime, &request, cancellation, started).await;
    }
    if matches!(
        request.config.mode,
        Mode::Framed | Mode::FieldContext | Mode::ProbabilityRejoin | Mode::FollowingText
    ) {
        return lab_rejoin(runtime, &request, cancellation, started).await;
    }
    let mut prepared = prepare_prompt(&request);
    let mut result = request.result(runtime.identity());
    result.used_context = !request.context.is_empty();
    result.used_style = !request.style_examples.is_empty();
    if request.config.mode == Mode::NativeInstructed {
        let messages = native_instruction_messages(&request);
        let template = match transport::apply_template(
            runtime.client(),
            messages,
            request.config.budget_ms,
            cancellation.clone(),
        )
        .await
        {
            Ok(template) => template,
            Err(ClientError::Timeout) => {
                result.outcome = "deadline";
                result.reason = "native_template_deadline";
                result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                result.preflight_ms = Some(result.latency_ms);
                return Ok(result);
            }
            Err(error) => return Err(error.into()),
        };
        let before = request
            .before
            .strip_suffix(&prepared.echo)
            .ok_or(LabError::InvalidRequest)?;
        prepared.payload["prompt"] = json!(format!("{template}{before}"));
        // Native EOS belongs to the selected tokenizer. No Qwen token IDs or
        // literal chat delimiters are imposed on another model family.
        prepared.payload["stop"] = json!(["\n"]);
    }
    let prompt = prepared.payload["prompt"]
        .as_str()
        .ok_or(LabError::InvalidRequest)?;
    result.effective_prompt = Some(prompt.to_owned());
    let count = match transport::token_count(
        runtime.client(),
        prompt,
        request
            .config
            .budget_ms
            .saturating_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
        cancellation.clone(),
    )
    .await
    {
        Ok(count) => count,
        Err(ClientError::Timeout) => {
            result.outcome = "deadline";
            result.reason = "token_preflight_deadline";
            result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
            result.preflight_ms = Some(result.latency_ms);
            return Ok(result);
        }
        Err(error) => return Err(error.into()),
    };
    // One spare token ensures llama.cpp never reaches context shifting.
    if count + usize::from(request.config.max_tokens) + 1 > usize::from(CONTEXT_TOKENS) {
        return Err(LabError::ContextOverflow);
    }
    result.preflight_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
    result.model_requests.push(prepared.payload.clone());
    result.token_count = Some(count);
    let remaining = request
        .config
        .budget_ms
        .saturating_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    let observed = transport::stream(
        runtime.client(),
        prepared.payload,
        remaining,
        cancellation,
        Some(LabObservation {
            before: &request.before,
            language: &request.language,
            echo: &prepared.echo,
            started,
        }),
    )
    .await?;
    apply_observation(&mut result, &request, observed, &prepared.echo);
    result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    result.within_production_budget = result.latency_ms <= 550.0;
    result
        .warnings
        .push("experimental_output_not_qualified_for_production");
    Ok(result)
}

async fn baseline(
    runtime: &OwnedRuntime,
    request: &Request,
    cancellation: CancellationToken,
    started: Instant,
) -> Result<ResultRecord, LabError> {
    let mut result = request.result(runtime.identity());
    result.config = Config {
        mode: request.config.mode,
        budget_ms: 550,
        max_tokens: 8,
        cache_prompt: true,
        temperature: 0.0,
        seed: 42,
        length: request.config.length,
    };
    let bias_context = request.config.mode == Mode::ContextBias;
    let vocabulary = (request.config.mode == Mode::Vocabulary).then(|| {
        badi_broker::vocabulary::Vocabulary::from_texts(&request.context, &request.style_examples)
    });
    if vocabulary.is_none()
        && ((!bias_context && !request.context.is_empty()) || !request.style_examples.is_empty())
    {
        result
            .warnings
            .push("context_and_style_ignored_by_production");
    }
    result.warnings.push("production_raw_and_ttft_not_retained");
    result.used_context = (bias_context || vocabulary.is_some()) && !request.context.is_empty();
    result.used_style = vocabulary.is_some() && !request.style_examples.is_empty();
    // The same rule the provider runs first, so a result says which path answered.
    let finished_word = vocabulary.as_ref().and_then(|vocabulary| {
        badi_broker::writing::WritingLanguage::from_tag(&request.language).and_then(|language| {
            badi_broker::vocabulary::complete(&request.before, &request.after, language, vocabulary)
                .ok()
        })
    });
    let trace = Arc::new(RequestTrace::default());
    let client = runtime
        .client()
        .clone()
        .with_request_observer(Arc::clone(&trace) as _)
        .with_window_context(if bias_context { &request.context } else { "" });
    // Clones share the length, and each request sets its own.
    client.set_suggestion_length(request.config.length);
    let mut provider_request = request.provider_request();
    provider_request.vocabulary = vocabulary;
    let proposal = client
        .propose(provider_request, cancellation, true)
        .await
        .map_err(|error| match error {
            badi_broker::provider::ProviderError::Cancelled => LabError::Cancelled,
            badi_broker::provider::ProviderError::Unavailable => LabError::RuntimeUnavailable,
        })?;
    result.model_requests = trace.requests().ok_or(LabError::RuntimeUnavailable)?;
    result.effective_prompt = result
        .model_requests
        .last()
        .and_then(|payload| payload["prompt"].as_str())
        .map(str::to_owned);
    if let Some(tokens) = result
        .model_requests
        .last()
        .and_then(|payload| payload["n_predict"].as_u64())
    {
        result.config.max_tokens =
            u16::try_from(tokens).map_err(|_| LabError::RuntimeUnavailable)?;
    }
    result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    if let Some(proposal) = proposal {
        result.outcome = "suggestion";
        result.reason = if proposal.replace_before.is_some() {
            "production_correction"
        } else if finished_word
            .as_ref()
            .is_some_and(|completion| completion.suffix == proposal.text)
        {
            "vocabulary_completion"
        } else {
            "production_continuation"
        };
        result.text = Some(proposal.text);
        result.replace_before = proposal.replace_before;
        result.word_complete = true;
        result.shape_valid = true;
    } else {
        result.reason = "production_abstention_or_budget";
    }
    result.within_production_budget = result.latency_ms <= 550.0;
    Ok(result)
}

fn quality_parts(request: &Request) -> (String, String) {
    let echo = writing::production_healed_echo(&request.before, &request.language).unwrap_or("");
    let stem = request.before.strip_suffix(echo).unwrap_or(&request.before);
    let prefix = match request.config.mode {
        Mode::FieldContext => {
            let earlier = field_context::stable_prefix(&request.context);
            if earlier.is_empty() {
                String::new()
            } else {
                format!("{earlier}\n\n")
            }
        }
        Mode::FollowingText => midline::following_text_prefix(&request.after),
        _ => midline::FRAME.to_owned(),
    };
    (format!("{prefix}{stem}"), echo.to_owned())
}

fn lab_completion_payload(
    prompt: &str,
    echo: &str,
    grammar: Option<String>,
    n_predict: u16,
    probs: bool,
) -> Value {
    let mut payload = json!({"prompt":prompt,"n_predict":n_predict,"temperature":0.0,"seed":42,
        "stop":[".","\n"],"stream":true,"cache_prompt":true});
    if probs {
        payload["n_probs"] = json!(1);
        payload["post_sampling_probs"] = json!(false);
    }
    if let Some(grammar) = grammar {
        payload["grammar"] = json!(grammar);
    } else if !echo.is_empty() {
        let literal = serde_json::to_string(echo).expect("echo is serializable");
        payload["grammar"] = json!(format!("root ::= {literal} [^<>\\n\\r`]*"));
    }
    payload
}

struct RejoinJob {
    payload: Value,
    echo: String,
    forced: bool,
}

async fn lab_rejoin(
    runtime: &OwnedRuntime,
    request: &Request,
    cancellation: CancellationToken,
    started: Instant,
) -> Result<ResultRecord, LabError> {
    if request.config.mode == Mode::ProbabilityRejoin {
        return probability_rejoin(runtime, request, cancellation, started).await;
    }
    let mut result = request.result(runtime.identity());
    let lab_only = match request.config.mode {
        Mode::FieldContext => "field_context_is_lab_only",
        Mode::FollowingText => "following_text_prompt_is_lab_only",
        _ => midline::framed_warning(),
    };
    result.warnings.push(lab_only);
    result
        .warnings
        .push("experimental_output_not_qualified_for_production");
    result.used_context = request.config.mode == Mode::FieldContext && !request.context.is_empty();
    result.config.budget_ms = 550;
    result.config.max_tokens = 8;
    result.config.cache_prompt = true;
    result.config.temperature = 0.0;
    result.config.seed = 42;
    let (prompt, echo) = quality_parts(request);
    let payload = lab_completion_payload(&prompt, &echo, None, 8, false);
    finish_lab_rejoin(
        runtime,
        request,
        cancellation,
        started,
        result,
        RejoinJob {
            payload,
            echo,
            forced: false,
        },
    )
    .await
}

async fn probability_rejoin(
    runtime: &OwnedRuntime,
    request: &Request,
    cancellation: CancellationToken,
    started: Instant,
) -> Result<ResultRecord, LabError> {
    let Some(words) = midline::following_words(&request.after) else {
        let mut result = baseline(runtime, request, cancellation, started).await?;
        result.warnings.push("two_word_check_not_applicable");
        return Ok(result);
    };
    let Some(forced) = midline::forced_word_grammar(&words) else {
        return Err(LabError::InvalidRequest);
    };
    let echo = writing::production_healed_echo(&request.before, &request.language).unwrap_or("");
    let stem = request.before.strip_suffix(echo).unwrap_or(&request.before);
    let grammar = if echo.is_empty() {
        forced
    } else {
        let literal = serde_json::to_string(echo).expect("echo is serializable");
        format!(
            "root ::= {literal} {}",
            forced.trim_start_matches("root ::= ")
        )
    };
    let mut result = request.result(runtime.identity());
    result
        .warnings
        .push("rejoin_logprob_floor_minus_2_is_uncalibrated");
    result
        .warnings
        .push("experimental_output_not_qualified_for_production");
    // Room for a short prefix plus the forced words. The deadline stays the
    // request budget, so a longer token cap can still miss 550 ms.
    let tokens = request.config.max_tokens.max(24);
    let payload = lab_completion_payload(stem, echo, Some(grammar), tokens, true);
    finish_lab_rejoin(
        runtime,
        request,
        cancellation,
        started,
        result,
        RejoinJob {
            payload,
            echo: echo.to_owned(),
            forced: true,
        },
    )
    .await
}

async fn finish_lab_rejoin(
    runtime: &OwnedRuntime,
    request: &Request,
    cancellation: CancellationToken,
    started: Instant,
    mut result: ResultRecord,
    job: RejoinJob,
) -> Result<ResultRecord, LabError> {
    result.effective_prompt = job.payload["prompt"].as_str().map(str::to_owned);
    result.model_requests.push(job.payload.clone());
    let budget_ms = if matches!(
        request.config.mode,
        Mode::Framed | Mode::FieldContext | Mode::FollowingText
    ) {
        550
    } else {
        request.config.budget_ms
    };
    let remaining =
        budget_ms.saturating_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
    let observed = transport::stream(
        runtime.client(),
        job.payload,
        remaining,
        cancellation,
        Some(LabObservation {
            before: &request.before,
            language: &request.language,
            echo: &job.echo,
            started,
        }),
    )
    .await?;
    record_observation_metrics(&mut result, &observed);
    let continuation = observed
        .raw
        .strip_prefix(job.echo.as_str())
        .unwrap_or(observed.raw.as_str());
    result.raw = Some(observed.raw.clone());
    let insertion = if job.forced {
        forced_insertion(
            request,
            &observed.raw,
            &job.echo,
            observed.token_logprobs.as_deref(),
        )
    } else {
        midline::rejoin(&request.before, &request.after, continuation)
            .or_else(|| end_of_text_continuation(&request.after, continuation))
    };
    let Some(insertion) = insertion else {
        result.outcome = if observed.deadline {
            "deadline"
        } else {
            "abstention"
        };
        result.reason = if job.forced {
            "forced_words_unlikely"
        } else if observed.deadline {
            "no_complete_word"
        } else {
            "rejoin_rejected"
        };
        result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        result.within_production_budget = result.latency_ms <= 550.0;
        return Ok(result);
    };
    if writing::validate_proposal(
        &request.before,
        &request.after,
        &insertion,
        Some(&request.language),
    )
    .is_err()
        || !accepts_language_output(&request.language, &insertion)
    {
        result.outcome = "abstention";
        result.reason = "output_seam";
    } else {
        result.outcome = "suggestion";
        result.reason = "lab_rejoin";
        result.word_complete = true;
        result.shape_valid = true;
        result.text = Some(insertion);
    }
    result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    result.within_production_budget = result.latency_ms <= 550.0;
    Ok(result)
}

fn end_of_text_continuation(after: &str, continuation: &str) -> Option<String> {
    if writing::line_rest(after).is_some() {
        return None;
    }
    writing::complete_word_prefix(continuation, 4, true)
}

fn forced_insertion(
    request: &Request,
    raw: &str,
    echo: &str,
    tokens: Option<&[TokenLogprob]>,
) -> Option<String> {
    let continuation = raw.strip_prefix(echo).unwrap_or(raw);
    let words = midline::following_words(&request.after)?;
    let prefix = midline::prefix_before_forced(continuation, &words)?;
    let suffix = format!(" {}", words.join(" "));
    let mean = midline::suffix_mean_logprob(raw, &suffix, tokens?)?;
    midline::accept_forced_prefix(prefix, mean)?;
    midline::rejoin(&request.before, &request.after, continuation)
}

fn apply_observation(
    result: &mut ResultRecord,
    request: &Request,
    observed: LabStream,
    echo: &str,
) {
    record_observation_metrics(result, &observed);
    let continuation = observed.raw.strip_prefix(echo);
    let echo_matched = continuation.is_some();
    let candidate = continuation
        .and_then(|text| complete_output(text, observed.stopped))
        .ok_or(if echo_matched {
            "no_complete_word"
        } else {
            "echo_mismatch"
        });
    result.raw = Some(observed.raw);
    if !echo.is_empty() {
        result.warnings.push("ttft_includes_forced_boundary_echo");
    }
    let candidate = match candidate {
        Ok(candidate) => candidate,
        Err(reason) => {
            result.outcome = if observed.deadline {
                "deadline"
            } else {
                "abstention"
            };
            result.reason = reason;
            return;
        }
    };
    let language_valid = accepts_language_output(&request.language, &candidate);
    let seam_valid =
        writing::validate_proposal(&request.before, "", &candidate, Some(&request.language))
            .is_ok();
    if !language_valid || !seam_valid {
        result.outcome = "abstention";
        result.reason = if language_valid {
            "output_seam"
        } else {
            "output_language_or_shape"
        };
        return;
    }
    // A style example can carry a date or number from a different draft. Drop
    // that completed fact unless the current draft or context already states it.
    if style_fact_conflict(&candidate, request) {
        result.outcome = "abstention";
        result.reason = "style_fact_conflict";
        return;
    }
    apply_confidence(
        result,
        request,
        observed.token_logprobs.as_deref(),
        &candidate,
    );
    result.outcome = "suggestion";
    result.reason = if observed.deadline {
        "complete_prefix_at_deadline"
    } else {
        "complete_prefix"
    };
    result.word_complete = true;
    result.shape_valid = candidate.chars().count() <= 64;
    result.text = Some(candidate);
}

fn record_observation_metrics(result: &mut ResultRecord, observed: &LabStream) {
    result.inference_ms = Some(observed.latency_ms);
    result.ttft_ms = observed.ttft_ms;
    result.first_word_ms = observed.first_word_ms;
    result.first_four_words_ms = observed.first_four_words_ms;
    result.tokens_predicted = observed.tokens_predicted;
    result.tokens_evaluated = observed.tokens_evaluated;
    result.slot_tokens_cached = observed.slot_tokens_cached;
    result.reused_prompt_tokens = observed.reused_prompt_tokens;
    result.newly_evaluated_prompt_tokens = observed.newly_evaluated_prompt_tokens;
    result.terminal_received = Some(observed.terminal_received);
    result
        .warnings
        .push("slot_tokens_cached_is_occupancy_not_proven_prompt_reuse");
    result
        .warnings
        .push("word_timing_is_observed_candidate_readiness_not_display_latency");
}

fn apply_confidence(
    result: &mut ResultRecord,
    request: &Request,
    tokens: Option<&[TokenLogprob]>,
    candidate: &str,
) {
    if request.config.mode != Mode::ContextConfidence {
        return;
    }
    result
        .warnings
        .push("candidate_probability_is_not_calibrated_usefulness");
    if let Some((mean, count)) =
        candidate_token_logprob(result.raw.as_deref().unwrap_or_default(), candidate, tokens)
    {
        result.candidate_mean_token_logprob = Some(mean);
        result.candidate_logprob_token_count = Some(count);
    } else {
        result.warnings.push("candidate_probability_unavailable");
    }
}

fn candidate_token_logprob(
    raw: &str,
    candidate: &str,
    tokens: Option<&[TokenLogprob]>,
) -> Option<(f64, usize)> {
    let tokens = tokens?;
    if candidate.is_empty() || !raw.starts_with(candidate) {
        return None;
    }
    let mut generated = Vec::new();
    let mut total = 0.0;
    let mut count = 0;
    let mut byte_count = 0;
    for token in tokens {
        generated.extend_from_slice(&token.bytes);
        if byte_count < candidate.len() {
            byte_count += token.bytes.len();
            if byte_count > candidate.len() {
                return None;
            }
            total += token.logprob;
            count += 1;
        }
    }
    if count == 0 || byte_count != candidate.len() || generated != raw.as_bytes() {
        return None;
    }
    Some((total / f64::from(u16::try_from(count).ok()?), count))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FactRun {
    Digits,
    Letters,
}

fn fact_run(character: char, current: Option<FactRun>) -> Option<FactRun> {
    if writing::ascii_digit(character).is_some() {
        Some(FactRun::Digits)
    } else if character.is_alphabetic()
        || (character == '\u{200c}' && current == Some(FactRun::Letters))
    {
        Some(FactRun::Letters)
    } else {
        None
    }
}

fn is_weekday(token: &str) -> bool {
    const WEEKDAYS: &[&str] = &[
        "monday",
        "tuesday",
        "wednesday",
        "thursday",
        "friday",
        "saturday",
        "sunday",
        "montag",
        "dienstag",
        "mittwoch",
        "donnerstag",
        "freitag",
        "samstag",
        "sonntag",
        "lundi",
        "mardi",
        "mercredi",
        "jeudi",
        "vendredi",
        "samedi",
        "dimanche",
        "lunes",
        "martes",
        "miércoles",
        "jueves",
        "viernes",
        "sábado",
        "domingo",
        "شنبه",
        "یکشنبه",
        "دوشنبه",
        "سهشنبه",
        "چهارشنبه",
        "پنجشنبه",
        "جمعه",
    ];
    WEEKDAYS.contains(&token)
}

fn record_fact_run(
    text: &str,
    kind: FactRun,
    start: usize,
    end: usize,
    found: &mut BTreeSet<String>,
) {
    match kind {
        FactRun::Digits => {
            found.insert(
                text[start..end]
                    .chars()
                    .map(|character| {
                        writing::ascii_digit(character)
                            .expect("numeric fact run contains supported digits")
                    })
                    .collect(),
            );
        }
        FactRun::Letters => {
            let token = text[start..end].replace('\u{200c}', "").to_lowercase();
            if is_weekday(&token) {
                found.insert(token);
            }
        }
    }
}

fn collect_fact_tokens(text: &str, found: &mut BTreeSet<String>) {
    let mut run_start = 0;
    let mut run_kind: Option<FactRun> = None;
    for (index, character) in text.char_indices() {
        let next = fact_run(character, run_kind);
        if next == run_kind {
            continue;
        }
        if let Some(kind) = run_kind {
            record_fact_run(text, kind, run_start, index, found);
        }
        run_kind = next;
        run_start = index;
    }
    if let Some(kind) = run_kind {
        record_fact_run(text, kind, run_start, text.len(), found);
    }
}

fn style_fact_conflict(candidate: &str, request: &Request) -> bool {
    if request.style_examples.is_empty() {
        return false;
    }
    let mut introduced = BTreeSet::new();
    collect_fact_tokens(candidate, &mut introduced);
    if introduced.is_empty() {
        return false;
    }
    let mut style = BTreeSet::new();
    for example in &request.style_examples {
        collect_fact_tokens(example, &mut style);
    }
    let mut current = BTreeSet::new();
    collect_fact_tokens(&request.before, &mut current);
    collect_fact_tokens(&request.context, &mut current);
    introduced
        .iter()
        .any(|token| style.contains(token) && !current.contains(token))
}

fn complete_output(text: &str, natural_stop: bool) -> Option<String> {
    // An EOS/stop boundary proves the final word ended. A budget/token limit
    // does not: keep only words preceding an observed whitespace separator.
    let raw = if natural_stop {
        text
    } else {
        text.rsplit_once(char::is_whitespace)
            .map_or("", |(prefix, _)| prefix)
    };
    writing::complete_word_prefix(raw, 4, true)
}

#[cfg(test)]
mod tests;
