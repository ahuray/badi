use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::sync::CancellationToken;

use crate::metrics::NoSuggestionReason;
use crate::protocol::{MAX_BEFORE_CHARS, ProviderKind};

/// Provider writing budget for a suggestion requested by typing, in ms.
pub const AUTOMATIC_WRITING_BUDGET_MS: u64 = 550;
/// Provider writing budget for an explicit request (Tab or a control
/// request): the user asked and waits for one suggestion.
pub const EXPLICIT_WRITING_BUDGET_MS: u64 = 1_200;
/// A spelling-only field's check: no keystroke waits for it, and choosing
/// between dictionary candidates needs an uncached model call.
pub const SPELLING_WRITING_BUDGET_MS: u64 = 2_500;

/// What asked for a suggestion. Only the time budget differs; policy,
/// binding and validation are the same for both.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum RequestTrigger {
    #[default]
    Automatic,
    Explicit,
    /// A spelling-only field (the Writer extension), automatic or explicit.
    Spelling,
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
            Self::Spelling => SPELLING_WRITING_BUDGET_MS,
        })
    }
}

/// Content-free counts from one provider call, for the debug trace. Token
/// counts come from the runtime's own timings when it finished the call;
/// `generated_tokens` falls back to the streamed token events seen.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ProviderStats {
    pub ttft: Option<Duration>,
    pub prompt_tokens: Option<u32>,
    pub cache_tokens: Option<u32>,
    pub generated_tokens: u32,
    /// The lowest token log-probability of a gated suggestion in hundredths,
    /// when known (an integer, so stats stay comparable).
    pub min_logprob_hundredths: Option<i32>,
}

/// One provider reply: the outcome and, when the provider measured the
/// call, its counts.
#[derive(Clone, Debug)]
pub struct ProviderReply {
    pub outcome: ProviderOutcome,
    pub stats: Option<ProviderStats>,
}

impl From<ProviderOutcome> for ProviderReply {
    fn from(outcome: ProviderOutcome) -> Self {
        Self {
            outcome,
            stats: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderRequest {
    pub before: String,
    pub after: String,
    pub language: Option<String>,
    /// Words for finishing the word at the caret: from the field's window,
    /// the writer's earlier writing in the app, and the draft itself. `None`
    /// unless the person turned context on; then no draft word is used either.
    pub vocabulary: Option<crate::vocabulary::Vocabulary>,
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

    /// Applies the person's language choices from settings: which languages
    /// get suggestions and which heal a trailing space. Providers without a
    /// model ignore them.
    fn set_language_preferences(&self, _preferences: &crate::writing::LanguagePreferences) {}

    /// Applies the person's suggestion length from settings. Providers that
    /// do not generate text, such as the phrase fixture, ignore it.
    fn set_suggestion_length(&self, _length: crate::settings::SuggestionLength) {}

    /// Completes once [`Self::is_alive`] turns false. Providers that own a
    /// process override both; the default never completes.
    async fn exited(&self) {
        std::future::pending::<()>().await;
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

    /// Like [`Self::propose_outcome`], and may publish a growing prefix of the
    /// same continuation before the final text. The final outcome is still
    /// authoritative. Senders that are dropped are ignored.
    async fn propose_outcome_with_partials(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        trigger: RequestTrigger,
        partials: Option<UnboundedSender<String>>,
    ) -> Result<ProviderReply, ProviderError> {
        let _ = partials;
        self.propose_outcome(request, cancellation, allow_replacement, trigger)
            .await
            .map(ProviderReply::from)
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
        allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderOutcome, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if allow_replacement {
            if let Some(proposal) = dictionary_correction(&request) {
                return Ok(ProviderOutcome::Proposal(proposal));
            }
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

/// The fixture shares the writing model's inference-free dictionary path: an
/// English word before the caret, at the end of the field, with exactly one
/// single-edit dictionary candidate. Editor integrations can then test exact
/// replacement deterministically; ambiguous words stay a model decision.
#[cfg(feature = "local-model")]
fn dictionary_correction(request: &ProviderRequest) -> Option<WritingProposal> {
    let english = request.language.as_deref().is_some_and(|language| {
        language
            .split('-')
            .next()
            .is_some_and(|primary| primary.eq_ignore_ascii_case("en"))
    });
    if !english || !request.after.is_empty() {
        return None;
    }
    let target = crate::writing::correction_target(&request.before)?;
    let corrected = crate::writing::unambiguous_correction(target.word)?;
    Some(WritingProposal {
        text: format!("{corrected}{}", target.delimiter),
        replace_before: Some(target.suffix.to_owned()),
    })
}

#[cfg(not(feature = "local-model"))]
fn dictionary_correction(_request: &ProviderRequest) -> Option<WritingProposal> {
    None
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
            vocabulary: None,
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
                    vocabulary: None,
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
                vocabulary: None,
            },
            ProviderRequest {
                before: "I will not say thank you".to_owned(),
                after: String::new(),
                language: Some("en-US".to_owned()),
                vocabulary: None,
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: " already written".to_owned(),
                language: Some("en".to_owned()),
                vocabulary: None,
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: String::new(),
                language: Some("de".to_owned()),
                vocabulary: None,
            },
            ProviderRequest {
                before: "thank you".to_owned(),
                after: String::new(),
                language: None,
                vocabulary: None,
            },
            ProviderRequest {
                before: "thank you ".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
                vocabulary: None,
            },
            ProviderRequest {
                before: format!("{}thank you", " ".repeat(MAX_BEFORE_CHARS - 9)),
                after: String::new(),
                language: Some("en".to_owned()),
                vocabulary: None,
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
                    vocabulary: None,
                },
                CancellationToken::new(),
            )
            .await
            .expect("provider result");

        assert_eq!(result.as_deref(), Some(" to hearing from you"));
    }

    #[test]
    fn explicit_and_spelling_requests_have_longer_writing_budgets() {
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
        assert_eq!(
            RequestTrigger::Spelling.writing_budget(),
            Duration::from_millis(2_500)
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
                        vocabulary: None,
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
                    vocabulary: None,
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
                        vocabulary: None,
                    },
                    cancelled,
                    false,
                    RequestTrigger::Automatic,
                )
                .await,
            Err(super::ProviderError::Cancelled)
        ));
    }

    #[cfg(feature = "local-model")]
    #[tokio::test]
    async fn phrase_fixture_offers_only_negotiated_unambiguous_dictionary_corrections() {
        use super::{ProviderOutcome, RequestTrigger, WritingProposal};
        let provider = DeterministicPhraseProvider::default();
        let propose = |before: &str, after: &str, language: &str, allow_replacement: bool| {
            provider.propose_outcome(
                ProviderRequest {
                    before: before.to_owned(),
                    after: after.to_owned(),
                    language: Some(language.to_owned()),
                    vocabulary: None,
                },
                CancellationToken::new(),
                allow_replacement,
                RequestTrigger::Explicit,
            )
        };
        assert_eq!(
            propose("We need an adress ", "", "en-US", true)
                .await
                .unwrap(),
            ProviderOutcome::Proposal(WritingProposal {
                text: "address ".to_owned(),
                replace_before: Some("adress ".to_owned()),
            })
        );
        assert_eq!(
            propose("We need an adress", "", "en", true).await.unwrap(),
            ProviderOutcome::Proposal(WritingProposal {
                text: "address".to_owned(),
                replace_before: Some("adress".to_owned()),
            })
        );
        // No negotiated replacement, text after the caret, another language
        // or an ambiguous word (teh: the/ten/tea...) never yields an edit.
        for (before, after, language, allow) in [
            ("We need an adress ", "", "en", false),
            ("We need an adress", " here", "en", true),
            ("We need an adress", "", "de", true),
            ("This is teh", "", "en", true),
        ] {
            let outcome = propose(before, after, language, allow).await.unwrap();
            assert!(
                outcome
                    .into_proposal()
                    .is_none_or(|proposal| proposal.replace_before.is_none()),
                "{before:?} {after:?} {language} {allow}"
            );
        }
    }
}
