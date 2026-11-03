//! Alternatives on request: on a shown continuation, Ctrl+Shift+Space asks
//! the model for its next-best one, starting with a word not shown yet for
//! that context. After [`MAX_ALTERNATIVES`], or once the model has none,
//! further presses cycle through those already shown, with no model call.
//! Each is an ordinary suggestion: Tab and Ctrl+Right accept it as usual.

use std::time::Instant;

use super::rejections::next_word;
use super::{Broker, BrokerError};
use crate::policy::PolicyReason;
use crate::protocol::{
    Capability, Coordinates, SessionControlRequestPayload, SuggestRequestPayload,
};
use crate::provider::AlternativeAsk;

/// New continuations asked for one context before presses only cycle.
pub(super) const MAX_ALTERNATIVES: usize = 3;

/// The continuations shown for one context (fingerprint), in order.
#[derive(Default)]
pub(super) struct Alternatives {
    fingerprint: String,
    shown: Vec<String>,
    exhausted: bool,
}

/// What a press shows next.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Plan {
    /// Ask the model for a continuation that `ask` allows; without one,
    /// show `fallback` instead.
    Generate {
        ask: AlternativeAsk,
        fallback: String,
    },
    /// Ask the model for alternative next words to list for the shown
    /// suggestion; without any, show `fallback` instead (ADR 0014).
    List {
        ask: AlternativeAsk,
        fallback: String,
        suggestion_id: String,
    },
    /// Show this continuation again, with no model call.
    Show(String),
}

/// How an admitted generation answers an alternatives press.
#[derive(Clone)]
pub(super) enum AlternativeBinding {
    Generate {
        ask: AlternativeAsk,
        fallback: String,
    },
    List {
        ask: AlternativeAsk,
        fallback: String,
        suggestion_id: String,
    },
}

impl AlternativeBinding {
    pub(super) fn fallback(&self) -> &str {
        match self {
            Self::Generate { fallback, .. } | Self::List { fallback, .. } => fallback,
        }
    }
}

/// A numbered list of alternative next words offered for one shown
/// suggestion, live until it expires or the context moves on.
pub(super) struct LiveList {
    pub(super) fingerprint: String,
    pub(super) suggestion_id: String,
    pub(super) words: Vec<String>,
    pub(super) expires_at: Instant,
}

impl Alternatives {
    /// Notes `text`, shown for `fingerprint`; returns its place among the
    /// continuations shown for this context.
    fn note(&mut self, fingerprint: &str, text: &str) -> usize {
        if self.fingerprint != fingerprint {
            *self = Self {
                fingerprint: fingerprint.to_owned(),
                ..Self::default()
            };
        }
        self.shown
            .iter()
            .position(|shown| shown == text)
            .unwrap_or_else(|| {
                self.shown.push(text.to_owned());
                self.shown.len() - 1
            })
    }

    fn ask(&self, desired: Option<String>) -> AlternativeAsk {
        AlternativeAsk {
            excluded: self
                .shown
                .iter()
                .filter_map(|shown| next_word(shown))
                .filter(|word| desired.as_deref() != Some(word.as_str()))
                .collect(),
            // The first suggestion shows where the caret is: after a
            // finished word, it started the next one.
            new_word: self
                .shown
                .first()
                .is_some_and(|first| first.starts_with(char::is_whitespace)),
            desired,
        }
    }

    /// Notes `text`, shown for `fingerprint`, and plans the next press.
    pub(super) fn next(&mut self, fingerprint: &str, text: &str) -> Plan {
        let current = self.note(fingerprint, text);
        let following = self.shown[(current + 1) % self.shown.len()].clone();
        if self.exhausted || self.shown.len() > MAX_ALTERNATIVES {
            return Plan::Show(following);
        }
        Plan::Generate {
            ask: self.ask(None),
            fallback: following,
        }
    }

    /// Plans a numbered list for the shown suggestion: words differing
    /// from every continuation shown for this context. `None` once the
    /// model has no more, or the shown ones already fill the cycle.
    pub(super) fn list_plan(
        &mut self,
        fingerprint: &str,
        text: &str,
        suggestion_id: &str,
    ) -> Option<Plan> {
        self.note(fingerprint, text);
        if self.exhausted || self.shown.len() > MAX_ALTERNATIVES {
            return None;
        }
        Some(Plan::List {
            ask: self.ask(None),
            fallback: text.to_owned(),
            suggestion_id: suggestion_id.to_owned(),
        })
    }

    /// Plans the continuation of a listed word the writer picked.
    pub(super) fn pick_plan(&self, word: &str, fallback: String) -> Plan {
        Plan::Generate {
            ask: self.ask(Some(word.to_lowercase())),
            fallback,
        }
    }

    /// The model had no further continuation for this context.
    pub(super) fn exhaust(&mut self) {
        self.exhausted = true;
    }
}

impl Broker {
    /// Shows the next alternative to the suggestion `payload` names, as an
    /// explicit request for the same context.
    pub(super) async fn request_alternative(
        &self,
        coordinates: Coordinates,
        payload: SessionControlRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        let suggestion_id = payload
            .suggestion_id
            .as_deref()
            .ok_or(BrokerError::InvalidPayload)?;
        let plan = {
            let mut state = self.inner.state.lock().await;
            if state.effective_paused() {
                return Err(BrokerError::Denied(PolicyReason::Paused));
            }
            let session = state.session_with_data_access(coordinates.session_id)?;
            session.ensure_coordinates(coordinates)?;
            session.ensure_fingerprint(&payload.fingerprint)?;
            if let Some(word) = &payload.word {
                // A pick answers a live list, whose suggestion admission
                // already retired: the list itself is the binding.
                let list = session.alternative_list.take();
                let offered = list.filter(|list| {
                    list.fingerprint == payload.fingerprint
                        && list.suggestion_id == suggestion_id
                        && list.expires_at > Instant::now()
                });
                let Some(list) = offered else {
                    return Err(BrokerError::Stale);
                };
                let Some(word) = list
                    .words
                    .iter()
                    .find(|listed| listed.eq_ignore_ascii_case(word))
                    .cloned()
                else {
                    return Err(BrokerError::Stale);
                };
                // The listed suggestion was retired by the list's own
                // admission; the last shown continuation is the fallback.
                let fallback = session
                    .alternatives
                    .shown
                    .last()
                    .cloned()
                    .unwrap_or_default();
                session.alternatives.pick_plan(&word, fallback)
            } else {
                let visible = session.visible.as_ref().ok_or(BrokerError::NoSuggestion)?;
                // A correction has no alternative continuation.
                if visible.payload.suggestion_id != suggestion_id
                    || visible.payload.replace_before.is_some()
                {
                    return Err(BrokerError::Stale);
                }
                let text = visible.payload.text.clone();
                session.alternative_list = None;
                if session.authority.grants(Capability::AlternativeList) {
                    session
                        .alternatives
                        .list_plan(&payload.fingerprint, &text, suggestion_id)
                        .unwrap_or_else(|| session.alternatives.next(&payload.fingerprint, &text))
                } else {
                    session.alternatives.next(&payload.fingerprint, &text)
                }
            }
        };
        let request = SuggestRequestPayload {
            fingerprint: payload.fingerprint,
            explicit: true,
        };
        if let Some(admitted) = self
            .admit_generation(coordinates, request, request_id, Some(plan))
            .await?
        {
            tokio::spawn(self.clone().run_generation(admitted));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
