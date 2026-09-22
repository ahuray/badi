//! Opt-in, memory-only prediction experiments. This is not an adapter authority
//! surface: inputs are explicitly supplied and results never edit a document.

pub mod artifact;
mod attestation;
pub mod context_lookup;
pub mod paced_probe;
pub mod prefill_probe;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod process;
pub mod spelling;
pub mod stop_token_probe;

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;

use crate::provider::{CompletionProvider, ProviderRequest};
use crate::semantic::client::{ClientError, LabObservation, LabStream, TokenLogprob};
use crate::semantic::runtime::{OwnedRuntime, StableRuntimeIdentity};
use crate::writing::{self, WritingLanguage};

pub const REQUEST_SCHEMA: &str = "badi.prediction-lab.request.v1";
pub const LAUNCH_CONTRACT: &str = "badi.prediction-lab.owned-2048.v2";
pub const CONTEXT_TOKENS: u16 = 2048;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

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
    ProductionBaseline,
    ProductionBoundary,
    Context,
    ContextConfidence,
    Instructed,
    Healed,
    InstructedHealed,
    InstructedWord,
    HealedAttested,
    NativeInstructed,
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
    let launch = if let Some(artifact) = artifact {
        writing::prepare_lab_artifact(directory, artifact)
            .await
            .map_err(|_| LabError::RuntimeUnavailable)?
            .for_writing_lab_artifact()
    } else {
        writing::prepare_launch(directory)
            .await
            .map_err(|_| LabError::RuntimeUnavailable)?
            .0
    };
    launch
        .for_writing_lab()
        .with_lab_prefill_batch(prefill_batch)
        .spawn()
        .await
        .map_err(|_| LabError::RuntimeUnavailable)
}

fn valid_text(text: &str, limit: usize) -> bool {
    text.chars().count() <= limit && !text.contains("<|")
        && !text.chars().any(|ch| {
            (ch.is_control() && !matches!(ch, '\n' | '\t'))
                || matches!(ch, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200b}' | '\u{feff}')
        })
        && crate::segment::valid_orthographic_joiners(text)
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
        if matches!(
            config.mode,
            Mode::ProductionBaseline | Mode::ProductionBoundary
        ) && self.before.chars().count() > 512
        {
            return Err(LabError::InvalidRequest);
        }
        Ok(())
    }

    fn provider_request(&self) -> ProviderRequest {
        ProviderRequest {
            before: self.before.clone(),
            after: String::new(),
            language: Some(self.language.clone()),
        }
    }

    fn result(&self, identity: &StableRuntimeIdentity) -> ResultRecord {
        let mut warnings = vec![
            "lab_runtime_2048_differs_from_installed_512",
            "no_application_edit_or_user_acceptance_measured",
        ];
        if self.config.mode == Mode::InstructedWord {
            warnings.extend([
                "constrained_terminal_is_not_natural_model_choice",
                "constrained_word_readiness_timing_unavailable",
                "constrained_lexical_alphabet_is_bounded",
            ]);
        }
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
        Mode::Healed
            | Mode::InstructedHealed
            | Mode::InstructedWord
            | Mode::HealedAttested
            | Mode::NativeInstructed
    ) {
        let without_spaces = before.trim_end_matches(' ');
        if without_spaces.len() != before.len() {
            before[without_spaces.len()..].clone_into(&mut echo);
            before = without_spaces;
        } else if let Some(stem) = writing::healing_prefix(before) {
            stem.clone_into(&mut echo);
            before = &before[..before.len() - stem.len()];
        }
    }
    let context = context_prefix(&request.context, &request.style_examples);
    let prompt = if matches!(
        request.config.mode,
        Mode::Instructed | Mode::InstructedHealed | Mode::InstructedWord
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
    if request.config.mode == Mode::InstructedWord {
        payload["grammar"] = json!(word_grammar(
            &echo,
            request.before.ends_with(' '),
            &request.language
        ));
    } else if !echo.is_empty() {
        let literal = serde_json::to_string(&echo).expect("string serialization");
        payload["grammar"] = json!(format!("root ::= {literal} [^<>\\n\\r`]*"));
    }
    PreparedPrompt { payload, echo }
}

const LATIN_LETTERS: &[(char, char)] = &[
    ('A', 'Z'),
    ('a', 'z'),
    ('\u{00c0}', '\u{00d6}'),
    ('\u{00d8}', '\u{00f6}'),
    ('\u{00f8}', '\u{024f}'),
    ('\u{1e00}', '\u{1eff}'),
];
// Match the existing orthographic joiner base ranges. The normal sanitizer
// remains authoritative; this Lab grammar must never broaden its joiner rules.
const ARABIC_LETTERS: &[(char, char)] = &[
    ('\u{0620}', '\u{063f}'),
    ('\u{0641}', '\u{064a}'),
    ('\u{066e}', '\u{066f}'),
    ('\u{0671}', '\u{06d3}'),
    ('\u{06d5}', '\u{06d5}'),
    ('\u{06e5}', '\u{06e6}'),
    ('\u{06ee}', '\u{06ef}'),
    ('\u{06fa}', '\u{06fc}'),
    ('\u{06ff}', '\u{06ff}'),
];

fn lexical_letters(persian: bool) -> &'static [(char, char)] {
    if persian {
        ARABIC_LETTERS
    } else {
        LATIN_LETTERS
    }
}

fn word_grammar(echo: &str, after_space: bool, language: &str) -> String {
    let literal = serde_json::to_string(echo).expect("string serialization");
    let leading_space = if after_space { "" } else { " \" \"?" };
    let persian = WritingLanguage::from_tag(language) == Some(WritingLanguage::Persian);
    let letters: String = lexical_letters(persian)
        .iter()
        .map(|&(start, end)| {
            if start == end {
                format!("\\u{:04x}", u32::from(start))
            } else {
                format!("\\u{:04x}-\\u{:04x}", u32::from(start), u32::from(end))
            }
        })
        .collect();
    let (punctuation, rules) = if persian {
        (
            r"[.,;:!?\u2026\u060c\u061b\u061f]",
            format!(
                "word ::= letter (marks? letter | \"\\u200c\" letter)* marks?\nletter ::= [{letters}]\nmarks ::= [\\u064b-\\u065f\\u0670]+"
            ),
        )
    } else {
        (
            r"[.,;:!?\u2026]",
            format!(
                "word ::= letter+ (['\\u2019] letter+)*\nletter ::= [{letters}] [\\u0300-\\u036f]*"
            ),
        )
    };
    // Completion requires an emitted separator. Grammar exhaustion permits the
    // runtime's real EOS; it does not prove the model would naturally stop here.
    format!("root ::= {literal}{leading_space} word {punctuation}? \" \"\n{rules}")
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
    if matches!(
        request.config.mode,
        Mode::ProductionBaseline | Mode::ProductionBoundary
    ) {
        return baseline(runtime, &request, cancellation, started).await;
    }
    let mut prepared = prepare_prompt(&request);
    let mut result = request.result(runtime.identity());
    result.used_context = !request.context.is_empty();
    result.used_style = !request.style_examples.is_empty();
    if request.config.mode == Mode::NativeInstructed {
        let messages = native_instruction_messages(&request);
        let template = match runtime
            .client()
            .lab_apply_template(messages, request.config.budget_ms, cancellation.clone())
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
    let count = match runtime
        .client()
        .lab_token_count(
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
    let observed = runtime
        .client()
        .lab_stream(
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
        result.warnings.push("ignored_by_production_baseline");
    }
    result.warnings.push("production_raw_and_ttft_not_retained");
    let (client, trace) = runtime.client().with_lab_trace();
    let client = if request.config.mode == Mode::ProductionBoundary {
        result
            .warnings
            .push("experimental_boundary_policy_not_enabled_in_production");
        client.with_lab_production_boundary()
    } else {
        client
    };
    let proposal = client
        .propose(request.provider_request(), cancellation, true)
        .await
        .map_err(|error| match error {
            crate::provider::ProviderError::Cancelled => LabError::Cancelled,
            crate::provider::ProviderError::Unavailable => LabError::RuntimeUnavailable,
        })?;
    result
        .model_requests
        .clone_from(&*trace.lock().map_err(|_| LabError::RuntimeUnavailable)?);
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

fn apply_observation(
    result: &mut ResultRecord,
    request: &Request,
    observed: LabStream,
    echo: &str,
) {
    record_observation_metrics(result, request, &observed);
    let continuation = observed.raw.strip_prefix(echo);
    let echo_matched = continuation.is_some();
    let candidate = if request.config.mode == Mode::InstructedWord {
        continuation.ok_or("echo_mismatch").and_then(|text| {
            complete_constrained_word(
                text,
                request.before.ends_with(' '),
                &request.language,
                observed.terminal_received,
                observed.deadline,
            )
        })
    } else {
        continuation
            .and_then(|text| complete_output(text, observed.stopped))
            .ok_or(if echo_matched {
                "no_complete_word"
            } else {
                "echo_mismatch"
            })
    };
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
        if language_valid && request.config.mode == Mode::HealedAttested {
            if let Some(recovered) =
                attestation::recover_from_complete_candidate(request, echo, &candidate)
            {
                apply_attested_suffix(result, &recovered, observed.deadline);
                return;
            }
        }
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
    result.reason = if request.config.mode == Mode::InstructedWord {
        "constrained_complete_word"
    } else if observed.deadline {
        "complete_prefix_at_deadline"
    } else {
        "complete_prefix"
    };
    result.word_complete = true;
    result.shape_valid = candidate.chars().count() <= 64;
    result.text = Some(candidate);
}

fn record_observation_metrics(result: &mut ResultRecord, request: &Request, observed: &LabStream) {
    result.inference_ms = Some(observed.latency_ms);
    result.ttft_ms = observed.ttft_ms;
    result.first_word_ms = observed.first_word_ms;
    result.first_four_words_ms = observed.first_four_words_ms;
    if request.config.mode == Mode::InstructedWord {
        // The shared observer does not apply this mode's terminal and exact
        // one-word checks. Preserve measured request latency, not that earlier
        // observation as evidence this stricter candidate was ready to display.
        result.first_word_ms = None;
        result.first_four_words_ms = None;
    }
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

fn complete_constrained_word(
    text: &str,
    after_space: bool,
    language: &str,
    terminal_received: bool,
    deadline: bool,
) -> Result<String, &'static str> {
    if !terminal_received || deadline {
        return Err("word_terminal_missing");
    }
    // A token limit is sufficient only when the runtime really finished and
    // the generated bytes already contain the required ASCII separator.
    let candidate = text.strip_suffix(' ').ok_or("word_separator_missing")?;
    let word = if after_space {
        candidate
    } else {
        candidate.strip_prefix(' ').unwrap_or(candidate)
    };
    if word.chars().any(char::is_whitespace) {
        return Err("word_spacing");
    }
    if word.contains(['<', '>', '`']) || word.chars().any(char::is_control) {
        return Err("word_grammar_shape");
    }
    match word.unicode_words().count() {
        0 => Err("word_count"),
        1 => {
            let candidate = crate::segment::sanitize_suggestion(candidate)
                .map_err(|_| "output_language_or_shape")?;
            if !WritingLanguage::from_tag(language)
                .is_some_and(|value| value.accepts_output(&candidate))
            {
                return Err("output_language_or_shape");
            }
            if !valid_lexical_word(word, language) {
                return Err("word_grammar_shape");
            }
            Ok(candidate)
        }
        // Compound segmentation is intentionally unsupported in this bounded
        // experiment; do not silently retain only its first component.
        _ => Err("unsupported_compound"),
    }
}

fn valid_lexical_word(word: &str, language: &str) -> bool {
    let persian = WritingLanguage::from_tag(language) == Some(WritingLanguage::Persian);
    let punctuation = |ch| {
        matches!(ch, '.' | ',' | ';' | ':' | '!' | '?' | '\u{2026}')
            || (persian && matches!(ch, '\u{060c}' | '\u{061b}' | '\u{061f}'))
    };
    let word = word
        .char_indices()
        .next_back()
        .filter(|&(_, ch)| punctuation(ch))
        .map_or(word, |(end, _)| &word[..end]);
    let mut needs_letter = true;
    let mut previous_letter = false;
    for ch in word.chars() {
        if lexical_letters(persian)
            .iter()
            .any(|&(start, end)| (start..=end).contains(&ch))
        {
            needs_letter = false;
            previous_letter = true;
        } else if !needs_letter
            && if persian {
                matches!(ch, '\u{064b}'..='\u{065f}' | '\u{0670}')
            } else {
                matches!(ch, '\u{0300}'..='\u{036f}')
            }
        {
            previous_letter = false;
        } else if (!persian && !needs_letter && matches!(ch, '\'' | '\u{2019}'))
            || (persian && previous_letter && ch == '\u{200c}')
        {
            needs_letter = true;
            previous_letter = false;
        } else {
            return false;
        }
    }
    !needs_letter
}

fn apply_attested_suffix(
    result: &mut ResultRecord,
    recovered: &attestation::AttestedSuffix<'_>,
    deadline: bool,
) {
    result.outcome = "suggestion";
    result.reason = match recovered.source {
        attestation::Source::Context => "attested_context_word_suffix",
        attestation::Source::Style => "attested_style_word_suffix",
    };
    result.text = Some(recovered.suffix.to_owned());
    result.replace_before = None;
    result.word_complete = true;
    result.shape_valid = true;
    // The unchanged stream observer applies the legacy seam rule. It cannot
    // measure when this experimental recovery first became eligible.
    result.first_word_ms = None;
    result.first_four_words_ms = None;
    result.warnings.extend([
        "attested_word_is_contextual_reuse_not_spelling_validation",
        "attested_word_readiness_timing_unavailable",
    ]);
    if deadline {
        result
            .warnings
            .push("attested_word_from_complete_prefix_at_deadline");
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FactRun {
    Digits,
    Letters,
}

fn fact_digit(character: char) -> Option<char> {
    let value = match character {
        '0'..='9' => character as u32 - '0' as u32,
        '\u{0660}'..='\u{0669}' => character as u32 - '\u{0660}' as u32,
        '\u{06F0}'..='\u{06F9}' => character as u32 - '\u{06F0}' as u32,
        _ => return None,
    };
    char::from_u32('0' as u32 + value)
}

fn fact_run(character: char, current: Option<FactRun>) -> Option<FactRun> {
    if fact_digit(character).is_some() {
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
                        fact_digit(character).expect("numeric fact run contains supported digits")
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
        for mode in [Mode::ProductionBaseline, Mode::ProductionBoundary] {
            let record = serde_json::to_value(self::request(mode).result(&fixture_identity()))
                .expect("production record defaults");
            assert!(record["reused_prompt_tokens"].is_null());
            assert!(record["newly_evaluated_prompt_tokens"].is_null());
        }
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
                assert!(prepared.echo.is_empty());
            }
        }
        let mut value = request(Mode::Healed);
        value.before = "Hello\u{a0}".to_owned();
        assert!(prepare_prompt(&value).echo.is_empty());
        value.before = "Please review the docum".to_owned();
        assert_eq!(prepare_prompt(&value).echo, "docum");
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
            ("Please review the docum", "en", "documents and", "ents and"),
            (
                "Bitte lies das ",
                "de",
                " Dokument sorgfältig",
                "Dokument sorgfältig",
            ),
            ("لطفا این گزارش را ", "fa", " بخوانید", "بخوانید"),
            ("او می‌تواند", "fa", "می‌تواند بنویسد", " بنویسد"),
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
            assert!(
                result
                    .warnings
                    .contains(&"ttft_includes_forced_boundary_echo")
            );
        }
    }

    #[test]
    fn instructed_word_changes_only_grammar_and_preserves_exact_prompt_and_echo() {
        for (before, language, echo) in [
            ("Please review the", "en", ""),
            ("Please review the  ", "en", "  "),
            ("Please review the docum", "en", "docum"),
            ("Bitte lies das Dokument", "de", "Dokument"),
            ("Bitte lies das ", "de", " "),
            ("لطفا این گزارش را ", "fa", " "),
            ("او می‌تواند", "fa", "می‌تواند"),
            ("Please review\u{a0}", "en", ""),
            ("Please review\t", "en", ""),
            ("Please review\n", "en", ""),
        ] {
            let mut value = request(Mode::InstructedHealed);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            value.config.budget_ms = 550;
            value.config.max_tokens = 8;
            let original = prepare_prompt(&value);
            value.config.mode = Mode::InstructedWord;
            value.validate().unwrap();
            let actual = prepare_prompt(&value);
            assert_eq!(original.echo, echo);
            assert_eq!(actual.echo, echo);
            let mut expected = original.payload;
            let literal = serde_json::to_string(echo).unwrap();
            let prefix = if before.ends_with(' ') {
                format!("root ::= {literal}")
            } else {
                format!("root ::= {literal} \" \"?")
            };
            let (punctuation, rules) = if language == "fa" {
                (
                    r"[.,;:!?\u2026\u060c\u061b\u061f]",
                    concat!(
                        "word ::= letter (marks? letter | \"\\u200c\" letter)* marks?\n",
                        "letter ::= [\\u0620-\\u063f\\u0641-\\u064a\\u066e-\\u066f\\u0671-\\u06d3\\u06d5\\u06e5-\\u06e6\\u06ee-\\u06ef\\u06fa-\\u06fc\\u06ff]\n",
                        "marks ::= [\\u064b-\\u065f\\u0670]+"
                    ),
                )
            } else {
                (
                    r"[.,;:!?\u2026]",
                    concat!(
                        "word ::= letter+ (['\\u2019] letter+)*\n",
                        "letter ::= [\\u0041-\\u005a\\u0061-\\u007a\\u00c0-\\u00d6\\u00d8-\\u00f6\\u00f8-\\u024f\\u1e00-\\u1eff] [\\u0300-\\u036f]*"
                    ),
                )
            };
            expected["grammar"] = json!(format!("{prefix} word {punctuation}? \" \"\n{rules}"));
            assert_eq!(actual.payload, expected, "{before}");
            assert_eq!(
                serde_json::to_value(value.config.mode).unwrap(),
                "instructed_word"
            );
            let decoded: Mode = serde_json::from_value(json!("instructed_word")).unwrap();
            assert_eq!(decoded, Mode::InstructedWord);
            value.config.budget_ms = 549;
            assert!(value.validate().is_err());
            value.config.budget_ms = 550;
            value.config.max_tokens = 7;
            assert!(value.validate().is_err());
        }
    }

    #[test]
    fn instructed_word_requires_a_real_terminal_and_exact_separator_even_after_eos() {
        let value = request(Mode::InstructedWord);
        let prepared = prepare_prompt(&value);
        for (raw, stopped, terminal, deadline, reason) in [
            (" report", true, true, false, "word_separator_missing"),
            (" report", false, true, false, "word_separator_missing"),
            (" report\u{a0}", true, true, false, "word_separator_missing"),
            (" report ", true, false, false, "word_terminal_missing"),
            (" report ", false, false, true, "word_terminal_missing"),
            (" report ", true, true, true, "word_terminal_missing"),
            (" report  ", true, true, false, "word_spacing"),
            ("  report ", true, true, false, "word_spacing"),
            (" report next ", true, true, false, "word_spacing"),
            (" report\t ", true, true, false, "word_spacing"),
            (" report\u{2009} ", true, true, false, "word_spacing"),
            (" rep\0ort ", true, true, false, "word_grammar_shape"),
            (" <report> ", true, true, false, "word_grammar_shape"),
            (" `report` ", true, true, false, "word_grammar_shape"),
            ("  ", true, true, false, "word_count"),
            (" --- ", true, true, false, "word_count"),
            (" city-tour ", true, true, false, "unsupported_compound"),
        ] {
            let mut result = value.result(&fixture_identity());
            apply_observation(
                &mut result,
                &value,
                LabStream {
                    raw: raw.to_owned(),
                    stopped,
                    terminal_received: terminal,
                    deadline,
                    first_word_ms: Some(60.0),
                    first_four_words_ms: Some(210.0),
                    ..LabStream::default()
                },
                &prepared.echo,
            );
            assert_eq!(result.reason, reason, "{raw:?}");
            assert_eq!(
                result.outcome,
                if deadline { "deadline" } else { "abstention" }
            );
            assert_eq!(result.terminal_received, Some(terminal));
            assert!(result.text.is_none() && !result.word_complete && !result.shape_valid);
            assert!(result.first_word_ms.is_none() && result.first_four_words_ms.is_none());
        }
    }

    #[test]
    fn instructed_word_returns_exact_single_word_continuations_with_honest_timing() {
        for (before, language, raw, expected) in [
            ("Please review the  ", "en", "  report ", "report"),
            ("Please review the", "en", " report. ", " report."),
            ("Please review the docum", "en", "documents ", "ents"),
            ("I ", "en", " don't ", "don't"),
            ("Bitte lies das ", "de", " Dokument ", "Dokument"),
            (
                "Bitte lies das Dokument",
                "de",
                "Dokument sorgfältig ",
                " sorgfältig",
            ),
            ("لطفا این گزارش را ", "fa", " بخوانید ", "بخوانید"),
            ("او می‌تواند", "fa", "می‌تواند بنویسد ", " بنویسد"),
            ("او ", "fa", " می‌نویسد ", "می‌نویسد"),
        ] {
            let mut value = request(Mode::InstructedWord);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            value.validate().unwrap();
            for stopped in [true, false] {
                // Both natural EOS and a token limit must have supplied the
                // same explicit separator and actual terminal record.
                let result = observation_result(&value, raw, stopped, false);
                assert_eq!(result.outcome, "suggestion", "{before}: {}", result.reason);
                assert_eq!(result.reason, "constrained_complete_word");
                assert_eq!(result.text.as_deref(), Some(expected));
                assert!(result.word_complete && result.shape_valid);
                assert_eq!(result.terminal_received, Some(true));
                assert!(result.replace_before.is_none());
                assert!(result.first_word_ms.is_none() && result.first_four_words_ms.is_none());
                assert_eq!(result.inference_ms, Some(321.0));
                assert_eq!(result.ttft_ms, Some(17.0));
                assert_eq!(result.raw.as_deref(), Some(raw));
                assert!(
                    result
                        .warnings
                        .contains(&"constrained_terminal_is_not_natural_model_choice")
                );
                assert!(
                    result
                        .warnings
                        .contains(&"constrained_word_readiness_timing_unavailable")
                );
            }
        }
    }

    #[test]
    fn instructed_word_preserves_echo_language_seam_and_output_safety_guards() {
        for (before, language, raw, reason) in [
            ("Please review the  ", "en", " report ", "echo_mismatch"),
            ("او می‌تواند", "fa", "میتواند بنویسد ", "echo_mismatch"),
            ("Please review the docum", "en", "documzzzz ", "output_seam"),
            ("Bitte lies das Dokum", "de", "Dokument ", "output_seam"),
            ("من می‌ر", "fa", "می‌روم ", "output_seam"),
            ("Please review the", "en", "the ", "output_seam"),
            (
                "لطفا این گزارش را ",
                "fa",
                " report ",
                "output_language_or_shape",
            ),
            (
                "Please review the ",
                "en",
                " گزارش ",
                "output_language_or_shape",
            ),
            (
                "Please review the ",
                "en",
                " \u{202e}report ",
                "output_language_or_shape",
            ),
            ("او ", "fa", " می‌ ", "output_language_or_shape"),
        ] {
            let mut value = request(Mode::InstructedWord);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            value.validate().unwrap();
            let result = observation_result(&value, raw, true, false);
            assert_eq!(result.reason, reason, "{before}: {raw}");
            assert_eq!(result.outcome, "abstention");
            assert!(result.text.is_none() && !result.word_complete && !result.shape_valid);
        }
        let long_word = format!(" {} ", "a".repeat(65));
        let result = observation_result(&request(Mode::InstructedWord), &long_word, true, false);
        assert_eq!(result.reason, "output_language_or_shape");
        assert!(result.text.is_none());
        let result = request(Mode::InstructedWord).result(&fixture_identity());
        assert!(
            result
                .warnings
                .contains(&"constrained_terminal_is_not_natural_model_choice")
        );
        assert!(
            result
                .warnings
                .contains(&"constrained_word_readiness_timing_unavailable")
        );
    }

    #[test]
    fn instructed_word_lexical_grammar_rejects_joined_words_and_foreign_scripts() {
        for (word, language) in [
            ("water,散发着aromatic清香,", "en"),
            ("water,aromatic", "en"),
            ("water,,", "en"),
            ("water散发", "en"),
            ("report2", "en"),
            ("city-tour", "en"),
            ("'tis", "en"),
            ("dogs'", "en"),
            ("don''t", "en"),
            ("\u{0308}fur", "de"),
            ("\u{2c60}rger", "de"),
            ("گزارش،بعد", "fa"),
            ("گزارشEnglish", "fa"),
            ("می\u{064e}‌روم", "fa"),
            ("می‌\u{064e}روم", "fa"),
            ("می‌", "fa"),
            ("\u{064e}روم", "fa"),
        ] {
            assert!(!valid_lexical_word(word, language), "{word}");
        }
        for (word, language) in [
            ("water", "en"),
            ("water,", "en"),
            ("don't", "en"),
            ("don’t", "en"),
            ("für", "de"),
            ("fu\u{0308}r", "de"),
            ("Ärger!", "de"),
            ("Straße", "de"),
            ("می‌روم", "fa"),
            ("گزارش،", "fa"),
            ("آماد\u{064e}ه", "fa"),
        ] {
            assert!(valid_lexical_word(word, language), "{word}");
        }
        for raw in [" water,, ", " report2 ", " 'tis "] {
            let result = observation_result(&request(Mode::InstructedWord), raw, true, false);
            assert_eq!(result.reason, "word_grammar_shape", "{raw}");
            assert!(result.text.is_none());
        }
        let result = observation_result(
            &request(Mode::InstructedWord),
            " water,散发着aromatic清香, ",
            true,
            false,
        );
        assert_eq!(result.reason, "unsupported_compound");
        assert!(result.text.is_none());
        assert!(
            result
                .warnings
                .contains(&"constrained_lexical_alphabet_is_bounded")
        );
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
                "او می‌تواند",
                "fa",
                "میتواند بنویسد",
                true,
                false,
                "echo_mismatch",
            ),
            (
                "Please review the docum",
                "en",
                "documzzzz ",
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
        for mode in ["instructed_healed", "instructed_word"] {
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
    fn attested_mode_preserves_healed_payloads_and_remains_outside_paced_replay() {
        for (before, language) in [
            ("Please review the docum", "en"),
            ("Bitte lies die Tagesord", "de"),
            ("Wir möchten jetzt lad", "de"),
            ("Bitte lies das  ", "de"),
            ("من هر روز می‌ر", "fa"),
            ("این گزارش را ", "fa"),
            ("Bitte lies das\u{a0}", "de"),
        ] {
            let mut value = request(Mode::Healed);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            value.validate().unwrap();
            let expected = prepare_prompt(&value);
            value.config.mode = Mode::HealedAttested;
            value.validate().unwrap();
            let actual = prepare_prompt(&value);
            assert_eq!(actual.payload, expected.payload);
            assert_eq!(actual.echo, expected.echo);
            assert_eq!(
                serde_json::to_value(value.config.mode).unwrap(),
                "healed_attested"
            );
        }
        let events: Vec<_> = [0, 100, 250, 500]
            .into_iter()
            .enumerate()
            .map(|(index, at_ms)| {
                json!({"at_ms":at_ms,"request":{"schema":REQUEST_SCHEMA,
                "id":uuid::Uuid::new_v4(),"before":format!("Draft {}", &"abc"[..index]),
                "language":"en","context":"Stable context.","style_examples":[],
                "config":{"mode":"healed_attested","budget_ms":550,"max_tokens":8,
                    "cache_prompt":true,"temperature":0,"seed":42}}})
            })
            .collect();
        let wire = json!({"schema":"badi.paced-probe.request.v1","arm":"cold",
            "prelude_ms":1500,"events":events});
        assert!(
            serde_json::from_value::<paced_probe::ProbeInput>(wire)
                .unwrap()
                .validate()
                .is_err()
        );
    }

    fn observation_result(
        value: &Request,
        raw: &str,
        stopped: bool,
        deadline: bool,
    ) -> ResultRecord {
        let prepared = prepare_prompt(value);
        let mut result = value.result(&fixture_identity());
        apply_observation(
            &mut result,
            value,
            LabStream {
                raw: raw.to_owned(),
                latency_ms: 321.0,
                ttft_ms: Some(17.0),
                first_word_ms: Some(60.0),
                first_four_words_ms: Some(210.0),
                stopped,
                deadline,
                terminal_received: !deadline,
                ..LabStream::default()
            },
            &prepared.echo,
        );
        result
    }

    #[test]
    fn attested_observations_recover_only_bound_suffixes_with_honest_provenance_and_timing() {
        for (before, language, source, continuation, suffix, style) in [
            (
                "Die Tagesord",
                "de",
                "Die Tagesordnung liegt bereit.",
                "nung liegt bereit.",
                "nung",
                false,
            ),
            (
                "Wir möchten jetzt lad",
                "de",
                "Wir können jetzt laden.",
                "en und weiterfahren",
                "en",
                false,
            ),
            (
                "من هر روز می‌ر",
                "fa",
                "من هر روز می‌روم.",
                "وم و بازمی‌گردم",
                "وم",
                false,
            ),
            (
                "Die Nachricht geht an Marl",
                "de",
                "Marlene erhält eine Nachricht.",
                "ene und weitere Gäste",
                "ene",
                true,
            ),
        ] {
            let mut value = request(Mode::Healed);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            value.context.clear();
            value.style_examples.clear();
            if style {
                value.style_examples.push(source.to_owned());
            } else {
                source.clone_into(&mut value.context);
            }
            let raw = format!("{}{continuation}", prepare_prompt(&value).echo);
            assert_eq!(
                observation_result(&value, &raw, true, false).reason,
                "output_seam"
            );
            value.config.mode = Mode::HealedAttested;
            let result = observation_result(&value, &raw, true, false);
            assert_eq!(result.outcome, "suggestion", "{before}: {}", result.reason);
            assert_eq!(result.text.as_deref(), Some(suffix));
            assert_eq!(
                result.reason,
                if style {
                    "attested_style_word_suffix"
                } else {
                    "attested_context_word_suffix"
                }
            );
            assert!(result.word_complete && result.shape_valid);
            assert!(result.replace_before.is_none());
            assert!(result.first_word_ms.is_none() && result.first_four_words_ms.is_none());
            assert_eq!(result.raw.as_deref(), Some(raw.as_str()));
            assert_eq!(result.inference_ms, Some(321.0));
            assert_eq!(result.ttft_ms, Some(17.0));
            assert_eq!(result.terminal_received, Some(true));
            for warning in [
                "attested_word_is_contextual_reuse_not_spelling_validation",
                "attested_word_readiness_timing_unavailable",
            ] {
                assert!(result.warnings.contains(&warning));
            }
        }
    }

    #[test]
    fn attested_observations_require_exact_echo_and_generated_word_completion() {
        let mut value = request(Mode::HealedAttested);
        "Die Tagesord".clone_into(&mut value.before);
        "de".clone_into(&mut value.language);
        "Die Tagesordnung liegt bereit.".clone_into(&mut value.context);
        value.style_examples.clear();
        for (raw, stopped, deadline, reason, outcome) in [
            (
                "Tagesordnung",
                false,
                false,
                "no_complete_word",
                "abstention",
            ),
            ("Tagesordnung", false, true, "no_complete_word", "deadline"),
            ("Tagesornu", true, false, "echo_mismatch", "abstention"),
            ("Tagesordnu ", false, true, "output_seam", "abstention"),
            (
                "Tagesordnung-spezifisch",
                true,
                false,
                "output_seam",
                "abstention",
            ),
        ] {
            let result = observation_result(&value, raw, stopped, deadline);
            assert_eq!((result.reason, result.outcome), (reason, outcome));
            assert!(result.text.is_none() && !result.word_complete && !result.shape_valid);
        }
        let result = observation_result(&value, "Tagesordnung jetzt", false, true);
        assert_eq!(result.text.as_deref(), Some("nung"));
        assert_eq!(result.reason, "attested_context_word_suffix");
        assert!(
            result
                .warnings
                .contains(&"attested_word_from_complete_prefix_at_deadline")
        );
        assert_eq!(result.terminal_received, Some(false));
        assert!(result.first_word_ms.is_none() && result.first_four_words_ms.is_none());
        value.context.clear();
        assert!(
            observation_result(&value, "Tagesordnung", true, false)
                .text
                .is_none()
        );
        "Tagesordnung".clone_into(&mut value.context);
        assert!(
            observation_result(&value, "Tagesordnung", true, false)
                .text
                .is_none()
        );
    }

    #[test]
    fn attested_mode_does_not_change_legacy_accepted_or_nonrecoverable_outputs() {
        for (before, language, context, raw) in [
            (
                "Please review the docum",
                "en",
                "The document is ready.",
                "document and report",
            ),
            (
                "Bitte lies das ",
                "de",
                "Das Dokument liegt bereit.",
                " Dokument sorgfältig",
            ),
            ("این گزارش را ", "fa", "گزارش آماده است.", " بررسی کنید"),
            (
                "Please review the docum",
                "en",
                "The documzzzz is ready.",
                "documzzzz",
            ),
            (
                "Die Tagesord",
                "de",
                "Die Tagesordnung liegt bereit.",
                "Tagesordnung بررسی",
            ),
        ] {
            let mut value = request(Mode::Healed);
            before.clone_into(&mut value.before);
            language.clone_into(&mut value.language);
            context.clone_into(&mut value.context);
            let mut legacy =
                serde_json::to_value(observation_result(&value, raw, true, false)).unwrap();
            value.config.mode = Mode::HealedAttested;
            let actual =
                serde_json::to_value(observation_result(&value, raw, true, false)).unwrap();
            legacy["config"]["mode"] = json!("healed_attested");
            assert_eq!(actual, legacy);
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
        let raw = if mode == Mode::InstructedWord {
            format!("{}{continuation} ", prepared.echo)
        } else {
            format!("{}{continuation}", prepared.echo)
        };
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
            Mode::InstructedWord,
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
            Mode::InstructedWord,
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
            Mode::InstructedWord,
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
            Mode::InstructedWord,
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
        for mode in [Mode::ProductionBaseline, Mode::ProductionBoundary] {
            let mut value = request(mode);
            value.validate().expect("same context/style case");
            value.before = "a".repeat(513);
            assert!(value.validate().is_err());
        }
    }
}
