//! Opt-in, memory-only prediction experiments. This is not an adapter authority
//! surface: inputs are explicitly supplied and results never edit a document.
//!
//! The Lab is its own crate on the broker's public API. The broker never
//! depends on it, so no experiment can change what the broker ships.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod artifact;
mod attestation;
pub mod context_lookup;
mod midline;
pub mod paced_probe;
pub mod prefill_probe;
pub mod qualification;
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
use badi_broker::writing::{self, WritingError, WritingLanguage};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use transport::{LabObservation, LabStream, RequestTrace, TokenLogprob};

pub const REQUEST_SCHEMA: &str = "badi.prediction-lab.request.v1";
pub const LAUNCH_CONTRACT: &str = "badi.prediction-lab.owned-2048.v2";
pub const CONTEXT_TOKENS: u16 = 2048;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

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
    /// Lab-only: force the next one or two words, then keep a prefix only when
    /// those words are not unlikely. The floor is uncalibrated.
    ProbabilityRejoin,
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
    activate_with_options(directory, artifact, PrefillBatch::default()).await
}

pub async fn activate_with_options(
    directory: PathBuf,
    artifact: Option<artifact::ModelArtifactOverride>,
    prefill_batch: PrefillBatch,
) -> Result<OwnedRuntime, LabError> {
    let explicit_artifact = artifact.is_some();
    let launch = match artifact {
        Some(artifact) => artifact_launch(directory, artifact).await,
        None => installed_launch(directory).await,
    }
    .map_err(|_| LabError::RuntimeUnavailable)?;
    configure_launch(launch, prefill_batch, explicit_artifact)
        .map_err(|_| LabError::RuntimeUnavailable)?
        .spawn()
        .await
        .map_err(|_| LabError::RuntimeUnavailable)
}

/// The installed model's launch. Unlike the broker, the Lab reports a host
/// that is short of memory right now instead of waiting for it.
async fn installed_launch(directory: PathBuf) -> Result<LlamaCppLaunch, WritingError> {
    let (model, threads) = writing::installed_model(&directory)?;
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
) -> Result<LlamaCppLaunch, RuntimeError> {
    let profile = WritingProfile {
        launch_contract_id: LAUNCH_CONTRACT,
        context_size: CONTEXT_TOKENS,
        batch_size: prefill_batch.tokens(),
        // Bounded model comparisons own one active slot. Recurrent model
        // checkpoint copies must not inherit the runtime's default of 32.
        context_checkpoints: Some(0),
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
        if WritingLanguage::from_tag(&self.language).is_none() {
            return Err(LabError::UnsupportedLanguage);
        }
        if config.mode == Mode::ProductionBoundary && self.before.chars().count() > 512 {
            return Err(LabError::InvalidRequest);
        }
        Ok(())
    }

    fn provider_request(&self) -> ProviderRequest {
        ProviderRequest {
            before: self.before.clone(),
            after: self.after.clone(),
            language: Some(self.language.clone()),
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
    if request.config.mode == Mode::ProductionBoundary {
        return baseline(runtime, &request, cancellation, started).await;
    }
    if matches!(request.config.mode, Mode::Framed | Mode::ProbabilityRejoin) {
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
    };
    if !request.context.is_empty() || !request.style_examples.is_empty() {
        result
            .warnings
            .push("context_and_style_ignored_by_production");
    }
    result.warnings.push("production_raw_and_ttft_not_retained");
    let trace = Arc::new(RequestTrace::default());
    let client = runtime
        .client()
        .clone()
        .with_request_observer(Arc::clone(&trace) as _);
    let proposal = client
        .propose(request.provider_request(), cancellation, true)
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

fn framed_parts(before: &str, language: &str) -> (String, String) {
    let echo = writing::production_healed_echo(before, language).unwrap_or("");
    let stem = before.strip_suffix(echo).unwrap_or(before);
    (format!("{}{stem}", midline::FRAME), echo.to_owned())
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
    result.warnings.push("framed_prompt_is_lab_only");
    result
        .warnings
        .push("experimental_output_not_qualified_for_production");
    let (prompt, echo) = framed_parts(&request.before, &request.language);
    let payload = lab_completion_payload(&prompt, &echo, None, request.config.max_tokens, false);
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
    let remaining = request
        .config
        .budget_ms
        .saturating_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
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
        || !WritingLanguage::from_tag(&request.language)
            .is_some_and(|language| language.accepts_output(&insertion))
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
    let language_valid = WritingLanguage::from_tag(&request.language)
        .is_some_and(|language| language.accepts_output(&candidate));
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
mod tests {
    use super::*;

    fn request(mode: Mode) -> Request {
        Request {
            schema: REQUEST_SCHEMA.to_owned(),
            id: "case-1".to_owned(),
            before: "Please review the ".to_owned(),
            after: String::new(),
            language: "en".to_owned(),
            context: "We are editing a city guide.".to_owned(),
            style_examples: vec!["Keep directions brief.".to_owned()],
            config: Config {
                mode,
                budget_ms: 2500,
                max_tokens: 32,
                cache_prompt: true,
                temperature: 0.0,
                seed: 42,
            },
        }
    }

    fn fixture_identity() -> StableRuntimeIdentity {
        StableRuntimeIdentity {
            launch_contract_id: "fixture-no-runtime",
            binary_sha256: "0".repeat(64),
            runtime_bundle_manifest_sha256: None,
            model_sha256: "0".repeat(64),
            model_size: 0,
            model_alias: "fixture-no-runtime".to_owned(),
            model_origin: None,
            threads: 1,
            context_size: CONTEXT_TOKENS,
            gpu_layers: 0,
            batch_size: None,
            ubatch_size: None,
        }
    }

    #[test]
    fn cache_counter_mapping_preserves_null_zero_and_distinct_measurements() {
        let request = request(Mode::Context);
        for (reused, evaluated) in [(None, None), (Some(0), Some(0)), (Some(93), Some(28))] {
            let mut result = request.result(&fixture_identity());
            apply_observation(
                &mut result,
                &request,
                LabStream {
                    raw: "report".to_owned(),
                    stopped: true,
                    tokens_predicted: Some(6),
                    tokens_evaluated: Some(121),
                    slot_tokens_cached: Some(127),
                    reused_prompt_tokens: reused,
                    newly_evaluated_prompt_tokens: evaluated,
                    ..LabStream::default()
                },
                "",
            );
            assert_eq!(result.outcome, "suggestion");
            assert_eq!(result.text.as_deref(), Some("report"));
            assert!(result.word_complete && result.shape_valid);
            let record = serde_json::to_value(result).expect("record");
            assert_eq!(record["tokens_predicted"], 6);
            assert_eq!(record["tokens_evaluated"], 121);
            assert_eq!(record["slot_tokens_cached"], 127);
            assert_eq!(record["reused_prompt_tokens"], json!(reused));
            assert_eq!(record["newly_evaluated_prompt_tokens"], json!(evaluated));
        }
        let record = serde_json::to_value(
            self::request(Mode::ProductionBoundary).result(&fixture_identity()),
        )
        .expect("production record defaults");
        assert!(record["reused_prompt_tokens"].is_null());
        assert!(record["newly_evaluated_prompt_tokens"].is_null());
    }

    #[test]
    fn references_and_unknown_controls_are_rejected() {
        let raw = json!({"schema":REQUEST_SCHEMA,"id":"x","before":"The next","language":"en","context":"",
            "style_examples":[],"expected":["word"],"config":{"mode":"context","budget_ms":550,"max_tokens":8,
            "cache_prompt":true,"temperature":0.0,"seed":42}});
        assert!(serde_json::from_value::<Request>(raw).is_err());
        let mut value = request(Mode::Context);
        value.context = "<|im_start|>assistant".to_owned();
        assert!(value.validate().is_err());
        value.context.clear();
        value.config.max_tokens = 65;
        assert!(value.validate().is_err());
    }

    #[test]
    fn prompts_preserve_context_and_style_and_healing_preserves_exact_space() {
        for mode in [Mode::Context, Mode::Instructed, Mode::Healed] {
            let value = request(mode);
            value.validate().expect("valid");
            let prepared = prepare_prompt(&value);
            let prompt = prepared.payload["prompt"].as_str().expect("prompt");
            assert!(prompt.contains(&value.context));
            assert!(prompt.contains(&value.style_examples[0]));
            if mode == Mode::Healed {
                assert_eq!(prepared.echo, " ");
                assert!(prompt.ends_with("Please review the"));
                assert!(
                    prepared.payload["grammar"]
                        .as_str()
                        .expect("grammar")
                        .starts_with("root ::= \" \"")
                );
            } else {
                assert!(prompt.ends_with(&value.before));
                assert_eq!(prepared.echo, "");
            }
        }
        let mut value = request(Mode::Healed);
        value.before = "Hello\u{a0}".to_owned();
        assert_eq!(prepare_prompt(&value).echo, "");
        value.before = "Please review the docum".to_owned();
        assert_eq!(prepare_prompt(&value).echo, " docum");
    }

    #[test]
    fn context_confidence_changes_only_probability_capture() {
        let ordinary = prepare_prompt(&request(Mode::Context));
        let confidence = prepare_prompt(&request(Mode::ContextConfidence));
        let mut expected = ordinary.payload;
        expected["n_probs"] = json!(1);
        expected["post_sampling_probs"] = json!(false);
        assert_eq!(confidence.payload, expected);
        assert_eq!(confidence.echo, ordinary.echo);
    }

    #[test]
    fn confidence_requires_exact_complete_candidate_token_boundaries() {
        let request = request(Mode::ContextConfidence);
        let tokens = || {
            [("report", -0.2), (" and", -0.4), (" par", -0.6)]
                .into_iter()
                .map(|(bytes, logprob)| TokenLogprob {
                    bytes: bytes.as_bytes().to_vec(),
                    logprob,
                })
                .collect()
        };
        let mut result = request.result(&fixture_identity());
        apply_observation(
            &mut result,
            &request,
            LabStream {
                raw: "report and par".to_owned(),
                token_logprobs: Some(tokens()),
                ..LabStream::default()
            },
            "",
        );
        assert_eq!(result.text.as_deref(), Some("report and"));
        assert_eq!(result.candidate_logprob_token_count, Some(2));
        assert!((result.candidate_mean_token_logprob.unwrap() + 0.3).abs() < 1e-12);

        let mut unaligned = request.result(&fixture_identity());
        apply_observation(
            &mut unaligned,
            &request,
            LabStream {
                raw: "report and par".to_owned(),
                token_logprobs: Some(vec![TokenLogprob {
                    bytes: b"report and par".to_vec(),
                    logprob: -0.1,
                }]),
                ..LabStream::default()
            },
            "",
        );
        assert_eq!(unaligned.outcome, "suggestion");
        assert!(unaligned.candidate_mean_token_logprob.is_none());
        assert!(
            unaligned
                .warnings
                .contains(&"candidate_probability_unavailable")
        );
    }

    #[test]
    fn legacy_context_instruction_and_healing_payloads_remain_exact() {
        let plain = "We are editing a city guide.\n\nKeep directions brief.\n\nPlease review the ";
        let instructed = "<|im_start|>system\nContinue the user's draft in en. Preserve its language and style. Use the supplied context only when relevant. Do not invent facts. Write only the continuation.\n<|im_end|>\n<|im_start|>user\nContext and writing examples:\nWe are editing a city guide.\n\nKeep directions brief.\n\nContinue the draft in the assistant message.<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\nPlease review the ";
        for (mode, prompt, echo) in [
            (Mode::Context, plain, ""),
            (Mode::Instructed, instructed, ""),
            (Mode::Healed, plain.trim_end_matches(' '), " "),
        ] {
            let prepared = prepare_prompt(&request(mode));
            let mut expected = json!({"prompt":prompt,"n_predict":32,"temperature":0.0,
                "seed":42,"stop":["\n","<|im_end|>","<|endoftext|>"],"stream":true,"cache_prompt":true});
            if mode == Mode::Healed {
                expected["grammar"] = json!("root ::= \" \" [^<>\\n\\r`]*");
            }
            assert_eq!(prepared.payload, expected);
            assert_eq!(prepared.echo, echo);
        }
    }

    #[test]
    fn instructed_healed_composes_existing_prompt_and_echo_without_budget_or_whitespace_changes() {
        for (before, language) in [
            ("Please review the", "en"),
            ("Please review the  ", "en"),
            ("Please review the docum", "en"),
            ("Bitte lies das Dokument", "de"),
            ("Bitte lies das ", "de"),
            ("لطفا این گزارش را ", "fa"),
            ("او می‌تواند", "fa"),
            ("Please review\u{a0}", "en"),
            ("Please review\t", "en"),
            ("Please review\n", "en"),
        ] {
            let mut value = request(Mode::Instructed);
            value.before = before.to_owned();
            value.language = language.to_owned();
            value.config.budget_ms = 550;
            value.config.max_tokens = 8;
            value.validate().unwrap();
            let mut expected = prepare_prompt(&value).payload;
            value.config.mode = Mode::Healed;
            let healed = prepare_prompt(&value);
            let prefix = expected["prompt"]
                .as_str()
                .unwrap()
                .strip_suffix(before)
                .unwrap();
            expected["prompt"] = json!(format!(
                "{prefix}{}",
                before.strip_suffix(&healed.echo).unwrap()
            ));
            if let Some(grammar) = healed.payload.get("grammar") {
                expected["grammar"] = grammar.clone();
            }
            value.config.mode = Mode::InstructedHealed;
            let combined = prepare_prompt(&value);
            assert_eq!(combined.payload, expected);
            assert_eq!(combined.echo, healed.echo);
            assert_eq!(combined.payload["n_predict"], 8);
            assert_eq!(
                serde_json::to_value(value.config.mode).unwrap(),
                "instructed_healed"
            );
        }
    }

    #[test]
    fn instructed_healed_returns_only_exact_multilingual_continuations() {
        for (before, language, raw, expected) in [
            ("Please review the  ", "en", "  report", "report"),
            (
                "Please review the docum",
                "en",
                " documents and",
                "ents and",
            ),
            (
                "Bitte lies das ",
                "de",
                "Dokument sorgfältig",
                "Dokument sorgfältig",
            ),
            ("لطفا این گزارش را ", "fa", " بخوانید", "بخوانید"),
            ("او می‌تواند", "fa", " بنویسد", " بنویسد"),
        ] {
            let mut value = request(Mode::InstructedHealed);
            value.before = before.to_owned();
            value.language = language.to_owned();
            value.validate().unwrap();
            let prepared = prepare_prompt(&value);
            let mut result = value.result(&fixture_identity());
            apply_observation(
                &mut result,
                &value,
                LabStream {
                    raw: raw.to_owned(),
                    stopped: true,
                    terminal_received: true,
                    ..LabStream::default()
                },
                &prepared.echo,
            );
            assert_eq!(result.outcome, "suggestion");
            assert_eq!(result.text.as_deref(), Some(expected));
            assert!(result.word_complete && result.shape_valid);
            assert_eq!(result.terminal_received, Some(true));
            if !prepared.echo.is_empty() {
                assert!(
                    result
                        .warnings
                        .contains(&"ttft_includes_forced_boundary_echo")
                );
            }
        }
    }

    #[test]
    fn instructed_healed_preserves_exact_echo_language_seam_and_completion_guards() {
        for (before, language, raw, natural, deadline, reason) in [
            (
                "Please review the  ",
                "en",
                " report",
                true,
                false,
                "echo_mismatch",
            ),
            (
                "او می‌تواند ",
                "fa",
                "میتواند بنویسد",
                true,
                false,
                "echo_mismatch",
            ),
            (
                "Please review the docum",
                "en",
                " documzzzz ",
                true,
                false,
                "output_seam",
            ),
            (
                "Bitte lies das Dokum",
                "de",
                "Dokument ",
                true,
                false,
                "output_seam",
            ),
            (
                "لطفا این گزارش را ",
                "fa",
                " English words",
                true,
                false,
                "output_language_or_shape",
            ),
            (
                "Please review the ",
                "en",
                " report",
                false,
                false,
                "no_complete_word",
            ),
            (
                "Please review the ",
                "en",
                " report",
                false,
                true,
                "no_complete_word",
            ),
        ] {
            let mut value = request(Mode::InstructedHealed);
            value.before = before.to_owned();
            value.language = language.to_owned();
            value.validate().unwrap();
            let prepared = prepare_prompt(&value);
            let mut result = value.result(&fixture_identity());
            apply_observation(
                &mut result,
                &value,
                LabStream {
                    raw: raw.to_owned(),
                    stopped: natural,
                    deadline,
                    terminal_received: !deadline,
                    ..LabStream::default()
                },
                &prepared.echo,
            );
            assert_eq!(
                result.outcome,
                if deadline { "deadline" } else { "abstention" }
            );
            assert_eq!(result.reason, reason);
            assert!(result.text.is_none() && !result.word_complete && !result.shape_valid);
        }
    }

    #[test]
    fn instructed_healed_does_not_change_the_frozen_paced_healed_contract() {
        let events: Vec<_> = [0, 100, 250, 500]
            .into_iter()
            .enumerate()
            .map(|(index, at_ms)| {
                json!({"at_ms":at_ms,"request":{"schema":REQUEST_SCHEMA,
                "id":uuid::Uuid::new_v4(),"before":format!("Draft {}", &"abc"[..index]),
                "language":"en","context":"Stable context.","style_examples":[],
                "config":{"mode":"healed","budget_ms":550,"max_tokens":8,
                    "cache_prompt":true,"temperature":0,"seed":42}}})
            })
            .collect();
        let mut wire = json!({"schema":"badi.paced-probe.request.v1","arm":"cold",
            "prelude_ms":1500,"events":events});
        serde_json::from_value::<paced_probe::ProbeInput>(wire.clone())
            .unwrap()
            .validate()
            .unwrap();
        for mode in ["instructed_healed"] {
            for event in wire["events"].as_array_mut().unwrap() {
                event["request"]["config"]["mode"] = json!(mode);
            }
            assert!(
                serde_json::from_value::<paced_probe::ProbeInput>(wire.clone())
                    .unwrap()
                    .validate()
                    .is_err()
            );
        }
    }

    #[test]
    fn a_token_limit_never_proves_word_completion() {
        assert_eq!(complete_output(" next", false), None);
        assert_eq!(
            complete_output(" next wor", false).as_deref(),
            Some(" next")
        );
        assert_eq!(complete_output(" next", true).as_deref(), Some(" next"));
        assert_eq!(
            complete_output(" گزارش جد", false).as_deref(),
            Some(" گزارش")
        );
    }

    fn observe_continuation(
        mode: Mode,
        before: &str,
        language: &str,
        context: &str,
        style: &[&str],
        continuation: &str,
    ) -> ResultRecord {
        let mut value = request(mode);
        value.before = before.to_owned();
        value.language = language.to_owned();
        value.context = context.to_owned();
        value.style_examples = style.iter().map(|example| (*example).to_owned()).collect();
        let prepared = prepare_prompt(&value);
        let raw = format!("{}{continuation}", prepared.echo);
        let mut result = value.result(&fixture_identity());
        apply_observation(
            &mut result,
            &value,
            LabStream {
                raw,
                stopped: true,
                terminal_received: true,
                ..LabStream::default()
            },
            &prepared.echo,
        );
        result
    }

    #[test]
    fn style_examples_cannot_introduce_calendar_or_numeric_facts() {
        let conflict = observe_continuation(
            Mode::Healed,
            "We agreed on ",
            "en",
            "The meeting stays on Tuesday.",
            &["The sample letter is dated Friday."],
            "Friday",
        );
        assert_eq!(conflict.outcome, "abstention");
        assert_eq!(conflict.reason, "style_fact_conflict");
        assert!(conflict.text.is_none());

        let current_day = observe_continuation(
            Mode::Healed,
            "We agreed on ",
            "en",
            "The meeting stays on Tuesday.",
            &["The sample letter is dated Friday."],
            "Tuesday",
        );
        assert_eq!(current_day.outcome, "suggestion");
        assert_eq!(current_day.text.as_deref(), Some("Tuesday"));

        let style_word = observe_continuation(
            Mode::Context,
            "Please review the ",
            "en",
            "We are editing a city guide.",
            &["Keep directions brief."],
            "brief",
        );
        assert_eq!(style_word.outcome, "suggestion");
        assert_eq!(style_word.text.as_deref(), Some("brief"));

        let no_style = observe_continuation(
            Mode::Context,
            "We agreed on ",
            "en",
            "The meeting stays on Tuesday.",
            &[],
            "Friday",
        );
        assert_eq!(no_style.outcome, "suggestion");
        assert_eq!(no_style.text.as_deref(), Some("Friday"));

        let persian_conflict = observe_continuation(
            Mode::Healed,
            "جلسه ",
            "fa",
            "برنامه هفته بعد است.",
            &["نمونهٔ نامه برای جمعه است."],
            "جمعه",
        );
        assert_eq!(persian_conflict.outcome, "abstention");
        assert_eq!(persian_conflict.reason, "style_fact_conflict");
        assert!(persian_conflict.text.is_none());

        let persian_current = observe_continuation(
            Mode::Healed,
            "جلسه ",
            "fa",
            "جلسه جمعه برقرار است.",
            &["نمونهٔ نامه برای جمعه است."],
            "جمعه",
        );
        assert_eq!(persian_current.outcome, "suggestion");
        assert_eq!(persian_current.text.as_deref(), Some("جمعه"));

        let numeric_conflict = observe_continuation(
            Mode::Context,
            "Call me ",
            "en",
            "The office is open today.",
            &["Use extension 14 after lunch."],
            "at 14",
        );
        assert_eq!(numeric_conflict.outcome, "abstention");
        assert_eq!(numeric_conflict.reason, "style_fact_conflict");
        assert!(numeric_conflict.text.is_none());

        let numeric_current = observe_continuation(
            Mode::Context,
            "Call extension 14 ",
            "en",
            "The office is open today.",
            &["Use extension 14 after lunch."],
            "at 14",
        );
        assert_eq!(numeric_current.outcome, "suggestion");
        assert_eq!(numeric_current.text.as_deref(), Some("at 14"));

        let longer_number = observe_continuation(
            Mode::Context,
            "Call me ",
            "en",
            "The office is open today.",
            &["Use extension 140 after lunch."],
            "at 14",
        );
        assert_eq!(longer_number.outcome, "suggestion");
        assert_eq!(longer_number.text.as_deref(), Some("at 14"));
    }

    #[test]
    fn persian_weekday_with_zwnj_is_a_style_fact() {
        let wednesday = observe_continuation(
            Mode::Context,
            "جلسه ",
            "fa",
            "برنامه هفته است.",
            &["نمونه برای سه‌شنبه است."],
            "سه‌شنبه",
        );
        assert_eq!(wednesday.outcome, "abstention");
        assert_eq!(wednesday.reason, "style_fact_conflict");
    }

    #[test]
    fn numeric_style_facts_match_across_digit_scripts() {
        let mut arabic_indic = request(Mode::Context);
        arabic_indic.before = "باب ".to_owned();
        arabic_indic.context.clear();
        arabic_indic.language = "fa".to_owned();
        arabic_indic.style_examples = vec!["رقم \u{0661}\u{0664}".to_owned()];
        assert!(style_fact_conflict("باب \u{0661}\u{0664}", &arabic_indic));
        arabic_indic.context = "رقم \u{0661}\u{0664}".to_owned();
        assert!(!style_fact_conflict("باب \u{0661}\u{0664}", &arabic_indic));

        arabic_indic.context.clear();
        arabic_indic.style_examples = vec!["رقم 14".to_owned()];
        assert!(style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));
        assert!(style_fact_conflict("باب 1\u{0664}", &arabic_indic));
        arabic_indic.context = "رقم \u{0661}\u{0664}".to_owned();
        assert!(!style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));
        arabic_indic.context = "رقم 140".to_owned();
        assert!(style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));

        let cross_script_conflict = observe_continuation(
            Mode::Context,
            "شماره ",
            "fa",
            "",
            &["در نمونه شماره 14 را بنویس."],
            "باب \u{06f1}\u{06f4}",
        );
        assert_eq!(cross_script_conflict.outcome, "abstention");
        assert_eq!(cross_script_conflict.reason, "style_fact_conflict");
        assert!(cross_script_conflict.text.is_none());
    }

    #[test]
    fn baseline_accepts_identical_cases_but_keeps_the_production_bound() {
        let mut value = request(Mode::ProductionBoundary);
        value.validate().expect("same context/style case");
        value.before = "a".repeat(513);
        assert!(value.validate().is_err());
    }

    /// The Lab's launch settings bind its runtime identity, which every
    /// record carries and run provenance compares across builds. This pins
    /// the exact serialized identity of each Lab launch variant.
    #[test]
    fn lab_launch_identity_serialization_is_pinned() {
        use badi_broker::semantic::provenance::{FileExpectation, VerifiedFile, verify_file};
        use badi_broker::semantic::runtime::FixtureBehavior;

        let temporary = tempfile::tempdir().expect("temporary directory");
        let verified = |name: &str, bytes: &[u8]| -> VerifiedFile {
            let path = temporary.path().join(name);
            std::fs::write(&path, bytes).expect("fixture file");
            let size = u64::try_from(bytes.len()).expect("fixture size");
            verify_file(&FileExpectation::new(&path, sha256_hex(bytes), size).expect("expectation"))
                .expect("verified fixture")
        };
        let launch = LlamaCppLaunch::for_fixture(
            verified("runtime", b"fixture runtime"),
            verified("model", b"fixture model"),
            FixtureBehavior::Ready,
        )
        .expect("fixture launch");
        for (prefill_batch, explicit_artifact, expected) in [
            (PrefillBatch::Tokens16, false, GOLDEN_DEFAULT),
            (PrefillBatch::Tokens64, false, GOLDEN_TOKENS64),
            (PrefillBatch::Tokens16, true, GOLDEN_EXPLICIT_ARTIFACT),
        ] {
            let identity = configure_launch(launch.clone(), prefill_batch, explicit_artifact)
                .expect("Lab launch")
                .identity();
            let serialized = serde_json::to_string(&identity).expect("identity JSON");
            assert_eq!(
                serialized, expected,
                "{prefill_batch:?} {explicit_artifact}"
            );
        }
    }

    const GOLDEN_DEFAULT: &str = concat!(
        "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
        ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
        ",\"runtime_bundle_manifest_sha256\":null",
        ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
        ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
        ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":16,\"ubatch_size\":16}",
    );
    const GOLDEN_TOKENS64: &str = concat!(
        "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
        ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
        ",\"runtime_bundle_manifest_sha256\":null",
        ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
        ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
        ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":64,\"ubatch_size\":64}",
    );
    const GOLDEN_EXPLICIT_ARTIFACT: &str = concat!(
        "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
        ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
        ",\"runtime_bundle_manifest_sha256\":null",
        ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
        ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\"",
        ",\"model_origin\":\"explicit_lab_artifact\",\"threads\":1",
        ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":16,\"ubatch_size\":16}",
    );
}
