use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::protocol::{MAX_BEFORE_CHARS, ProviderKind};

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
        // This lane is only a deterministic integration probe. It deliberately
        // abstains unless the caret is at the end, the language is compatible
        // with its English rules, and the complete trimmed context is an exact
        // trigger. Semantic suffix completion belongs to a qualified model.
        let Some(language) = request.language.as_deref() else {
            return Ok(None);
        };
        if !request.after.is_empty()
            || (request.before.chars().count() == MAX_BEFORE_CHARS
                && !request.before.contains(['\n', '\r']))
            || !language
                .split('-')
                .next()
                .is_some_and(|primary| primary.eq_ignore_ascii_case("en"))
        {
            return Ok(None);
        }
        let before = request
            .before
            .rsplit(['\n', '\r'])
            .next()
            .unwrap_or_default()
            .trim_start();
        let completion = self
            .rules
            .iter()
            .find(|rule| before.eq_ignore_ascii_case(&rule.trigger))
            .map(|rule| rule.completion.clone());
        Ok(completion)
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
}
