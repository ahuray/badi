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

/// Whether `text` has the continuation shape `kind`'s suggestions must have
/// between `before` and `after`: the local model's own rules, or the phrase
/// fixture's segment rules.
#[must_use]
pub fn continuation_shape_valid(kind: ProviderKind, before: &str, after: &str, text: &str) -> bool {
    match kind {
        ProviderKind::LocalModel => {
            crate::writing::validate_suggestion_shape(before, after, text).is_ok()
        }
        ProviderKind::PhraseV1 => {
            crate::segment::validate_suggestion_shape(before, after, text).is_ok()
        }
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

    /// What the broker calls: like `propose`, and names the content-free
    /// class of a missing proposal, with the call's counts when the provider
    /// measured them. Providers that cannot distinguish request and model
    /// abstention report the model class. A provider with a time budget
    /// sizes it by `trigger`. The reply is one whole suggestion.
    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderReply, ProviderError> {
        self.propose(request, cancellation, allow_replacement)
            .await
            .map(|proposal| {
                proposal
                    .map_or(
                        ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained),
                        ProviderOutcome::Proposal,
                    )
                    .into()
            })
    }

    /// For an explicit request for alternatives: the best continuation
    /// that `ask` allows. A provider without one abstains.
    async fn alternative(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        _trigger: RequestTrigger,
        _ask: AlternativeAsk,
    ) -> Result<ProviderReply, ProviderError> {
        Ok(ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained).into())
    }
}

/// What an alternative must differ from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AlternativeAsk {
    /// First words already shown, lowercased; the alternative starts with
    /// none of them.
    pub excluded: Vec<String>,
    /// The shown suggestion started a new word after the caret (`the`
    /// showed ` latest`), so the alternative starts one too instead of
    /// finishing the word at the caret another way.
    pub new_word: bool,
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
    /// Further continuations, offered in order as alternatives.
    alternatives: Vec<String>,
}

impl PhraseRule {
    #[must_use]
    pub fn new(suffix: impl Into<String>, completion: impl Into<String>) -> Self {
        Self {
            trigger: suffix.into(),
            completion: completion.into(),
            alternatives: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_alternatives(mut self, alternatives: &[&str]) -> Self {
        self.alternatives = alternatives.iter().map(|text| (*text).to_owned()).collect();
        self
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
                PhraseRule::new("thank you", " for your time")
                    .with_alternatives(&[" so much", " again"]),
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
    ) -> Result<ProviderReply, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if allow_replacement {
            if let Some(proposal) = dictionary_correction(&request) {
                return Ok(ProviderOutcome::Proposal(proposal).into());
            }
        }
        Ok(match self.phrase(&request) {
            Ok(text) => ProviderOutcome::Proposal(WritingProposal {
                text,
                replace_before: None,
            }),
            Err(reason) => ProviderOutcome::NoSuggestion(reason),
        }
        .into())
    }

    async fn alternative(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        _trigger: RequestTrigger,
        ask: AlternativeAsk,
    ) -> Result<ProviderReply, ProviderError> {
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(match self.rule(&request) {
            Ok(rule) => rule
                .alternatives
                .iter()
                .find(|text| {
                    text.split_whitespace()
                        .next()
                        .is_some_and(|word| !ask.excluded.contains(&word.to_lowercase()))
                })
                .map_or(
                    ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained),
                    |text| {
                        ProviderOutcome::Proposal(WritingProposal {
                            text: text.clone(),
                            replace_before: None,
                        })
                    },
                ),
            Err(reason) => ProviderOutcome::NoSuggestion(reason),
        }
        .into())
    }
}

/// The fixture shares the writing model's inference-free dictionary path: an
/// English word before the caret, at the end of the field, with exactly one
/// single-edit dictionary candidate. Editor integrations can then test exact
/// replacement deterministically; ambiguous words stay a model decision.
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

impl DeterministicPhraseProvider {
    fn phrase(&self, request: &ProviderRequest) -> Result<String, NoSuggestionReason> {
        self.rule(request).map(|rule| rule.completion.clone())
    }

    fn rule(&self, request: &ProviderRequest) -> Result<&PhraseRule, NoSuggestionReason> {
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
            .ok_or(NoSuggestionReason::ModelAbstained)
    }
}

#[cfg(test)]
mod tests;
