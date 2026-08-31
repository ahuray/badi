#![cfg(feature = "local-model-eval")]

use std::error::Error;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::time::Duration;

use badi_broker::local_model::{ProductionActivationStatus, production_activation_status};
use badi_broker::protocol::ProviderKind;
use badi_broker::provider::{CompletionProvider, ProviderRequest};
use badi_broker::semantic;
use badi_broker::semantic::candidate::RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT;
use sha2::{Digest, Sha256};
use tempfile::TempDir;
use tokio_util::sync::CancellationToken;

#[path = "../../evaluation/src/fixture_backend.rs"]
mod fixture_backend;
#[path = "../../evaluation/src/raw_run.rs"]
mod raw_run;
#[path = "../../evaluation/src/runner.rs"]
mod runner;
use fixture_backend::FixtureBackend;
use raw_run::{
    BackendProvenance, CHECK_NO_NORMAL_BINARY, CHECK_OUTPUT_SCRIPT, CHECK_PROVENANCE,
    CHECK_RAW_RUN_DERIVATION, CHECK_RUNTIME_OWNERSHIP, CHECK_SCOPE_GUARD, CHECK_STREAMING_TTFT,
    CandidateModelProvenance, CaseObservation, CheckClass, CheckStatus, EvaluationIdentity,
    EvaluationReceipt, EvidenceClass, FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT, RawRun,
    RevisionStatus,
};
use semantic::client::{
    ClientError, CompletionDisposition, HealthStatus, PROMPT_CONTRACT_ID, SemanticClient,
    SemanticClientConfig, prompt_contract_sha256,
};
use semantic::provenance::{FileExpectation, ProvenanceError, VerifiedFile, verify_file};
use semantic::runtime::{
    FIXTURE_TOKEN_CANARY, FixtureBehavior, LlamaCppLaunch, RuntimeError, StableRuntimeIdentity,
};

const FIXTURE_TOKEN: &str = "fixture-secret-canary-8fdd1d0c";
const REVISION_ONE: &str = "0000000000000000000000000000000000000001";
const REVISION_TWO: &str = "0000000000000000000000000000000000000002";
const REVISION_THREE: &str = "0000000000000000000000000000000000000003";
const REVISION_FOUR: &str = "0000000000000000000000000000000000000004";

#[tokio::test]
async fn scope_guard_sends_no_request_or_body_before_english_eligibility()
-> Result<(), Box<dyn Error>> {
    let backend = FixtureBackend::start(FIXTURE_TOKEN).await?;
    let client = fixture_client(&backend, FIXTURE_TOKEN)?;
    for language in [None, Some("fa"), Some("ar"), Some("zh")] {
        let observed = client
            .complete_observed(request("fixture:valid", language), CancellationToken::new())
            .await?;
        assert_eq!(
            observed.disposition(),
            CompletionDisposition::LanguageAbstained
        );
        assert_eq!(observed.request_body_bytes(), 0);
        assert_eq!(observed.response_body_bytes(), 0);
    }
    let malformed = client
        .complete_observed(
            request("fixture:valid", Some("en--US")),
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(malformed, Err(ClientError::InvalidRequest)));
    assert_eq!(backend.audit().connections(), 0);
    assert_eq!(backend.audit().request_body_bytes(), 0);

    let english = client
        .complete_observed(
            request("fixture:valid", Some("en-GB")),
            CancellationToken::new(),
        )
        .await?;
    assert_eq!(english.disposition(), CompletionDisposition::Suggested);
    assert_eq!(english.output(), Some(" for your time."));
    assert!(english.request_body_bytes() > 0);
    assert!(backend.audit().request_body_bytes() > 0);
    backend.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn streaming_ttft_is_first_nonempty_token_not_response_headers() -> Result<(), Box<dyn Error>>
{
    let backend = FixtureBackend::start(FIXTURE_TOKEN).await?;
    let client = fixture_client(&backend, FIXTURE_TOKEN)?;
    let observed = client
        .complete_observed(
            request("fixture:valid", Some("en-US")),
            CancellationToken::new(),
        )
        .await?;
    let ttft = observed.ttft().expect("non-empty streamed token");
    assert!(ttft >= Duration::from_millis(15), "ttft={ttft:?}");
    assert!(ttft < observed.elapsed());
    assert!(observed.elapsed() >= Duration::from_millis(40));
    backend.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn english_output_gate_rejects_non_latin_and_emoji_tokens() -> Result<(), Box<dyn Error>> {
    let backend = FixtureBackend::start(FIXTURE_TOKEN).await?;
    let client = fixture_client(&backend, FIXTURE_TOKEN)?;
    for case in ["fixture:arabic", "fixture:cjk", "fixture:emoji"] {
        let observed = client
            .complete_observed(request(case, Some("en")), CancellationToken::new())
            .await?;
        assert_eq!(observed.disposition(), CompletionDisposition::InvalidOutput);
        assert_eq!(observed.output(), None);
        assert!(observed.ttft().is_some());
    }
    backend.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn timeout_cancellation_health_token_and_error_redaction_are_bounded()
-> Result<(), Box<dyn Error>> {
    let backend = FixtureBackend::start(FIXTURE_TOKEN).await?;
    let wrong = fixture_client(&backend, "wrong-secret-canary")?;
    let health = wrong.probe_health(CancellationToken::new()).await;
    assert_eq!(health?, HealthStatus::Ready);
    let challenge = wrong
        .probe_authorization_challenge(CancellationToken::new())
        .await;
    assert!(matches!(
        challenge,
        Err(ClientError::UnexpectedStatus(
            reqwest::StatusCode::UNAUTHORIZED
        ))
    ));
    assert!(!format!("{challenge:?}").contains("wrong-secret-canary"));

    let config = SemanticClientConfig::new(backend.endpoint(), "fixture-en-v1", FIXTURE_TOKEN)?
        .with_timeouts(Duration::from_millis(50), Duration::from_millis(100))?;
    let client = SemanticClient::new(config)?;
    let timed_out = client
        .complete_observed(
            request("fixture:timeout", Some("en")),
            CancellationToken::new(),
        )
        .await;
    assert!(matches!(timed_out, Err(ClientError::Timeout)));

    let cancellation = CancellationToken::new();
    let operation =
        client.complete_observed(request("fixture:timeout", Some("en")), cancellation.clone());
    tokio::pin!(operation);
    tokio::select! {
        () = tokio::time::sleep(Duration::from_millis(20)) => cancellation.cancel(),
        result = &mut operation => panic!("fixture completed before cancellation: {result:?}"),
    }
    let cancelled = operation.await;
    assert!(matches!(cancelled, Err(ClientError::Cancelled)));
    backend.shutdown().await?;
    Ok(())
}

#[tokio::test]
async fn raw_run_aggregates_and_receipt_are_derived_from_observations() -> Result<(), Box<dyn Error>>
{
    let fixture = OwnedFixture::new()?;
    let runtime = fixture.launch(FixtureBehavior::Ready)?.spawn().await?;
    assert_eq!(CompletionProvider::kind(&runtime), ProviderKind::LocalModel);
    assert!(runtime.endpoint().ip().is_loopback());
    let process_id = runtime.process_id().expect("owned child process");
    let token = FIXTURE_TOKEN_CANARY.to_owned();
    let cmdline = fs::read(format!("/proc/{process_id}/cmdline"))?;
    assert!(!contains_bytes(&cmdline, token.as_bytes()));
    assert!(!format!("{runtime:?}").contains(&token));

    let stable_identity = runtime.identity().clone();
    assert_eq!(stable_identity.runtime_bundle_manifest_sha256, None);
    let raw_run = runner::run_development_cases(
        runtime.client(),
        &runner::fixture_cases(),
        EvidenceClass::DevelopmentFixture,
    )
    .await?;
    let candidate_run_without_bundle = runner::run_development_cases(
        runtime.client(),
        &runner::fixture_cases(),
        EvidenceClass::DevelopmentCandidate,
    )
    .await?;
    let first = raw_run.aggregate();
    let second = raw_run.aggregate();
    assert_eq!(first, second);
    assert_eq!(first.case_count, 10);
    assert_eq!(first.suggested.numerator, 1);
    assert_eq!(first.language_abstained.numerator, 4);
    assert_eq!(first.invalid_output.numerator, 3);
    assert_eq!(first.model_abstained.numerator, 1);
    assert_eq!(first.truncated.numerator, 1);
    assert_eq!(first.ttft_observation_count, 5);
    assert_eq!(first.elapsed_observation_count, 6);
    assert_eq!(
        raw_run.observations()[0].disposition(),
        raw_run::CaseDisposition::LanguageAbstained
    );
    assert!(first.request_body_bytes.is_some_and(|bytes| bytes > 0));
    assert!(first.response_body_bytes.is_some_and(|bytes| bytes > 0));

    let lifecycle = runtime.shutdown()?;
    assert_eq!(lifecycle.process_id(), process_id);
    assert!(lifecycle.reaped());
    assert_eq!(lifecycle.exit_code(), None);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    let receipt = EvaluationReceipt::from_raw_run(
        &raw_run,
        fixture_identity(&stable_identity, raw_run.corpus_sha256()),
        &lifecycle,
    )?;
    assert_eq!(receipt.aggregate(), &first);
    assert_eq!(receipt.raw_run_sha256(), raw_run.sha256()?);
    let checks = receipt
        .checks()
        .iter()
        .map(|check| (check.id, check.status))
        .collect::<Vec<_>>();
    assert_eq!(
        checks,
        vec![
            (CHECK_SCOPE_GUARD, CheckStatus::Passed),
            (CHECK_OUTPUT_SCRIPT, CheckStatus::Passed),
            (CHECK_STREAMING_TTFT, CheckStatus::Passed),
            (CHECK_RUNTIME_OWNERSHIP, CheckStatus::Passed),
            (CHECK_RAW_RUN_DERIVATION, CheckStatus::Passed),
            (CHECK_PROVENANCE, CheckStatus::Passed),
        ]
    );
    let mut candidate_identity = fixture_identity(
        &stable_identity,
        candidate_run_without_bundle.corpus_sha256(),
    );
    candidate_identity.backend.distribution_archive_sha256 =
        Some(stable_identity.binary_sha256.clone());
    candidate_identity.backend.dynamic_bundle_constraint =
        RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT.to_owned();
    let candidate_receipt = EvaluationReceipt::from_raw_run(
        &candidate_run_without_bundle,
        candidate_identity,
        &lifecycle,
    )?;
    assert_eq!(
        candidate_receipt
            .checks()
            .iter()
            .find(|check| check.id == CHECK_PROVENANCE)
            .map(|check| check.status),
        Some(CheckStatus::Failed)
    );
    let raw_json = serde_json::to_string(&raw_run)?;
    assert!(!raw_json.contains("fixture:valid"));
    assert!(!raw_json.contains(&token));
    assert!(!serde_json::to_string(&receipt)?.contains(&token));
    Ok(())
}

#[test]
fn failed_case_transport_bytes_are_explicitly_unknown() -> Result<(), Box<dyn Error>> {
    let observation = CaseObservation::from_error(
        "transport-timeout",
        CheckClass::General,
        &ClientError::Timeout,
        Duration::from_millis(10),
    );
    assert_eq!(observation.request_body_bytes(), None);
    assert_eq!(observation.response_body_bytes(), None);

    let raw_run = RawRun::from_observations(
        EvidenceClass::DevelopmentFixture,
        "a".repeat(64),
        vec![observation],
    )?;
    let aggregate = raw_run.aggregate();
    assert_eq!(aggregate.request_body_bytes, None);
    assert_eq!(aggregate.response_body_bytes, None);

    let raw_json = serde_json::to_value(raw_run)?;
    assert!(raw_json["observations"][0]["request_body_bytes"].is_null());
    assert!(raw_json["observations"][0]["response_body_bytes"].is_null());
    Ok(())
}

#[tokio::test]
async fn runtime_rejects_wrong_artifacts_bad_health_early_exit_and_orphans()
-> Result<(), Box<dyn Error>> {
    let fixture = OwnedFixture::new()?;
    let wrong_digest = FileExpectation::new(
        fixture.model.path(),
        "0".repeat(64),
        fixture.model.identity().size,
    )?;
    assert!(matches!(
        verify_file(&wrong_digest),
        Err(ProvenanceError::DigestMismatch)
    ));

    let link_path = fixture.directory.path().join("linked-model.gguf");
    symlink(fixture.model.path(), &link_path)?;
    let linked = FileExpectation::new(
        fs::canonicalize(fixture.directory.path())?.join("linked-model.gguf"),
        fixture.model.sha256(),
        fixture.model.identity().size,
    )?;
    assert!(matches!(
        verify_file(&linked),
        Err(ProvenanceError::NonCanonicalPath | ProvenanceError::Symlink)
    ));

    let early = fixture.launch(FixtureBehavior::EarlyExit)?.spawn().await;
    assert!(matches!(early, Err(RuntimeError::EarlyExit(_))));
    let malformed = fixture
        .launch(FixtureBehavior::MalformedHealth)?
        .spawn()
        .await;
    assert!(matches!(malformed, Err(RuntimeError::Health(_))));
    let timeout = fixture
        .launch(FixtureBehavior::NoBind)?
        .with_startup_timeout(Duration::from_millis(80))?
        .spawn()
        .await;
    assert!(matches!(timeout, Err(RuntimeError::StartupTimeout)));

    let runtime = fixture.launch(FixtureBehavior::Ready)?.spawn().await?;
    let process_id = runtime.process_id().expect("owned child process");
    drop(runtime);
    assert!(!Path::new(&format!("/proc/{process_id}")).exists());
    Ok(())
}

#[test]
fn evaluator_is_feature_gated_and_absent_from_normal_broker_modules() -> Result<(), Box<dyn Error>>
{
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cargo = fs::read_to_string(manifest_dir.join("Cargo.toml"))?;
    assert!(cargo.contains("name = \"badi-evaluator\""));
    assert!(cargo.contains("required-features = [\"local-model-eval\"]"));
    let library = fs::read_to_string(manifest_dir.join("src/lib.rs"))?;
    assert!(library.contains("#[cfg(feature = \"local-model-eval\")]\npub mod semantic;"));
    let main = fs::read_to_string(manifest_dir.join("src/main.rs"))?;
    assert!(!main.contains("semantic"));
    assert!(!main.contains("local_model"));
    assert!(!main.contains("badi_evaluator"));
    assert_eq!(CHECK_NO_NORMAL_BINARY, "badi.semantic.no_normal_binary.v1");
    Ok(())
}

#[test]
fn production_semantic_activation_remains_disabled_without_qualification() {
    assert_eq!(
        production_activation_status(),
        ProductionActivationStatus::AwaitingQualifiedReceipt
    );
}

#[test]
fn candidate_provenance_can_state_unreported_and_embedded_revisions_honestly() {
    assert_ne!(
        EvidenceClass::DevelopmentCandidate,
        EvidenceClass::DevelopmentFixture
    );
    assert_eq!(runner::pinned_candidate_cases().len(), 6);
    let provenance = CandidateModelProvenance {
        artifact_filename: "Qwen3-1.7B-Q4_K_M.gguf".to_owned(),
        artifact_sha256: "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5"
            .to_owned(),
        artifact_bytes: 1_282_439_264,
        quantization: "Q4_K_M".to_owned(),
        quantizer_repository: "ggml-org/Qwen3-1.7B-GGUF".to_owned(),
        quantizer_revision: "daeb8e2d528a760970442092f6bf1e55c3b659eb".to_owned(),
        upstream_base_repository: "Qwen/Qwen3-1.7B".to_owned(),
        upstream_base_revision: None,
        upstream_base_revision_status: RevisionStatus::Unreported,
        tokenizer_repository: "ggml-org/Qwen3-1.7B-GGUF".to_owned(),
        tokenizer_revision: None,
        tokenizer_revision_status: RevisionStatus::EmbeddedInArtifact,
        license: "Apache-2.0".to_owned(),
    };
    provenance.validate().expect("honest candidate provenance");
}

fn fixture_client(backend: &FixtureBackend, token: &str) -> Result<SemanticClient, ClientError> {
    let config = SemanticClientConfig::new(backend.endpoint(), "fixture-en-v1", token)?;
    assert_eq!(config.endpoint(), backend.endpoint());
    let client = SemanticClient::new(config)?;
    assert_eq!(client.config().endpoint(), backend.endpoint());
    Ok(client)
}

fn request(before: &str, language: Option<&str>) -> ProviderRequest {
    ProviderRequest {
        before: before.to_owned(),
        after: String::new(),
        language: language.map(str::to_owned),
    }
}

struct OwnedFixture {
    directory: TempDir,
    binary: VerifiedFile,
    model: VerifiedFile,
}

impl OwnedFixture {
    fn new() -> Result<Self, Box<dyn Error>> {
        let directory = tempfile::tempdir()?;
        let directory_path = fs::canonicalize(directory.path())?;
        let model_path = directory_path.join("fixture-model.gguf");
        fs::write(
            &model_path,
            b"Badi semantic evaluator fixture artifact v1\n",
        )?;
        let binary_path = fs::canonicalize(env!("CARGO_BIN_EXE_badi-evaluator"))?;
        let binary = verify_observed_file(&binary_path)?;
        let model = verify_observed_file(&model_path)?;
        Ok(Self {
            directory,
            binary,
            model,
        })
    }

    fn launch(&self, behavior: FixtureBehavior) -> Result<LlamaCppLaunch, RuntimeError> {
        LlamaCppLaunch::for_fixture(self.binary.clone(), self.model.clone(), behavior)
    }
}

fn verify_observed_file(path: &Path) -> Result<VerifiedFile, Box<dyn Error>> {
    let bytes = fs::read(path)?;
    let expectation = FileExpectation::new(
        path,
        encode_lower_hex(Sha256::digest(&bytes)),
        u64::try_from(bytes.len()).unwrap_or(u64::MAX),
    )?;
    Ok(verify_file(&expectation)?)
}

fn fixture_identity(runtime: &StableRuntimeIdentity, corpus_sha256: &str) -> EvaluationIdentity {
    EvaluationIdentity {
        runtime: runtime.clone(),
        backend: BackendProvenance {
            repository: "badi/semantic-fixture-backend".to_owned(),
            revision: REVISION_ONE.to_owned(),
            version: "fixture-v1".to_owned(),
            distribution_archive_sha256: None,
            executed_loader_sha256: runtime.binary_sha256.clone(),
            runtime_bundle_manifest_sha256: None,
            dynamic_bundle_constraint: FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT.to_owned(),
        },
        model: CandidateModelProvenance {
            artifact_filename: "fixture-model.gguf".to_owned(),
            artifact_sha256: runtime.model_sha256.clone(),
            artifact_bytes: runtime.model_size,
            quantization: "fixture".to_owned(),
            quantizer_repository: "badi/semantic-fixture-quantizer".to_owned(),
            quantizer_revision: REVISION_TWO.to_owned(),
            upstream_base_repository: "badi/semantic-fixture-base".to_owned(),
            upstream_base_revision: Some(REVISION_THREE.to_owned()),
            upstream_base_revision_status: RevisionStatus::Exact,
            tokenizer_repository: "badi/semantic-fixture-tokenizer".to_owned(),
            tokenizer_revision: Some(REVISION_FOUR.to_owned()),
            tokenizer_revision_status: RevisionStatus::Exact,
            license: "fixture-only".to_owned(),
        },
        prompt_contract_id: PROMPT_CONTRACT_ID,
        prompt_contract_sha256: prompt_contract_sha256(),
        corpus_sha256: corpus_sha256.to_owned(),
    }
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn encode_lower_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}
