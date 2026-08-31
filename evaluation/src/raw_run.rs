use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

use crate::semantic::candidate::RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT;
use crate::semantic::client::{ClientError, CompletionDisposition, ObservedCompletion};
use crate::semantic::runtime::{RuntimeLifecycleObservation, StableRuntimeIdentity};

pub const RAW_RUN_SCHEMA: &str = "badi.semantic-raw-run.v1";
pub const EVALUATION_RECEIPT_SCHEMA: &str = "badi.semantic-evaluation-receipt.v1";
pub const EVALUATOR_ID: &str = "badi.semantic-evaluator.v1";

pub const CHECK_SCOPE_GUARD: &str = "badi.semantic.scope_guard.en_only.v1";
pub const CHECK_OUTPUT_SCRIPT: &str = "badi.semantic.output_script.en_only.v1";
pub const CHECK_STREAMING_TTFT: &str = "badi.semantic.streaming_ttft.v1";
pub const CHECK_RUNTIME_OWNERSHIP: &str = "badi.semantic.runtime_ownership.v1";
pub const CHECK_RAW_RUN_DERIVATION: &str = "badi.semantic.raw_run_derivation.v1";
pub const CHECK_PROVENANCE: &str = "badi.semantic.provenance.v1";
pub const FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT: &str =
    "fixture-executable-pinned; no-reviewed-sibling-bundle; system-dsos-platform-dependencies";
#[allow(dead_code)]
pub const CHECK_NO_NORMAL_BINARY: &str = "badi.semantic.no_normal_binary.v1";

const SHA256_HEX_BYTES: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceClass {
    DevelopmentFixture,
    DevelopmentCandidate,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckClass {
    General,
    ScopeGuardRejected,
    InvalidScriptRejected,
    StreamingSuggested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaseDisposition {
    Suggested,
    ModelAbstained,
    LanguageAbstained,
    InvalidOutput,
    Truncated,
    Cancelled,
    TimedOut,
    BackendError,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CaseObservation {
    case_id: String,
    check_class: CheckClass,
    disposition: CaseDisposition,
    ttft_micros: Option<u64>,
    elapsed_micros: u64,
    // `None` means the client failed before it could return trustworthy
    // counters. Exact zero remains reserved for observed no-transport paths.
    request_body_bytes: Option<u64>,
    response_body_bytes: Option<u64>,
    output_chars: u64,
}

impl CaseObservation {
    pub(crate) fn from_completion(
        case_id: impl Into<String>,
        check_class: CheckClass,
        completion: &ObservedCompletion,
    ) -> Self {
        Self {
            case_id: case_id.into(),
            check_class,
            disposition: match completion.disposition() {
                CompletionDisposition::Suggested => CaseDisposition::Suggested,
                CompletionDisposition::ModelAbstained => CaseDisposition::ModelAbstained,
                CompletionDisposition::LanguageAbstained => CaseDisposition::LanguageAbstained,
                CompletionDisposition::InvalidOutput => CaseDisposition::InvalidOutput,
                CompletionDisposition::Truncated => CaseDisposition::Truncated,
            },
            ttft_micros: completion.ttft().map(duration_micros),
            elapsed_micros: duration_micros(completion.elapsed()),
            request_body_bytes: Some(usize_to_u64(completion.request_body_bytes())),
            response_body_bytes: Some(usize_to_u64(completion.response_body_bytes())),
            output_chars: completion
                .output()
                .map_or(0, |output| usize_to_u64(output.chars().count())),
        }
    }

    pub(crate) fn from_error(
        case_id: impl Into<String>,
        check_class: CheckClass,
        error: &ClientError,
        elapsed: Duration,
    ) -> Self {
        let disposition = match error {
            ClientError::Cancelled => CaseDisposition::Cancelled,
            ClientError::Timeout => CaseDisposition::TimedOut,
            _ => CaseDisposition::BackendError,
        };
        Self {
            case_id: case_id.into(),
            check_class,
            disposition,
            ttft_micros: None,
            elapsed_micros: duration_micros(elapsed),
            request_body_bytes: None,
            response_body_bytes: None,
            output_chars: 0,
        }
    }

    #[must_use]
    pub fn case_id(&self) -> &str {
        &self.case_id
    }

    #[cfg(test)]
    #[allow(dead_code)]
    #[must_use]
    pub const fn disposition(&self) -> CaseDisposition {
        self.disposition
    }

    #[cfg(test)]
    #[allow(dead_code)]
    #[must_use]
    pub const fn request_body_bytes(&self) -> Option<u64> {
        self.request_body_bytes
    }

    #[cfg(test)]
    #[allow(dead_code)]
    #[must_use]
    pub const fn response_body_bytes(&self) -> Option<u64> {
        self.response_body_bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct Rate {
    pub numerator: u64,
    pub denominator: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AggregateMetrics {
    pub case_count: u64,
    pub suggested: Rate,
    pub language_abstained: Rate,
    pub model_abstained: Rate,
    pub invalid_output: Rate,
    pub truncated: Rate,
    pub cancelled: Rate,
    pub timed_out: Rate,
    pub backend_error: Rate,
    pub ttft_observation_count: u64,
    pub ttft_micros_p50: Option<u64>,
    pub ttft_micros_p95: Option<u64>,
    pub elapsed_observation_count: u64,
    pub elapsed_micros_p50: Option<u64>,
    pub elapsed_micros_p95: Option<u64>,
    /// `None` if any case lacks a trustworthy request-body observation.
    pub request_body_bytes: Option<u64>,
    /// `None` if any case lacks a trustworthy response-body observation.
    pub response_body_bytes: Option<u64>,
    pub output_chars: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RawRun {
    schema: &'static str,
    evaluator_id: &'static str,
    run_id: Uuid,
    created_at_unix_ms: u64,
    evidence_class: EvidenceClass,
    corpus_sha256: String,
    observations: Vec<CaseObservation>,
}

impl RawRun {
    pub(crate) fn from_observations(
        evidence_class: EvidenceClass,
        corpus_sha256: impl Into<String>,
        observations: Vec<CaseObservation>,
    ) -> Result<Self, EvaluationError> {
        let corpus_sha256 = corpus_sha256.into();
        if !is_lower_hex(&corpus_sha256, SHA256_HEX_BYTES) {
            return Err(EvaluationError::InvalidIdentity("corpus_sha256"));
        }
        if observations.is_empty() {
            return Err(EvaluationError::EmptyRun);
        }
        let mut ids = HashSet::with_capacity(observations.len());
        for observation in &observations {
            if !valid_case_id(observation.case_id()) || !ids.insert(observation.case_id()) {
                return Err(EvaluationError::InvalidCaseId);
            }
        }
        Ok(Self {
            schema: RAW_RUN_SCHEMA,
            evaluator_id: EVALUATOR_ID,
            run_id: Uuid::new_v4(),
            created_at_unix_ms: current_unix_ms()?,
            evidence_class,
            corpus_sha256,
            observations,
        })
    }

    #[must_use]
    pub fn observations(&self) -> &[CaseObservation] {
        &self.observations
    }

    #[must_use]
    pub fn corpus_sha256(&self) -> &str {
        &self.corpus_sha256
    }

    #[must_use]
    pub fn aggregate(&self) -> AggregateMetrics {
        aggregate(&self.observations)
    }

    pub fn sha256(&self) -> Result<String, EvaluationError> {
        let canonical = serde_json::to_vec(self).map_err(EvaluationError::Serialize)?;
        Ok(sha256_bytes(&canonical))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CandidateModelProvenance {
    pub(crate) artifact_filename: String,
    pub(crate) artifact_sha256: String,
    pub(crate) artifact_bytes: u64,
    pub(crate) quantization: String,
    pub(crate) quantizer_repository: String,
    pub(crate) quantizer_revision: String,
    pub(crate) upstream_base_repository: String,
    pub(crate) upstream_base_revision: Option<String>,
    pub(crate) upstream_base_revision_status: RevisionStatus,
    pub(crate) tokenizer_repository: String,
    pub(crate) tokenizer_revision: Option<String>,
    pub(crate) tokenizer_revision_status: RevisionStatus,
    pub(crate) license: String,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisionStatus {
    Exact,
    Unreported,
    EmbeddedInArtifact,
}

impl CandidateModelProvenance {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if self.artifact_filename.is_empty()
            || self.artifact_filename.contains(['/', '\\'])
            || Path::new(&self.artifact_filename).extension() != Some(OsStr::new("gguf"))
        {
            return Err(EvaluationError::InvalidIdentity("artifact_filename"));
        }
        if !is_lower_hex(&self.artifact_sha256, SHA256_HEX_BYTES) {
            return Err(EvaluationError::InvalidIdentity("artifact_sha256"));
        }
        if self.artifact_bytes == 0 {
            return Err(EvaluationError::InvalidIdentity("artifact_bytes"));
        }
        if self.quantization.is_empty() || self.quantization.len() > 32 {
            return Err(EvaluationError::InvalidIdentity("quantization"));
        }
        for (name, repository) in [
            ("quantizer_repository", &self.quantizer_repository),
            ("upstream_base_repository", &self.upstream_base_repository),
            ("tokenizer_repository", &self.tokenizer_repository),
        ] {
            if !valid_repository(repository) {
                return Err(EvaluationError::InvalidIdentity(name));
            }
        }
        if !is_git_revision(&self.quantizer_revision) {
            return Err(EvaluationError::InvalidIdentity("quantizer_revision"));
        }
        validate_revision(
            "upstream_base_revision",
            self.upstream_base_revision.as_deref(),
            self.upstream_base_revision_status,
        )?;
        validate_revision(
            "tokenizer_revision",
            self.tokenizer_revision.as_deref(),
            self.tokenizer_revision_status,
        )?;
        if self.license.is_empty()
            || self.license.len() > 64
            || self.license.chars().any(char::is_control)
        {
            return Err(EvaluationError::InvalidIdentity("license"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct BackendProvenance {
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) version: String,
    pub(crate) distribution_archive_sha256: Option<String>,
    pub(crate) executed_loader_sha256: String,
    pub(crate) runtime_bundle_manifest_sha256: Option<String>,
    pub(crate) dynamic_bundle_constraint: String,
}

impl BackendProvenance {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        if !valid_repository(&self.repository) {
            return Err(EvaluationError::InvalidIdentity("backend_repository"));
        }
        if !is_git_revision(&self.revision) {
            return Err(EvaluationError::InvalidIdentity("backend_revision"));
        }
        if self.version.is_empty()
            || self.version.len() > 64
            || self.version.chars().any(char::is_control)
        {
            return Err(EvaluationError::InvalidIdentity("backend_version"));
        }
        if self
            .distribution_archive_sha256
            .as_deref()
            .is_some_and(|digest| !is_lower_hex(digest, SHA256_HEX_BYTES))
        {
            return Err(EvaluationError::InvalidIdentity(
                "backend_distribution_archive_sha256",
            ));
        }
        if !is_lower_hex(&self.executed_loader_sha256, SHA256_HEX_BYTES) {
            return Err(EvaluationError::InvalidIdentity(
                "backend_executed_loader_sha256",
            ));
        }
        if self
            .runtime_bundle_manifest_sha256
            .as_deref()
            .is_some_and(|digest| !is_lower_hex(digest, SHA256_HEX_BYTES))
        {
            return Err(EvaluationError::InvalidIdentity(
                "backend_runtime_bundle_manifest_sha256",
            ));
        }
        if self.dynamic_bundle_constraint.is_empty()
            || self.dynamic_bundle_constraint.len() > 128
            || self.dynamic_bundle_constraint.chars().any(char::is_control)
        {
            return Err(EvaluationError::InvalidIdentity(
                "backend_dynamic_bundle_constraint",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvaluationIdentity {
    pub(crate) runtime: StableRuntimeIdentity,
    pub(crate) backend: BackendProvenance,
    pub(crate) model: CandidateModelProvenance,
    pub(crate) prompt_contract_id: &'static str,
    pub(crate) prompt_contract_sha256: String,
    pub(crate) corpus_sha256: String,
}

impl EvaluationIdentity {
    pub fn validate(&self) -> Result<(), EvaluationError> {
        self.backend.validate()?;
        self.model.validate()?;
        if self.backend.executed_loader_sha256 != self.runtime.binary_sha256 {
            return Err(EvaluationError::IdentityMismatch("backend_binary"));
        }
        if self.backend.runtime_bundle_manifest_sha256
            != self.runtime.runtime_bundle_manifest_sha256
        {
            return Err(EvaluationError::IdentityMismatch("backend_runtime_bundle"));
        }
        if self.model.artifact_sha256 != self.runtime.model_sha256
            || self.model.artifact_bytes != self.runtime.model_size
        {
            return Err(EvaluationError::IdentityMismatch("model_artifact"));
        }
        if !is_lower_hex(&self.prompt_contract_sha256, SHA256_HEX_BYTES) {
            return Err(EvaluationError::InvalidIdentity("prompt_contract_sha256"));
        }
        if !is_lower_hex(&self.corpus_sha256, SHA256_HEX_BYTES) {
            return Err(EvaluationError::InvalidIdentity("corpus_sha256"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Passed,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct SemanticCheck {
    pub id: &'static str,
    pub status: CheckStatus,
    pub passing_observations: u64,
    pub required_observations: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct EvaluationReceipt {
    schema: &'static str,
    evaluator_id: &'static str,
    created_at_unix_ms: u64,
    authority: &'static str,
    production_ready: bool,
    raw_run_sha256: String,
    identity: EvaluationIdentity,
    aggregate: AggregateMetrics,
    checks: Vec<SemanticCheck>,
}

impl EvaluationReceipt {
    pub(crate) fn from_raw_run(
        raw_run: &RawRun,
        identity: EvaluationIdentity,
        lifecycle: &RuntimeLifecycleObservation,
    ) -> Result<Self, EvaluationError> {
        identity.validate()?;
        if identity.corpus_sha256 != raw_run.corpus_sha256() {
            return Err(EvaluationError::IdentityMismatch("corpus"));
        }
        if lifecycle.runtime_identity_sha256() != identity.runtime.sha256() {
            return Err(EvaluationError::IdentityMismatch("runtime_lifecycle"));
        }
        let aggregate = raw_run.aggregate();
        let checks = derive_checks(raw_run, &identity, lifecycle);
        Ok(Self {
            schema: EVALUATION_RECEIPT_SCHEMA,
            evaluator_id: EVALUATOR_ID,
            created_at_unix_ms: current_unix_ms()?,
            authority: "evaluation_only",
            production_ready: false,
            raw_run_sha256: raw_run.sha256()?,
            identity,
            aggregate,
            checks,
        })
    }

    #[cfg(test)]
    #[allow(dead_code)]
    #[must_use]
    pub fn aggregate(&self) -> &AggregateMetrics {
        &self.aggregate
    }

    #[must_use]
    pub fn checks(&self) -> &[SemanticCheck] {
        &self.checks
    }

    #[cfg(test)]
    #[allow(dead_code)]
    #[must_use]
    pub fn raw_run_sha256(&self) -> &str {
        &self.raw_run_sha256
    }
}

#[derive(Debug, Error)]
pub enum EvaluationError {
    #[error("evaluation run cannot be empty")]
    EmptyRun,
    #[error("evaluation case IDs must be unique bounded opaque values")]
    InvalidCaseId,
    #[error("invalid evaluation identity field: {0}")]
    InvalidIdentity(&'static str),
    #[error("evaluation identity mismatch: {0}")]
    IdentityMismatch(&'static str),
    #[error("system clock precedes the Unix epoch")]
    Clock,
    #[error("evaluation serialization failed")]
    Serialize(#[source] serde_json::Error),
}

fn aggregate(observations: &[CaseObservation]) -> AggregateMetrics {
    let denominator = usize_to_u64(observations.len());
    let rate = |wanted| Rate {
        numerator: usize_to_u64(
            observations
                .iter()
                .filter(|observation| observation.disposition == wanted)
                .count(),
        ),
        denominator,
    };
    let mut ttft = observations
        .iter()
        .filter_map(|observation| observation.ttft_micros)
        .collect::<Vec<_>>();
    let mut elapsed = observations
        .iter()
        .filter(|observation| observation.disposition != CaseDisposition::LanguageAbstained)
        .map(|observation| observation.elapsed_micros)
        .collect::<Vec<_>>();
    ttft.sort_unstable();
    elapsed.sort_unstable();
    AggregateMetrics {
        case_count: denominator,
        suggested: rate(CaseDisposition::Suggested),
        language_abstained: rate(CaseDisposition::LanguageAbstained),
        model_abstained: rate(CaseDisposition::ModelAbstained),
        invalid_output: rate(CaseDisposition::InvalidOutput),
        truncated: rate(CaseDisposition::Truncated),
        cancelled: rate(CaseDisposition::Cancelled),
        timed_out: rate(CaseDisposition::TimedOut),
        backend_error: rate(CaseDisposition::BackendError),
        ttft_observation_count: usize_to_u64(ttft.len()),
        ttft_micros_p50: nearest_rank(&ttft, 50),
        ttft_micros_p95: nearest_rank(&ttft, 95),
        elapsed_observation_count: usize_to_u64(elapsed.len()),
        elapsed_micros_p50: nearest_rank(&elapsed, 50),
        elapsed_micros_p95: nearest_rank(&elapsed, 95),
        request_body_bytes: sum_known_bytes(
            observations
                .iter()
                .map(|observation| observation.request_body_bytes),
        ),
        response_body_bytes: sum_known_bytes(
            observations
                .iter()
                .map(|observation| observation.response_body_bytes),
        ),
        output_chars: observations.iter().fold(0, |total, observation| {
            total.saturating_add(observation.output_chars)
        }),
    }
}

fn derive_checks(
    raw_run: &RawRun,
    identity: &EvaluationIdentity,
    lifecycle: &RuntimeLifecycleObservation,
) -> Vec<SemanticCheck> {
    let observations = raw_run.observations();
    let scope = observations
        .iter()
        .filter(|observation| observation.check_class == CheckClass::ScopeGuardRejected)
        .collect::<Vec<_>>();
    let scope_passes = scope
        .iter()
        .filter(|observation| {
            observation.disposition == CaseDisposition::LanguageAbstained
                && observation.request_body_bytes == Some(0)
                && observation.response_body_bytes == Some(0)
        })
        .count();
    let invalid_script = observations
        .iter()
        .filter(|observation| observation.check_class == CheckClass::InvalidScriptRejected)
        .collect::<Vec<_>>();
    let invalid_script_passes = invalid_script
        .iter()
        .filter(|observation| {
            observation.disposition == CaseDisposition::InvalidOutput
                && observation.output_chars == 0
        })
        .count();
    let streaming = observations
        .iter()
        .filter(|observation| observation.check_class == CheckClass::StreamingSuggested)
        .collect::<Vec<_>>();
    let streaming_passes = streaming
        .iter()
        .filter(|observation| {
            observation.disposition == CaseDisposition::Suggested
                && observation.ttft_micros.is_some_and(|ttft| ttft > 0)
                && observation
                    .ttft_micros
                    .is_some_and(|ttft| ttft <= observation.elapsed_micros)
        })
        .count();
    let provenance_passed = provenance_check_passes(raw_run.evidence_class, identity);
    vec![
        check(
            CHECK_SCOPE_GUARD,
            scope_passes,
            scope.len().max(4),
            scope.len() >= 4,
        ),
        check(
            CHECK_OUTPUT_SCRIPT,
            invalid_script_passes,
            invalid_script.len().max(3),
            invalid_script.len() >= 3,
        ),
        check(
            CHECK_STREAMING_TTFT,
            streaming_passes,
            streaming.len().max(1),
            !streaming.is_empty(),
        ),
        SemanticCheck {
            id: CHECK_RUNTIME_OWNERSHIP,
            status: if lifecycle.challenge_completed() && lifecycle.reaped() {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            },
            passing_observations: u64::from(lifecycle.challenge_completed() && lifecycle.reaped()),
            required_observations: 1,
        },
        SemanticCheck {
            id: CHECK_RAW_RUN_DERIVATION,
            status: CheckStatus::Passed,
            passing_observations: 1,
            required_observations: 1,
        },
        SemanticCheck {
            id: CHECK_PROVENANCE,
            status: if provenance_passed {
                CheckStatus::Passed
            } else {
                CheckStatus::Failed
            },
            passing_observations: u64::from(provenance_passed),
            required_observations: 1,
        },
    ]
}

fn provenance_check_passes(evidence_class: EvidenceClass, identity: &EvaluationIdentity) -> bool {
    let backend = &identity.backend;
    let runtime = &identity.runtime;
    match evidence_class {
        EvidenceClass::DevelopmentCandidate => {
            backend.distribution_archive_sha256.is_some()
                && runtime
                    .runtime_bundle_manifest_sha256
                    .as_deref()
                    .is_some_and(|digest| is_lower_hex(digest, SHA256_HEX_BYTES))
                && backend.runtime_bundle_manifest_sha256 == runtime.runtime_bundle_manifest_sha256
                && backend.dynamic_bundle_constraint == RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT
        }
        EvidenceClass::DevelopmentFixture => {
            backend.repository == "badi/semantic-fixture-backend"
                && backend.version == "fixture-v1"
                && backend.distribution_archive_sha256.is_none()
                && backend.runtime_bundle_manifest_sha256.is_none()
                && runtime.runtime_bundle_manifest_sha256.is_none()
                && backend.dynamic_bundle_constraint == FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT
        }
    }
}

fn check(
    id: &'static str,
    passing: usize,
    required: usize,
    minimum_present: bool,
) -> SemanticCheck {
    SemanticCheck {
        id,
        status: if minimum_present && passing == required {
            CheckStatus::Passed
        } else {
            CheckStatus::Failed
        },
        passing_observations: usize_to_u64(passing),
        required_observations: usize_to_u64(required),
    }
}

fn nearest_rank(sorted: &[u64], percentile: usize) -> Option<u64> {
    if sorted.is_empty() {
        return None;
    }
    let rank = sorted.len().saturating_mul(percentile).div_ceil(100).max(1);
    sorted.get(rank - 1).copied()
}

fn sum_known_bytes(mut values: impl Iterator<Item = Option<u64>>) -> Option<u64> {
    values.try_fold(0_u64, |total, value| {
        value.and_then(|value| total.checked_add(value))
    })
}

fn valid_case_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

fn valid_repository(value: &str) -> bool {
    value.split_once('/').is_some_and(|(owner, name)| {
        !owner.is_empty()
            && !name.is_empty()
            && !name.contains('/')
            && owner.bytes().all(repository_byte)
            && name.bytes().all(repository_byte)
    })
}

const fn repository_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')
}

fn is_git_revision(value: &str) -> bool {
    is_lower_hex(value, 40) || is_lower_hex(value, 64)
}

fn validate_revision(
    name: &'static str,
    revision: Option<&str>,
    status: RevisionStatus,
) -> Result<(), EvaluationError> {
    match (status, revision) {
        (RevisionStatus::Exact, Some(revision)) if is_git_revision(revision) => Ok(()),
        (RevisionStatus::Unreported | RevisionStatus::EmbeddedInArtifact, None) => Ok(()),
        _ => Err(EvaluationError::InvalidIdentity(name)),
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn current_unix_ms() -> Result<u64, EvaluationError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| EvaluationError::Clock)?;
    Ok(u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
}

fn duration_micros(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
