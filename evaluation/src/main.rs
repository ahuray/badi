use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use badi_broker::semantic;
use badi_broker::semantic::candidate::{
    MODEL_FILENAME, MODEL_LICENSE, MODEL_QUANTIZATION, MODEL_QUANTIZER_REPOSITORY,
    MODEL_QUANTIZER_REVISION, MODEL_UPSTREAM_REPOSITORY, PinnedCandidatePaths,
    RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT, RUNTIME_REPOSITORY, RUNTIME_REVISION, RUNTIME_VERSION,
    VerifiedPinnedCandidate,
};

mod fixture_backend;
mod raw_run;
mod runner;

use raw_run::{
    BackendProvenance, CandidateModelProvenance, CheckStatus, EvaluationIdentity,
    EvaluationReceipt, EvidenceClass, FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT, RawRun,
    RevisionStatus,
};
use semantic::client::{PROMPT_CONTRACT_ID, prompt_contract_sha256};
use semantic::provenance::{FileExpectation, verify_file};
use semantic::runtime::{FixtureBehavior, LlamaCppLaunch};

const USAGE: &str = "Usage:\n\
  badi-evaluator fixture-self-test\n\
  badi-evaluator pinned-development <model.gguf> <llama-server> <release-archive.tar.gz>\n\
Internal: badi-evaluator __fixture-backend\n\
\n\
This feature-gated tool produces evaluation-only development evidence. It does\n\
not download, qualify, activate, or expose a model to the normal broker.\n";

#[tokio::main]
async fn main() {
    let mut arguments = std::env::args().skip(1);
    let result = match arguments.next().as_deref() {
        Some("__fixture-backend") if arguments.next().is_none() => {
            fixture_backend::run_from_environment()
                .await
                .map_err(|error| Box::new(error) as Box<dyn Error>)
        }
        Some("fixture-self-test") if arguments.next().is_none() => fixture_self_test().await,
        Some("pinned-development") => {
            let model = arguments.next().map(PathBuf::from);
            let runtime = arguments.next().map(PathBuf::from);
            let archive = arguments.next().map(PathBuf::from);
            if let (Some(model), Some(runtime), Some(archive), None) =
                (model, runtime, archive, arguments.next())
            {
                pinned_development(&model, &runtime, &archive).await
            } else {
                Err("invalid pinned-development arguments".into())
            }
        }
        Some("--help" | "-h") if arguments.next().is_none() => {
            print!("{USAGE}");
            Ok(())
        }
        None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(_) => Err("invalid evaluator arguments".into()),
    };
    if result.is_err() {
        eprintln!("error_code=evaluator_failed");
        std::process::exit(1);
    }
}

async fn fixture_self_test() -> Result<(), Box<dyn Error>> {
    let workspace = TemporaryFixture::new()?;
    let model_path = workspace.path().join("fixture-model.gguf");
    let mut model = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&model_path)?;
    model.write_all(b"Badi semantic evaluator fixture artifact v1\n")?;
    model.sync_all()?;
    drop(model);

    let executable = fs::canonicalize(std::env::current_exe()?)?;
    let model_path = fs::canonicalize(model_path)?;
    let binary = verify_file(&expectation_for_fixture(&executable)?)?;
    let model = verify_file(&expectation_for_fixture(&model_path)?)?;
    let launch = LlamaCppLaunch::for_fixture(binary, model, FixtureBehavior::Ready)?;
    let runtime = launch.spawn().await?;
    let runtime_identity = runtime.identity().clone();
    let cases = runner::fixture_cases();
    let raw_run =
        runner::run_development_cases(runtime.client(), &cases, EvidenceClass::DevelopmentFixture)
            .await?;
    let lifecycle = runtime.shutdown()?;
    let identity = EvaluationIdentity {
        backend: BackendProvenance {
            repository: "badi/semantic-fixture-backend".to_owned(),
            revision: "0000000000000000000000000000000000000001".to_owned(),
            version: "fixture-v1".to_owned(),
            distribution_archive_sha256: None,
            executed_loader_sha256: runtime_identity.binary_sha256.clone(),
            runtime_bundle_manifest_sha256: None,
            dynamic_bundle_constraint: FIXTURE_RUNTIME_PROVENANCE_CONSTRAINT.to_owned(),
        },
        model: CandidateModelProvenance {
            artifact_filename: "fixture-model.gguf".to_owned(),
            artifact_sha256: runtime_identity.model_sha256.clone(),
            artifact_bytes: runtime_identity.model_size,
            quantization: "fixture".to_owned(),
            quantizer_repository: "badi/semantic-fixture-quantizer".to_owned(),
            quantizer_revision: "0000000000000000000000000000000000000002".to_owned(),
            upstream_base_repository: "badi/semantic-fixture-base".to_owned(),
            upstream_base_revision: Some("0000000000000000000000000000000000000003".to_owned()),
            upstream_base_revision_status: RevisionStatus::Exact,
            tokenizer_repository: "badi/semantic-fixture-tokenizer".to_owned(),
            tokenizer_revision: Some("0000000000000000000000000000000000000004".to_owned()),
            tokenizer_revision_status: RevisionStatus::Exact,
            license: "fixture-only".to_owned(),
        },
        runtime: runtime_identity,
        prompt_contract_id: PROMPT_CONTRACT_ID,
        prompt_contract_sha256: prompt_contract_sha256(),
        corpus_sha256: raw_run.corpus_sha256().to_owned(),
    };
    let receipt = EvaluationReceipt::from_raw_run(&raw_run, identity, &lifecycle)?;
    if receipt
        .checks()
        .iter()
        .any(|check| check.status != CheckStatus::Passed)
    {
        return Err("fixture semantic check failed".into());
    }
    print_bundle(&raw_run, &receipt)?;
    Ok(())
}

async fn pinned_development(
    model_path: &Path,
    runtime_path: &Path,
    archive_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let paths = PinnedCandidatePaths::new(model_path, runtime_path, archive_path);
    let candidate = VerifiedPinnedCandidate::verify(&paths)?;
    let runtime = candidate.launch()?.spawn().await?;
    let runtime_identity = runtime.identity().clone();
    let cases = runner::pinned_candidate_cases();
    let raw_run_result = runner::run_development_cases(
        runtime.client(),
        &cases,
        EvidenceClass::DevelopmentCandidate,
    )
    .await;
    let lifecycle = runtime.shutdown()?;
    let raw_run = raw_run_result?;

    candidate.reverify()?;
    let identity = EvaluationIdentity {
        backend: BackendProvenance {
            repository: RUNTIME_REPOSITORY.to_owned(),
            revision: RUNTIME_REVISION.to_owned(),
            version: RUNTIME_VERSION.to_owned(),
            distribution_archive_sha256: Some(candidate.runtime_archive().sha256().to_owned()),
            executed_loader_sha256: candidate.runtime().sha256().to_owned(),
            runtime_bundle_manifest_sha256: Some(candidate.runtime_bundle().sha256().to_owned()),
            dynamic_bundle_constraint: RUNTIME_DYNAMIC_BUNDLE_CONSTRAINT.to_owned(),
        },
        model: CandidateModelProvenance {
            artifact_filename: MODEL_FILENAME.to_owned(),
            artifact_sha256: candidate.model().sha256().to_owned(),
            artifact_bytes: candidate.model().identity().size,
            quantization: MODEL_QUANTIZATION.to_owned(),
            quantizer_repository: MODEL_QUANTIZER_REPOSITORY.to_owned(),
            quantizer_revision: MODEL_QUANTIZER_REVISION.to_owned(),
            upstream_base_repository: MODEL_UPSTREAM_REPOSITORY.to_owned(),
            upstream_base_revision: None,
            upstream_base_revision_status: RevisionStatus::Unreported,
            tokenizer_repository: MODEL_QUANTIZER_REPOSITORY.to_owned(),
            tokenizer_revision: None,
            tokenizer_revision_status: RevisionStatus::EmbeddedInArtifact,
            license: MODEL_LICENSE.to_owned(),
        },
        runtime: runtime_identity,
        prompt_contract_id: PROMPT_CONTRACT_ID,
        prompt_contract_sha256: prompt_contract_sha256(),
        corpus_sha256: raw_run.corpus_sha256().to_owned(),
    };
    let receipt = EvaluationReceipt::from_raw_run(&raw_run, identity, &lifecycle)?;
    print_bundle(&raw_run, &receipt)
}

#[derive(Serialize)]
struct EvaluationBundle<'a> {
    schema: &'static str,
    raw_run: &'a RawRun,
    receipt: &'a EvaluationReceipt,
}

fn print_bundle(raw_run: &RawRun, receipt: &EvaluationReceipt) -> Result<(), Box<dyn Error>> {
    let bundle = EvaluationBundle {
        schema: "badi.semantic-evaluation-bundle.v1",
        raw_run,
        receipt,
    };
    println!("{}", serde_json::to_string_pretty(&bundle)?);
    Ok(())
}

fn expectation_for_fixture(path: &Path) -> Result<FileExpectation, Box<dyn Error>> {
    let metadata = fs::metadata(path)?;
    let digest = sha256_file(path)?;
    Ok(FileExpectation::new(path, digest, metadata.len())?)
}

fn sha256_file(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1_024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(encode_lower_hex(hasher.finalize()))
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

struct TemporaryFixture {
    path: PathBuf,
}

impl TemporaryFixture {
    fn new() -> Result<Self, std::io::Error> {
        let root = fs::canonicalize(std::env::temp_dir())?;
        let path = root.join(format!("badi-semantic-fixture-{}", Uuid::new_v4().simple()));
        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700);
        builder.create(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TemporaryFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}
