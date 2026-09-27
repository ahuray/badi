//! Device-specific fit and empirical qualification. Metadata cannot confer quality.
//!
//! The legacy pinned catalog remains unchanged. This engine consumes discovery
//! metadata and locally collected evidence; callers must refresh the device before
//! every load. Reports are development qualification, not adapter qualification.

mod device;
pub use device::{CpuInspection, DeviceInspection, DiskInspection, GpuInspection, inspect_device};

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const MIB: u64 = 1_048_576;
const SCHEMA: &str = "badi.device-qualification.v1";
pub const EVALUATION_VERSION: &str = "badi.prediction-quality.v1";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CandidateMetadata {
    pub id: String,
    pub revision: Option<String>,
    pub sha256: Option<String>,
    pub artifact_bytes: Option<u64>,
    /// For single-file GGUF, `gguf:<artifact sha256>` binds its embedded tokenizer.
    pub tokenizer_identity: Option<String>,
    pub quantization: Option<String>,
    pub architecture: Option<String>,
    pub context_length: Option<u64>,
    pub languages: Vec<String>,
    pub format: String,
    pub access: String,
    pub license: Option<String>,
    pub state: StateRequirements,
}

/// Architecture-specific metadata, never inferred from parameter count or name.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StateRequirements {
    Transformer {
        layers: u64,
        kv_heads: u64,
        key_length: u64,
        value_length: u64,
    },
    /// b10726 short-convolution layout, with float32 recurrent rows. All layer
    /// counts and dimensions must come from the exact model metadata.
    Lfm2 {
        attention_layers: u64,
        convolution_layers: u64,
        kv_heads: u64,
        key_length: u64,
        value_length: u64,
        embedding_length: u64,
        convolution_cache_length: u64,
    },
    /// Hybrid/recurrent cache layouts vary with runtime checkpoint policy. A
    /// generic KV formula must not make such a model appear to fit.
    Recurrent,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationSettings {
    pub runtime_identity: String,
    pub runtime_version: String,
    pub runtime_architecture: String,
    #[serde(default)]
    pub required_cpu_features: Vec<String>,
    pub backend: String,
    pub context_tokens: u64,
    pub batch_tokens: u64,
    pub threads: u64,
    pub parallel_sequences: u64,
    pub cache_element_bytes: u64,
    #[serde(default)]
    pub recurrent_snapshots: u64,
    #[serde(default)]
    pub context_checkpoints: u64,
    pub prompt_format: String,
    /// Hash of all generation/guard options, including stop and sampling behavior.
    pub generation_settings_sha256: String,
    pub languages: Vec<String>,
    pub evaluation_version: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceIdentity {
    pub candidate_sha256: String,
    pub device_fingerprint: String,
    pub settings_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LanguageMeasurement {
    pub language: String,
    /// All assigned cases, including errors, timeouts, abstentions and missing rows.
    pub total: u64,
    pub reviewed: u64,
    pub useful_on_time: u64,
    pub harmful: u64,
    pub generic_or_incorrect: u64,
    pub abstained_or_failed: u64,
    pub p50_complete_word_ms: u64,
    pub p95_complete_word_ms: u64,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationEvidence {
    pub identity: EvidenceIdentity,
    pub measured_at_unix_s: u64,
    pub loaded_and_exercised: bool,
    pub actual_backend: String,
    pub artifact_verified: bool,
    pub cold_start_ms: u64,
    pub preparation_ms: u64,
    pub peak_rss_bytes: u64,
    pub peak_vram_bytes: Option<u64>,
    pub sustained_seconds: u64,
    pub cancellation_recovery_passed: bool,
    pub cleanup_verified: bool,
    pub prompt_reuse_measured: bool,
    pub paced_typing_measured: bool,
    pub confirmation_set_sha256: String,
    pub confirmation_untouched: bool,
    pub review_protocol_sha256: String,
    pub languages: Vec<LanguageMeasurement>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentInput {
    pub candidate: CandidateMetadata,
    pub device: DeviceInspection,
    pub settings: QualificationSettings,
    pub evidence: Option<QualificationEvidence>,
    pub now_unix_s: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceEstimate {
    pub weights_bytes: u64,
    pub state_bytes: Option<u64>,
    pub runtime_buffers_bytes: u64,
    pub backend_overhead_bytes: u64,
    pub host_reserve_bytes: u64,
    pub required_host_bytes: Option<u64>,
    pub available_host_bytes: Option<u64>,
    pub download_required_bytes: u64,
    pub assumptions: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualificationStage {
    pub state: String,
    pub passed: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assessment {
    pub schema: String,
    pub candidate_id: String,
    pub identity: EvidenceIdentity,
    pub stages: Vec<QualificationStage>,
    pub estimate: ResourceEstimate,
    pub recommended: bool,
    pub evidence_valid: bool,
    pub language_measurements: Vec<LanguageMeasurement>,
    pub limits: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recommendation {
    pub schema: String,
    pub status: String,
    pub candidate_id: Option<String>,
    pub ranking: Vec<String>,
    pub reasons: Vec<String>,
}

#[must_use]
pub fn evidence_identity(
    candidate: &CandidateMetadata,
    device: &DeviceInspection,
    settings: &QualificationSettings,
) -> EvidenceIdentity {
    EvidenceIdentity {
        candidate_sha256: digest(candidate),
        device_fingerprint: device::fingerprint(device),
        settings_sha256: digest(settings),
    }
}

#[must_use]
pub fn assess_current(
    candidate: CandidateMetadata,
    settings: QualificationSettings,
    evidence: Option<QualificationEvidence>,
    cache_path: &Path,
) -> Assessment {
    assess(&AssessmentInput {
        candidate,
        device: inspect_device(cache_path),
        settings,
        evidence,
        now_unix_s: now_unix_s(),
    })
}

fn now_unix_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_secs())
}

fn digest(value: &impl Serialize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    // These structs only contain JSON-representable primitives and vectors.
    let bytes = serde_json::to_vec(value).expect("qualification types serialize");
    let mut result = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        result.push(char::from(HEX[usize::from(byte >> 4)]));
        result.push(char::from(HEX[usize::from(byte & 15)]));
    }
    result
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Hard gates precede quality-first ranking. This function does no network IO,
/// loading, or publication; supplied measurements must be collected locally.
#[must_use]
pub fn assess(input: &AssessmentInput) -> Assessment {
    let identity = evidence_identity(&input.candidate, &input.device, &input.settings);
    let estimate = estimate_resources(input);
    let mut fit_reasons = compatibility_reasons(input);
    check_resources(input, &estimate, &mut fit_reasons);
    let estimated_fit = fit_reasons.is_empty();
    let evidence_reasons = validate_evidence(input, &identity);
    let evidence_valid = evidence_reasons.is_empty();
    let evidence = input.evidence.as_ref().filter(|_| evidence_valid);
    let mut exercise_reasons = evidence_reasons;
    if let Some(evidence) = evidence {
        if !evidence.loaded_and_exercised || !evidence.artifact_verified {
            exercise_reasons.push(
                "The exact artifact has not been verified, loaded and exercised locally."
                    .to_owned(),
            );
        }
        if evidence.actual_backend != input.settings.backend {
            exercise_reasons
                .push("The exercised backend differs from the requested backend.".to_owned());
        }
        if !evidence.cleanup_verified {
            exercise_reasons.push("Owned runtime cleanup has not been verified.".to_owned());
        }
    }
    let exercised = exercise_reasons.is_empty();
    let mut performance_reasons = performance_reasons(input, evidence);
    let mut quality_reasons = quality_reasons(input, evidence);
    if !exercised {
        let reason = "Verified local exercise and owned cleanup are required before this measured stage can pass.";
        performance_reasons.push(reason.to_owned());
        quality_reasons.push(reason.to_owned());
    }
    let performance = exercised && performance_reasons.is_empty();
    let quality = exercised && quality_reasons.is_empty();
    let recommended = estimated_fit && performance && quality;
    let mut recommendation_reasons = Vec::new();
    if !estimated_fit {
        recommendation_reasons.push("Current compatibility/resource gates failed.".to_owned());
    }
    if !performance {
        recommendation_reasons
            .push("Measured performance requirements have not passed.".to_owned());
    }
    if !quality {
        recommendation_reasons
            .push("Measured prediction-quality requirements have not passed.".to_owned());
    }
    Assessment {
        schema: SCHEMA.to_owned(),
        candidate_id: input.candidate.id.clone(),
        identity,
        stages: vec![
            stage("discovered", !input.candidate.id.is_empty(), Vec::new()),
            stage("estimated_fit", estimated_fit, fit_reasons),
            stage("loaded_and_exercised", exercised, exercise_reasons),
            stage("meets_performance", performance, performance_reasons),
            stage("meets_prediction_quality", quality, quality_reasons),
            stage("recommended", recommended, recommendation_reasons),
        ],
        estimate,
        recommended,
        evidence_valid,
        language_measurements: evidence.map_or_else(Vec::new, |value| value.languages.clone()),
        limits: vec![
            "Fit is an estimate with explicit headroom, not a runtime allocation guarantee.".to_owned(),
            "Typing target is complete-word p95 <= 550 ms over all requests; diagnostic budgets do not pass it.".to_owned(),
            "Confirmation requires >=40 reviewed independent cases per requested language, >=60% useful on-time full additions, zero harmful suggestions, and 30 minutes of sustained exercise.".to_owned(),
            "Generic words, incorrect tails, missing outputs and abstentions do not earn useful credit. Review labels require inspection of the full displayed addition.".to_owned(),
            "This is local model qualification; application editing authority, acceptance and native undo require separate physical validation.".to_owned(),
        ],
    }
}

fn stage(state: &str, passed: bool, reasons: Vec<String>) -> QualificationStage {
    QualificationStage {
        state: state.to_owned(),
        passed,
        reasons,
    }
}

fn candidate_reasons(candidate: &CandidateMetadata) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.id.is_empty()
        || candidate.id.len() > 512
        || candidate.id.chars().any(char::is_control)
    {
        reasons.push("Model identity is missing or invalid.".to_owned());
    }
    if candidate.format != "gguf" {
        reasons.push("This local runtime requires a GGUF artifact; remote code and other formats are not loaded.".to_owned());
    }
    if candidate.access != "public" {
        reasons.push("Artifact access is restricted or unknown; this public discovery workflow cannot load it.".to_owned());
    }
    if candidate.license.as_ref().is_none_or(String::is_empty) {
        reasons
            .push("License metadata is missing; inspect the source repository first.".to_owned());
    }
    if candidate.architecture.as_ref().is_none_or(String::is_empty) {
        reasons.push("Model architecture is unknown.".to_owned());
    }
    if candidate.revision.as_ref().is_none_or(|value| {
        value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
    }) || candidate
        .sha256
        .as_ref()
        .is_none_or(|value| !valid_hash(value))
    {
        reasons.push("An immutable repository revision and exact SHA-256 are required.".to_owned());
    }
    if candidate.tokenizer_identity.as_ref().is_none_or(|value| {
        !value
            .strip_prefix("sha256:")
            .or_else(|| value.strip_prefix("gguf:"))
            .is_some_and(valid_hash)
    }) {
        reasons.push(
            "Tokenizer identity is not bound to verified tokenizer or GGUF bytes.".to_owned(),
        );
    }
    if candidate.quantization.as_ref().is_none_or(String::is_empty) {
        reasons.push("Quantization is unknown.".to_owned());
    }
    if candidate
        .artifact_bytes
        .is_none_or(|value| value == 0 || value > 8 * 1024 * MIB)
    {
        reasons
            .push("Artifact size is unknown or exceeds the 8 GiB Lab artifact limit.".to_owned());
    }
    reasons
}

fn compatibility_reasons(input: &AssessmentInput) -> Vec<String> {
    let candidate = &input.candidate;
    let settings = &input.settings;
    let mut reasons = candidate_reasons(candidate);
    if !matches!(input.device.architecture.as_str(), "x86_64" | "aarch64") {
        reasons.push("CPU architecture has no supported local runtime baseline.".to_owned());
    }
    if settings.runtime_architecture != input.device.architecture {
        reasons.push("Runtime binary architecture differs from this CPU architecture.".to_owned());
    }
    if settings.required_cpu_features.len() > 64
        || settings
            .required_cpu_features
            .iter()
            .any(|feature| !input.device.cpu.features.contains(feature))
    {
        reasons
            .push("The CPU does not expose every required runtime instruction feature.".to_owned());
    }
    if !(128..=131_072).contains(&settings.context_tokens)
        || !(1..=4).contains(&settings.parallel_sequences)
        || !matches!(settings.cache_element_bytes, 2 | 4)
        || settings.batch_tokens == 0
        || settings.batch_tokens > settings.context_tokens
        || settings.threads == 0
        || settings.threads > input.device.cpu.logical_cpus as u64
        || settings.recurrent_snapshots > 256
        || settings.context_checkpoints > 256
    {
        reasons.push("Runtime context, batch, threads, cache precision or checkpoint settings are unsupported.".to_owned());
    }
    if candidate
        .context_length
        .is_none_or(|limit| settings.context_tokens > limit)
    {
        reasons.push("Requested context exceeds the known model context contract, or that contract is unknown.".to_owned());
    }
    if !valid_hash(&settings.runtime_identity)
        || settings.runtime_version.is_empty()
        || !valid_hash(&settings.generation_settings_sha256)
        || settings.prompt_format.is_empty()
    {
        reasons.push(
            "Verified runtime identity and complete prompt/generation settings are required."
                .to_owned(),
        );
    }
    if settings.evaluation_version != EVALUATION_VERSION {
        reasons.push(
            "Evaluation version is unsupported; stale evidence cannot qualify this configuration."
                .to_owned(),
        );
    }
    if settings.languages.is_empty()
        || settings.languages.len() > 16
        || settings
            .languages
            .iter()
            .any(|language| !matches!(language.as_str(), "en" | "de" | "fa"))
        || settings
            .languages
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != settings.languages.len()
    {
        reasons.push("Request a distinct supported language set (en, de, fa).".to_owned());
    }
    if settings.backend != "cpu" {
        reasons.push("GPU execution is not yet qualified by this CPU estimator; detected or shared memory cannot grant acceleration.".to_owned());
    }
    reasons
}

fn estimate_resources(input: &AssessmentInput) -> ResourceEstimate {
    let weights = input.candidate.artifact_bytes.unwrap_or(0);
    let state = estimate_state(&input.candidate, &input.settings);
    // An explicit policy allowance, not a claimed architecture-derived exact
    // allocation. Larger batches need more activation/logit workspace.
    let buffers = (512 * MIB)
        .max(weights / 4)
        .saturating_add(input.settings.batch_tokens.saturating_mul(MIB));
    let overhead = 256 * MIB;
    let reserve = (2048 * MIB).max(input.device.total_memory_bytes.unwrap_or(0) / 5);
    let required = state
        .and_then(|state| weights.checked_add(state))
        .and_then(|value| value.checked_add(buffers))
        .and_then(|value| value.checked_add(overhead));
    ResourceEstimate {
        weights_bytes: weights,
        state_bytes: state,
        runtime_buffers_bytes: buffers,
        backend_overhead_bytes: overhead,
        host_reserve_bytes: reserve,
        required_host_bytes: required,
        available_host_bytes: input.device.available_memory_bytes.map(|value| value.saturating_sub(reserve)),
        download_required_bytes: weights.saturating_add(64 * MIB),
        assumptions: vec![
            "Entire artifact is charged to host RAM, even with mmap; no GPU capacity is added.".to_owned(),
            "Standard attention state uses layers * padded context * KV heads * (key+value dimensions) * cache bytes * sequences.".to_owned(),
            "LFM2 additionally uses float32 hidden*(conv_cache-1) rows for convolution layers and explicit recurrent/checkpoint settings; only reviewed b10726 layout is accepted.".to_owned(),
            "Other recurrent/hybrid or unknown state is not guessed. Runtime workspace allowance is max(512 MiB, weights/4) + batch MiB; runtime overhead is 256 MiB.".to_owned(),
            "Normal-use reserve is max(2 GiB, 20% total RAM); a fresh download needs artifact size + 64 MiB free disk, even when an already cached artifact may avoid that cost.".to_owned(),
        ],
    }
}

fn product(values: &[u64]) -> Option<u64> {
    values
        .iter()
        .try_fold(1_u64, |value, factor| value.checked_mul(*factor))
}

fn attention_state(
    layers: u64,
    heads: u64,
    key: u64,
    value: u64,
    settings: &QualificationSettings,
) -> Option<u64> {
    if !(1..=4096).contains(&layers)
        || !(1..=512).contains(&heads)
        || !(1..=65_536).contains(&key)
        || !(1..=65_536).contains(&value)
    {
        return None;
    }
    let context = settings.context_tokens.checked_add(255)? / 256 * 256;
    product(&[
        layers,
        context,
        heads,
        key.checked_add(value)?,
        settings.cache_element_bytes,
        settings.parallel_sequences,
    ])
}

fn estimate_state(candidate: &CandidateMetadata, settings: &QualificationSettings) -> Option<u64> {
    let architecture = candidate.architecture.as_deref()?;
    let bytes = match candidate.state {
        StateRequirements::Transformer {
            layers,
            kv_heads,
            key_length,
            value_length,
        } => {
            if !matches!(
                architecture,
                "llama"
                    | "qwen2"
                    | "qwen3"
                    | "granite"
                    | "granitemoe"
                    | "granitemoehybrid"
                    | "gemma"
                    | "gemma2"
                    | "phi2"
                    | "phi3"
                    | "gpt2"
            ) {
                return None;
            }
            attention_state(layers, kv_heads, key_length, value_length, settings)?
        }
        StateRequirements::Lfm2 {
            attention_layers,
            convolution_layers,
            kv_heads,
            key_length,
            value_length,
            embedding_length,
            convolution_cache_length,
        } => {
            if architecture != "lfm2"
                || settings.runtime_version != "b10726"
                || !(1..=4096).contains(&convolution_layers)
                || !(1..=65_536).contains(&embedding_length)
                || !(2..=4096).contains(&convolution_cache_length)
            {
                return None;
            }
            let attention = attention_state(
                attention_layers,
                kv_heads,
                key_length,
                value_length,
                settings,
            )?;
            // llama-hparams.cpp n_embd_r; llama-memory-recurrent.cpp n_rows.
            // 4 KiB per tensor covers alignment; runtime overhead covers GGML metadata.
            let recurrent = product(&[
                convolution_layers,
                embedding_length,
                convolution_cache_length - 1,
                4,
                settings.parallel_sequences,
                settings.recurrent_snapshots.checked_add(1)?,
            ])?
            .checked_add(convolution_layers.checked_mul(4096)?)?;
            attention.checked_add(recurrent)?
        }
        StateRequirements::Recurrent | StateRequirements::Unknown => return None,
    };
    // Saved server context checkpoints can retain full state in host RAM.
    bytes.checked_mul(settings.context_checkpoints.checked_add(1)?)
}

fn check_resources(
    input: &AssessmentInput,
    estimate: &ResourceEstimate,
    reasons: &mut Vec<String>,
) {
    if input.device.inspected_at_unix_s > input.now_unix_s
        || input
            .now_unix_s
            .saturating_sub(input.device.inspected_at_unix_s)
            > 60
    {
        reasons.push(
            "Device snapshot is stale; recheck available resources immediately before loading."
                .to_owned(),
        );
    }
    match (
        input.device.total_memory_bytes,
        input.device.available_memory_bytes,
    ) {
        (Some(total), Some(available)) if total > 0 && available <= total => (),
        _ => reasons.push("Total/available RAM is unknown or inconsistent.".to_owned()),
    }
    match (estimate.required_host_bytes, estimate.available_host_bytes) {
        (Some(required), Some(available)) if required <= available => (),
        (None, _) => reasons.push("Architecture-specific cache/recurrent-state memory is unknown or overflowed; load is blocked.".to_owned()),
        _ => reasons.push("Model, state and runtime allowances exceed RAM after normal laptop headroom.".to_owned()),
    }
    if input
        .device
        .disk
        .available_bytes
        .is_none_or(|bytes| bytes < estimate.download_required_bytes)
    {
        reasons.push(
            "Free cache disk is unknown or insufficient for the artifact and transfer headroom."
                .to_owned(),
        );
    }
}

fn validate_evidence(input: &AssessmentInput, identity: &EvidenceIdentity) -> Vec<String> {
    let Some(evidence) = &input.evidence else {
        return vec!["No locally measured evidence is attached; metadata only establishes estimated compatibility.".to_owned()];
    };
    let mut reasons = Vec::new();
    if &evidence.identity != identity {
        reasons.push("Evidence identity changed: model revision/hash/tokenizer/quantization, hardware/power, runtime/backend, settings, languages or evaluation version.".to_owned());
    }
    if evidence.measured_at_unix_s > input.now_unix_s
        || input.now_unix_s.saturating_sub(evidence.measured_at_unix_s) > 30 * 24 * 3600
    {
        reasons.push(
            "Evidence is future-dated or older than 30 days; repeat local measurements.".to_owned(),
        );
    }
    if evidence
        .languages
        .iter()
        .map(|row| &row.language)
        .collect::<std::collections::BTreeSet<_>>()
        .len()
        != evidence.languages.len()
        || evidence
            .languages
            .iter()
            .any(|row| !input.settings.languages.contains(&row.language) || !valid_counts(row))
    {
        reasons.push(
            "Evidence has duplicate/unrequested languages, invalid counts or omitted outcomes."
                .to_owned(),
        );
    }
    reasons
}

fn valid_counts(row: &LanguageMeasurement) -> bool {
    let categories = row
        .useful_on_time
        .checked_add(row.harmful)
        .and_then(|value| value.checked_add(row.generic_or_incorrect))
        .and_then(|value| value.checked_add(row.abstained_or_failed));
    row.total > 0
        && row.reviewed <= row.total
        && categories == Some(row.total)
        && row.p50_complete_word_ms <= row.p95_complete_word_ms
}

fn performance_reasons(
    input: &AssessmentInput,
    evidence: Option<&QualificationEvidence>,
) -> Vec<String> {
    let Some(evidence) = evidence else {
        return vec!["Performance is unmeasured for this exact identity.".to_owned()];
    };
    let mut reasons = missing_language_measurements(input, evidence);
    for row in &evidence.languages {
        if row.p95_complete_word_ms > 550 || row.p95_complete_word_ms == 0 {
            reasons.push(format!(
                "{} complete-word p95 is {} ms; target is <=550 ms including all requests.",
                row.language, row.p95_complete_word_ms
            ));
        }
    }
    if evidence.cold_start_ms == 0 || evidence.preparation_ms == 0 {
        reasons.push("Cold startup and preparation must be measured separately.".to_owned());
    }
    if evidence.sustained_seconds < 1800 {
        reasons.push("Less than 30 minutes of sustained resource-pressure measurement.".to_owned());
    }
    if !evidence.cancellation_recovery_passed
        || !evidence.prompt_reuse_measured
        || !evidence.paced_typing_measured
    {
        reasons.push(
            "Cancellation/recovery, actual prompt reuse and paced typing must each be exercised."
                .to_owned(),
        );
    }
    let budget = input
        .device
        .total_memory_bytes
        .unwrap_or(0)
        .saturating_sub((2048 * MIB).max(input.device.total_memory_bytes.unwrap_or(0) / 5));
    if evidence.peak_rss_bytes == 0 || evidence.peak_rss_bytes > budget {
        reasons.push(
            "Measured peak resident memory is missing or exceeds normal-use memory capacity."
                .to_owned(),
        );
    }
    reasons
}

fn quality_reasons(
    input: &AssessmentInput,
    evidence: Option<&QualificationEvidence>,
) -> Vec<String> {
    let Some(evidence) = evidence else {
        return vec!["Prediction quality is unmeasured for this exact identity.".to_owned()];
    };
    let mut reasons = missing_language_measurements(input, evidence);
    if !evidence.confirmation_untouched
        || !valid_hash(&evidence.confirmation_set_sha256)
        || !valid_hash(&evidence.review_protocol_sha256)
    {
        reasons.push(
            "An untouched confirmation set and frozen full-addition review protocol are required."
                .to_owned(),
        );
    }
    for row in &evidence.languages {
        if row.total < 40 || row.reviewed != row.total {
            reasons.push(format!("{} requires >=40 confirmation cases and review of every assigned outcome ({} total, {} reviewed).", row.language, row.total, row.reviewed));
        }
        if u128::from(row.useful_on_time) * 100 < u128::from(row.total) * 60 {
            reasons.push(format!(
                "{} useful on-time full additions: {}/{}; at least 60% required.",
                row.language, row.useful_on_time, row.total
            ));
        }
        if row.harmful != 0 {
            reasons.push(format!(
                "{} has {} harmful suggestions; the gate permits zero.",
                row.language, row.harmful
            ));
        }
    }
    reasons
}

fn missing_language_measurements(
    input: &AssessmentInput,
    evidence: &QualificationEvidence,
) -> Vec<String> {
    input.settings.languages.iter().filter(|language| !evidence.languages.iter().any(|row| &row.language == *language))
        .map(|language| format!("{language} has no complete measured case denominator; this language remains unqualified."))
        .collect()
}

/// Quality lexicographically precedes latency and memory: speed cannot offset
/// harmful or incorrect suggestions. Only candidates passing every gate enter.
#[must_use]
pub fn rank(assessments: &[Assessment]) -> Recommendation {
    let mut eligible: Vec<_> = assessments
        .iter()
        .filter(|assessment| assessment.recommended)
        .collect();
    eligible.sort_by(|left, right| {
        minimum_useful_rate(right)
            .cmp(&minimum_useful_rate(left))
            .then_with(|| useful_rate(right).cmp(&useful_rate(left)))
            .then_with(|| {
                right
                    .language_measurements
                    .len()
                    .cmp(&left.language_measurements.len())
            })
            .then_with(|| worst_latency(left).cmp(&worst_latency(right)))
            .then_with(|| {
                left.estimate
                    .required_host_bytes
                    .cmp(&right.estimate.required_host_bytes)
            })
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    Recommendation {
        schema: "badi.model-recommendation.v1".to_owned(),
        status: if eligible.is_empty() { "no_qualified_model" } else { "recommended" }.to_owned(),
        candidate_id: eligible.first().map(|candidate| candidate.candidate_id.clone()),
        ranking: eligible.iter().map(|candidate| candidate.candidate_id.clone()).collect(),
        reasons: vec![if eligible.is_empty() {
            "No candidate passed current resource, identity, performance and prediction-quality gates. Preserve the installed model and show experiment results without promotion."
        } else {
            "All ranked candidates passed every hard gate. Order: worst-language useful on-time yield, overall useful yield, language coverage, complete-word p95, then estimated memory."
        }.to_owned()],
    }
}

fn minimum_useful_rate(assessment: &Assessment) -> u128 {
    assessment
        .language_measurements
        .iter()
        .map(|row| u128::from(row.useful_on_time) * 1_000_000 / u128::from(row.total.max(1)))
        .min()
        .unwrap_or(0)
}
fn useful_rate(assessment: &Assessment) -> u128 {
    let useful: u128 = assessment
        .language_measurements
        .iter()
        .map(|row| u128::from(row.useful_on_time))
        .sum();
    let total: u128 = assessment
        .language_measurements
        .iter()
        .map(|row| u128::from(row.total))
        .sum();
    useful * 1_000_000 / total.max(1)
}
fn worst_latency(assessment: &Assessment) -> u64 {
    assessment
        .language_measurements
        .iter()
        .map(|row| row.p95_complete_word_ms)
        .max()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests;
