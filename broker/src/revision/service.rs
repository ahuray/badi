//! What runs a revision: the interface the engine calls, and the
//! deterministic fixture behind `--provider phrase`.

use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use super::{RevisionLanguage, RevisionMode};
use crate::protocol::ProviderKind;

/// One revision model call.
#[derive(Clone, Copy, Debug)]
pub struct RevisionCall<'a> {
    pub mode: RevisionMode,
    pub language: &'static RevisionLanguage,
    /// The field's text before the selection, whose sentence start the model
    /// reads with a mid-sentence selection.
    pub before: &'a str,
    /// The selection as the field holds it.
    pub selected: &'a str,
    /// Synonyms already shown for this selection, not to use again
    /// (`RevisionMode::Synonyms` only).
    pub excluded: &'a [String],
    /// The bound on the whole call, a cold start included.
    pub timeout: Duration,
}

/// The model's answer, before [`super::accept`] decides whether it is shown.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RevisionReply {
    pub output: String,
    /// The model stopped at its token limit, so the answer may be cut off.
    pub truncated: bool,
    /// This call started the revision runtime.
    pub cold: bool,
    pub generated_tokens: Option<u32>,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum RevisionError {
    /// No revision model is installed, or there is no memory to start it.
    #[error("revision_unavailable")]
    Unavailable,
    #[error("revision_timeout")]
    Timeout,
    #[error("revision_failed")]
    Failed,
    #[error("revision_cancelled")]
    Cancelled,
}

#[async_trait]
pub trait Reviser: Send + Sync {
    fn provider(&self) -> ProviderKind;

    async fn revise(
        &self,
        call: RevisionCall<'_>,
        cancellation: CancellationToken,
    ) -> Result<RevisionReply, RevisionError>;
}

/// The deterministic fixture: grammar fixes a few known mistakes; refine
/// also starts with a capital and ends with a period.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixtureReviser;

/// Fixture synonyms, in the order repeated requests offer them.
const FIXTURE_SYNONYMS: [(&str, &[&str]); 3] = [
    ("beautiful", &["lovely", "pretty"]),
    ("big", &["large", "huge"]),
    ("fast", &["quick", "rapid"]),
];

const FIXTURE_FIXES: [(&str, &str); 7] = [
    ("has went", "went"),
    ("buyed", "bought"),
    ("dont", "don't"),
    ("their going", "they're going"),
    ("teh", "the"),
    ("three apple", "three apples"),
    ("habe nach Hause gegangen", "bin nach Hause gegangen"),
];

#[async_trait]
impl Reviser for FixtureReviser {
    fn provider(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn revise(
        &self,
        call: RevisionCall<'_>,
        cancellation: CancellationToken,
    ) -> Result<RevisionReply, RevisionError> {
        if cancellation.is_cancelled() {
            return Err(RevisionError::Cancelled);
        }
        if call.mode == RevisionMode::Synonyms {
            let lead = super::sentence_lead(call.before);
            let core = call.selected.trim();
            let synonym = FIXTURE_SYNONYMS
                .iter()
                .find(|(word, _)| word.eq_ignore_ascii_case(core))
                .and_then(|(_, synonyms)| {
                    synonyms.iter().find(|synonym| {
                        !call
                            .excluded
                            .iter()
                            .any(|shown| shown.eq_ignore_ascii_case(synonym))
                    })
                });
            let output = match synonym {
                Some(synonym) => {
                    let mut synonym = (*synonym).to_owned();
                    if core.chars().next().is_some_and(char::is_uppercase) {
                        if let Some(first) = synonym.chars().next() {
                            let upper: String = first.to_uppercase().collect();
                            synonym.replace_range(..first.len_utf8(), &upper);
                        }
                    }
                    format!("{lead}{synonym}").trim().to_owned()
                }
                // No other fixture synonym: the gate reads this as unchanged.
                None => format!("{lead}{core}").trim().to_owned(),
            };
            return Ok(RevisionReply {
                output,
                ..RevisionReply::default()
            });
        }
        let mut output = format!("{}{}", super::sentence_lead(call.before), call.selected)
            .trim()
            .to_owned();
        for (wrong, right) in FIXTURE_FIXES {
            output = output.replace(wrong, right);
        }
        if call.mode == RevisionMode::Refine {
            if let Some(first) = output.chars().next().filter(|first| first.is_lowercase()) {
                let upper: String = first.to_uppercase().collect();
                output.replace_range(..first.len_utf8(), &upper);
            }
            if !output.ends_with(['.', '!', '?']) {
                output.push('.');
            }
        }
        Ok(RevisionReply {
            output,
            ..RevisionReply::default()
        })
    }
}
