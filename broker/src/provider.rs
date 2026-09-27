use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::metrics::NoSuggestionReason;
use crate::protocol::{MAX_BEFORE_CHARS, ProviderKind};

/// Provider writing budget for a suggestion requested by typing, in ms.
pub const AUTOMATIC_WRITING_BUDGET_MS: u64 = 550;
/// Provider writing budget for an explicit request (Tab or a control
/// request): the user asked and waits for one suggestion.
pub const EXPLICIT_WRITING_BUDGET_MS: u64 = 1_200;

/// What asked for a suggestion. Only the time budget differs; policy,
/// binding and validation are the same for both.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestTrigger {
    #[default]
    Automatic,
    Explicit,
}

impl RequestTrigger {
    #[must_use]
    pub const fn from_explicit(explicit: bool) -> Self {
        if explicit {
            Self::Explicit
        } else {
            Self::Automatic
        }
    }

    #[must_use]
    pub const fn writing_budget(self) -> Duration {
        Duration::from_millis(match self {
            Self::Automatic => AUTOMATIC_WRITING_BUDGET_MS,
            Self::Explicit => EXPLICIT_WRITING_BUDGET_MS,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRequest {
    pub before: String,
    pub after: String,
    pub language: Option<String>,
}

impl ProviderRequest {
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.before
            .len()
            .saturating_add(self.after.len())
            .saturating_add(self.language.as_ref().map_or(0, String::len))
    }
}

#[derive(Debug, Error)]
pub enum ProviderError {
    #[error("cancelled")]
    Cancelled,
    #[error("provider_unavailable")]
    Unavailable,
}

#[async_trait]
pub trait CompletionProvider: Send + Sync + 'static {
    fn kind(&self) -> ProviderKind;

    /// Whether this provider's owned resources can still serve requests.
    /// A false result is terminal: the broker shuts down without replaying work.
    fn is_alive(&self) -> bool {
        true
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError>;

    async fn propose(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        _allow_replacement: bool,
    ) -> Result<Option<WritingProposal>, ProviderError> {
        self.complete(request, cancellation).await.map(|output| {
            output.map(|text| WritingProposal {
                text,
                replace_before: None,
            })
        })
    }

    /// Like `propose`, and names the content-free class of a missing proposal.
    /// Providers that cannot distinguish request and model abstention report
    /// the model class. A provider with a time budget sizes it by `trigger`.
    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderOutcome, ProviderError> {
        self.propose(request, cancellation, allow_replacement)
            .await
            .map(|proposal| {
                proposal.map_or(
                    ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained),
                    ProviderOutcome::Proposal,
                )
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WritingProposal {
    pub text: String,
    /// Exact suffix of the guarded context to replace; absent for insertions.
    pub replace_before: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderOutcome {
    Proposal(WritingProposal),
    NoSuggestion(NoSuggestionReason),
}

impl ProviderOutcome {
    #[must_use]
    pub fn into_proposal(self) -> Option<WritingProposal> {
        match self {
            Self::Proposal(proposal) => Some(proposal),
            Self::NoSuggestion(_) => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PhraseRule {
    trigger: String,
    completion: String,
}

impl PhraseRule {
    #[must_use]
    pub fn new(suffix: impl Into<String>, completion: impl Into<String>) -> Self {
        Self {
            trigger: suffix.into(),
            completion: completion.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeterministicPhraseProvider {
    rules: Vec<PhraseRule>,
}

impl Default for DeterministicPhraseProvider {
    fn default() -> Self {
        Self {
            rules: vec![
                PhraseRule::new("thank you", " for your time"),
                PhraseRule::new("looking forward", " to hearing from you"),
                PhraseRule::new("the next step", " is to verify the result"),
                PhraseRule::new("please", " let me know what you think"),
            ],
        }
    }
}

impl DeterministicPhraseProvider {
    #[must_use]
    pub fn new(rules: Vec<PhraseRule>) -> Self {
        Self { rules }
    }
}

#[async_trait]
impl CompletionProvider for DeterministicPhraseProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(self.phrase(&request).ok())
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        _allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderOutcome, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(match self.phrase(&request) {
            Ok(text) => ProviderOutcome::Proposal(WritingProposal {
                text,
                replace_before: None,
            }),
            Err(reason) => ProviderOutcome::NoSuggestion(reason),
        })
    }
}

impl DeterministicPhraseProvider {
    fn phrase(&self, request: &ProviderRequest) -> Result<String, NoSuggestionReason> {
        // Exact English triggers keep this deterministic integration provider
        // separate from model-backed writing inference.
        let Some(language) = request.language.as_deref() else {
            return Err(NoSuggestionReason::RequestAbstained);
        };
        if !request.after.is_empty()
            || (request.before.chars().count() == MAX_BEFORE_CHARS
                && !request.before.contains(['\n', '\r']))
            || !language
                .split('-')
                .next()
                .is_some_and(|primary| primary.eq_ignore_ascii_case("en"))
        {
            return Err(NoSuggestionReason::RequestAbstained);
        }
        let before = request
            .before
            .rsplit(['\n', '\r'])
            .next()
            .unwrap_or_default()
            .trim_start();
        self.rules
            .iter()
            .find(|rule| before.eq_ignore_ascii_case(&rule.trigger))
            .map(|rule| rule.completion.clone())
            .ok_or(NoSuggestionReason::ModelAbstained)
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

    use crate::protocol::MAX_BEFORE_CHARS;

    use super::{CompletionProvider, DeterministicPhraseProvider, ProviderRequest};

    #[tokio::test]
    async fn phrase_provider_is_deterministic() {
        let provider = DeterministicPhraseProvider::default();
        let request = ProviderRequest {
            before: "Thank you".to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
        };
        let first = provider
            .complete(request.clone(), CancellationToken::new())
            .await
            .expect("provider result");
        let second = provider
            .complete(request, CancellationToken::new())
            .await
            .expect("provider result");
        assert_eq!(first, second);
        assert_eq!(first.as_deref(), Some(" for your time"));
    }

    #[tokio::test]
    async fn default_phrase_provider_is_silent_without_an_explicit_rule() {
        let result = DeterministicPhraseProvider::default()
            .complete(
                ProviderRequest {
                    before: "An unmatched sentence".to_owned(),
                    after: String::new(),
                    language: Some("en".to_owned()),
                },
                CancellationToken::new(),
            )
            .await
            .expect("provider result");

        assert_eq!(result, None);
    }

    #[tokio::test]
    async fn default_phrase_provider_abstains_on_unsafe_near_matches() {
        let provider = DeterministicPhraseProvider::default();
        for request in [
            ProviderRequest {
                before: "displease".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
            },
            ProviderRequest {
                before: "I will not say thank you".to_owned(),
                after: String::new(),
                language: Some("en-US".to_owned()),
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: " already written".to_owned(),
                language: Some("en".to_owned()),
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: String::new(),
                language: Some("de".to_owned()),
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: String::new(),
                language: None,
            },
            ProviderRequest {
                before: "thank you ".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
            },
            ProviderRequest {
                before: format!("{}thank you", " ".repeat(MAX_BEFORE_CHARS - 9)),
                after: String::new(),
                language: Some("en".to_owned()),
            },
        ] {
            let result = provider
                .complete(request, CancellationToken::new())
                .await
                .expect("provider result");
            assert_eq!(result, None);
        }
    }

    #[tokio::test]
    async fn default_phrase_provider_accepts_an_english_locale() {
        let result = DeterministicPhraseProvider::default()
            .complete(
                ProviderRequest {
                    before: "Dear reviewer,\n  Looking Forward".to_owned(),
                    after: String::new(),
                    language: Some("en-GB".to_owned()),
                },
                CancellationToken::new(),
            )
            .await
            .expect("provider result");

        assert_eq!(result.as_deref(), Some(" to hearing from you"));
    }

    #[test]
    fn explicit_requests_have_the_longer_writing_budget() {
        use super::RequestTrigger;
        use std::time::Duration;

        assert_eq!(RequestTrigger::default(), RequestTrigger::Automatic);
        assert_eq!(
            RequestTrigger::from_explicit(false).writing_budget(),
            Duration::from_millis(550)
        );
        assert_eq!(
            RequestTrigger::from_explicit(true).writing_budget(),
            Duration::from_millis(1_200)
        );
    }

    #[tokio::test]
    async fn phrase_outcomes_separate_request_and_model_abstention() {
        use super::{ProviderOutcome, RequestTrigger};
        use crate::metrics::NoSuggestionReason;

        let provider = DeterministicPhraseProvider::default();
        for (before, after, language, expected) in [
            ("thank you", "", None, NoSuggestionReason::RequestAbstained),
            (
                "thank you",
                "",
                Some("de"),
                NoSuggestionReason::RequestAbstained,
            ),
            (
                "thank you",
                " tail",
                Some("en"),
                NoSuggestionReason::RequestAbstained,
            ),
            (
                "unmatched",
                "",
                Some("en"),
                NoSuggestionReason::ModelAbstained,
            ),
        ] {
            let outcome = provider
                .propose_outcome(
                    ProviderRequest {
                        before: before.to_owned(),
                        after: after.to_owned(),
                        language: language.map(str::to_owned),
                    },
                    CancellationToken::new(),
                    false,
                    RequestTrigger::Automatic,
                )
                .await
                .expect("provider outcome");
            assert_eq!(outcome, ProviderOutcome::NoSuggestion(expected));
        }
        let outcome = provider
            .propose_outcome(
                ProviderRequest {
                    before: "Thank you".to_owned(),
                    after: String::new(),
                    language: Some("en".to_owned()),
                },
                CancellationToken::new(),
                false,
                RequestTrigger::Explicit,
            )
            .await
            .expect("provider outcome");
        assert_eq!(
            outcome
                .into_proposal()
                .map(|proposal| proposal.text)
                .as_deref(),
            Some(" for your time")
        );
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            provider
                .propose_outcome(
                    ProviderRequest {
                        before: "Thank you".to_owned(),
                        after: String::new(),
                        language: Some("en".to_owned()),
                    },
                    cancelled,
                    false,
                    RequestTrigger::Automatic,
                )
                .await,
            Err(super::ProviderError::Cancelled)
        ));
    }
}
