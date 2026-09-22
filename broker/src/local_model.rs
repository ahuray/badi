//! Historical evaluator qualification; interactive inference lives in [`crate::writing`].

use thiserror::Error;

use crate::semantic::candidate::{
    CandidateError, MODEL_SHA256, PinnedCandidatePaths, RUNTIME_ARCHIVE_SHA256, RUNTIME_SHA256,
    VerifiedPinnedCandidate,
};
use crate::semantic::client::{PROMPT_CONTRACT_ID, prompt_contract_sha256};
use crate::semantic::runtime::{LLAMA_CPP_LAUNCH_CONTRACT_ID, OwnedRuntime, RuntimeError};

pub const PRODUCTION_ACTIVATION_CONTRACT_ID: &str = "badi.semantic.production-activation.v1";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProductionActivationStatus {
    AwaitingQualifiedReceipt,
}

#[must_use]
pub const fn production_activation_status() -> ProductionActivationStatus {
    ProductionActivationStatus::AwaitingQualifiedReceipt
}

/// No public constructor: evaluation receipts cannot authorize production activation.
pub struct QualifiedSemanticActivation {
    model_sha256: String,
    runtime_sha256: String,
    runtime_archive_sha256: String,
    prompt_contract_id: String,
    prompt_contract_sha256: String,
    launch_contract_id: String,
}

impl QualifiedSemanticActivation {
    fn validate(&self) -> Result<(), ProductionActivationError> {
        for (matches, field) in [
            (self.model_sha256 == MODEL_SHA256, "model_sha256"),
            (self.runtime_sha256 == RUNTIME_SHA256, "runtime_sha256"),
            (
                self.runtime_archive_sha256 == RUNTIME_ARCHIVE_SHA256,
                "runtime_archive_sha256",
            ),
            (
                self.prompt_contract_id == PROMPT_CONTRACT_ID,
                "prompt_contract_id",
            ),
            (
                self.prompt_contract_sha256 == prompt_contract_sha256(),
                "prompt_contract_sha256",
            ),
            (
                self.launch_contract_id == LLAMA_CPP_LAUNCH_CONTRACT_ID,
                "launch_contract_id",
            ),
        ] {
            if !matches {
                return Err(ProductionActivationError::IdentityMismatch(field));
            }
        }
        Ok(())
    }
}

/// Starts the pinned runtime only with a matching qualification credential.
pub async fn activate_pinned_semantic_provider(
    qualification: QualifiedSemanticActivation,
    paths: PinnedCandidatePaths,
) -> Result<OwnedRuntime, ProductionActivationError> {
    qualification.validate()?;
    let candidate = tokio::task::spawn_blocking(move || VerifiedPinnedCandidate::verify(&paths))
        .await
        .map_err(|_| ProductionActivationError::VerificationTask)??;
    let runtime = candidate.launch()?.spawn().await?;
    if let Err(error) = candidate.reverify() {
        drop(runtime);
        return Err(CandidateError::Provenance(error).into());
    }
    Ok(runtime)
}

#[derive(Debug, Error)]
pub enum ProductionActivationError {
    #[error("semantic production activation identity mismatch: {0}")]
    IdentityMismatch(&'static str),
    #[error("semantic production candidate verification failed")]
    Candidate(#[from] CandidateError),
    #[error("semantic owned runtime failed")]
    Runtime(#[from] RuntimeError),
    #[error("semantic candidate verification task failed")]
    VerificationTask,
}

#[cfg(test)]
mod tests {
    use super::{ProductionActivationStatus, production_activation_status};

    #[test]
    fn production_activation_is_explicitly_unqualified() {
        assert_eq!(
            production_activation_status(),
            ProductionActivationStatus::AwaitingQualifiedReceipt
        );
    }
}
