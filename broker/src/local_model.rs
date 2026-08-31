//! Disabled-by-default production boundary for the one pinned semantic lane.
//!
//! Transport, prompt parsing, output policy, artifact verification, and child
//! ownership live in [`crate::semantic`]. This module intentionally contains no
//! second llama.cpp client or runtime implementation.
//!
//! The development evaluator emits evaluation-only receipts with
//! `production_ready: false`. Those receipts cannot construct
//! [`QualifiedSemanticActivation`], so the normal broker remains on the
//! deterministic provider until a future immutable scored run earns a separate
//! qualification constructor.

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

/// Opaque authorization for a scored, immutable production qualification.
///
/// There is deliberately no public constructor. In particular, an
/// evaluation-only receipt is not an activation credential. A future
/// qualification module must derive this value from its immutable raw run and
/// match every identity below before broker wiring can call
/// [`activate_pinned_semantic_provider`].
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

/// Verifies the pinned bytes, starts the one owned llama.cpp child, and returns
/// that owned runtime as the broker's `CompletionProvider`.
///
/// This function is unreachable from normal configuration today because
/// [`QualifiedSemanticActivation`] has no constructor. It also performs no
/// downloads and accepts no alternate model, backend, endpoint, or prompt.
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
