use std::time::Instant;

use badi_broker::provider::ProviderRequest;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::raw_run::{CaseObservation, CheckClass, EvaluationError, EvidenceClass, RawRun};
use crate::semantic::client::SemanticClient;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DevelopmentCase {
    id: &'static str,
    before: &'static str,
    after: &'static str,
    language: Option<&'static str>,
    check_class: CheckClass,
}

impl DevelopmentCase {
    #[must_use]
    pub const fn new(
        id: &'static str,
        before: &'static str,
        after: &'static str,
        language: Option<&'static str>,
        check_class: CheckClass,
    ) -> Self {
        Self {
            id,
            before,
            after,
            language,
            check_class,
        }
    }
}

pub async fn run_development_cases(
    client: &SemanticClient,
    cases: &[DevelopmentCase],
    evidence_class: EvidenceClass,
) -> Result<RawRun, EvaluationError> {
    let mut observations = Vec::with_capacity(cases.len());
    for case in cases {
        let started = Instant::now();
        let result = client
            .complete_observed(
                ProviderRequest {
                    before: case.before.to_owned(),
                    after: case.after.to_owned(),
                    language: case.language.map(str::to_owned),
                },
                CancellationToken::new(),
            )
            .await;
        let observation = match result {
            Ok(completion) => {
                CaseObservation::from_completion(case.id, case.check_class, &completion)
            }
            Err(error) => {
                CaseObservation::from_error(case.id, case.check_class, &error, started.elapsed())
            }
        };
        observations.push(observation);
    }
    RawRun::from_observations(evidence_class, corpus_sha256(cases), observations)
}

#[must_use]
pub fn fixture_cases() -> Vec<DevelopmentCase> {
    vec![
        DevelopmentCase::new(
            "scope-missing-language",
            "fixture:valid",
            "",
            None,
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-fa",
            "fixture:valid",
            "",
            Some("fa"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-ar",
            "fixture:valid",
            "",
            Some("ar"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-zh",
            "fixture:valid",
            "",
            Some("zh"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "stream-en-us",
            "fixture:valid",
            "",
            Some("en-US"),
            CheckClass::StreamingSuggested,
        ),
        DevelopmentCase::new(
            "output-arabic",
            "fixture:arabic",
            "",
            Some("en"),
            CheckClass::InvalidScriptRejected,
        ),
        DevelopmentCase::new(
            "output-cjk",
            "fixture:cjk",
            "",
            Some("en"),
            CheckClass::InvalidScriptRejected,
        ),
        DevelopmentCase::new(
            "output-emoji",
            "fixture:emoji",
            "",
            Some("en"),
            CheckClass::InvalidScriptRejected,
        ),
        DevelopmentCase::new(
            "model-abstention",
            "fixture:abstain",
            "",
            Some("en"),
            CheckClass::General,
        ),
        DevelopmentCase::new(
            "model-truncation",
            "fixture:truncated",
            "",
            Some("en"),
            CheckClass::General,
        ),
    ]
}

#[must_use]
pub fn pinned_candidate_cases() -> Vec<DevelopmentCase> {
    vec![
        DevelopmentCase::new(
            "scope-missing-language",
            "This sentence is never serialized",
            "",
            None,
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-fa",
            "This sentence is never serialized",
            "",
            Some("fa"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-ar",
            "This sentence is never serialized",
            "",
            Some("ar"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "scope-zh",
            "This sentence is never serialized",
            "",
            Some("zh"),
            CheckClass::ScopeGuardRejected,
        ),
        DevelopmentCase::new(
            "native-prefix-feedback",
            "Thank you for reviewing the proposal. I look forward",
            "",
            Some("en"),
            CheckClass::StreamingSuggested,
        ),
        DevelopmentCase::new(
            "native-prefix-final-version",
            "This will",
            "",
            Some("en-US"),
            CheckClass::StreamingSuggested,
        ),
    ]
}

#[must_use]
pub fn corpus_sha256(cases: &[DevelopmentCase]) -> String {
    let canonical = serde_json::to_vec(cases).expect("development cases are serializable");
    encode_lower_hex(Sha256::digest(canonical))
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
