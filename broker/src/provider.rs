use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::protocol::ProviderKind;

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

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError>;
}

#[derive(Clone, Debug)]
pub struct PhraseRule {
    suffix: String,
    completion: String,
}

impl PhraseRule {
    #[must_use]
    pub fn new(suffix: impl Into<String>, completion: impl Into<String>) -> Self {
        Self {
            suffix: suffix.into(),
            completion: completion.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct DeterministicPhraseProvider {
    rules: Vec<PhraseRule>,
    fallback: String,
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
            fallback: " and continue from there".to_owned(),
        }
    }
}

impl DeterministicPhraseProvider {
    #[must_use]
    pub fn new(rules: Vec<PhraseRule>, fallback: impl Into<String>) -> Self {
        Self {
            rules,
            fallback: fallback.into(),
        }
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
        let before = request.before.to_lowercase();
        let completion = self
            .rules
            .iter()
            .find(|rule| before.ends_with(&rule.suffix))
            .map_or_else(|| self.fallback.clone(), |rule| rule.completion.clone());
        Ok(Some(completion))
    }
}

#[cfg(test)]
mod tests {
    use tokio_util::sync::CancellationToken;

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
}
