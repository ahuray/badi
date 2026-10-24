//! Alternatives on request: on a shown continuation, Ctrl+Shift+Space asks
//! the model for its next-best one, starting with a word not shown yet for
//! that context. After [`MAX_ALTERNATIVES`], or once the model has none,
//! further presses cycle through those already shown, with no model call.
//! Each is an ordinary suggestion: Tab and Ctrl+Right accept it as usual.

use super::rejections::next_word;
use super::{Broker, BrokerError};
use crate::policy::PolicyReason;
use crate::protocol::{Coordinates, SessionControlRequestPayload, SuggestRequestPayload};
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
    /// Show this continuation again, with no model call.
    Show(String),
}

impl Alternatives {
    /// Notes `text`, shown for `fingerprint`, and plans the next press.
    pub(super) fn next(&mut self, fingerprint: &str, text: &str) -> Plan {
        if self.fingerprint != fingerprint {
            *self = Self {
                fingerprint: fingerprint.to_owned(),
                ..Self::default()
            };
        }
        let current = self
            .shown
            .iter()
            .position(|shown| shown == text)
            .unwrap_or_else(|| {
                self.shown.push(text.to_owned());
                self.shown.len() - 1
            });
        let following = self.shown[(current + 1) % self.shown.len()].clone();
        if self.exhausted || self.shown.len() > MAX_ALTERNATIVES {
            return Plan::Show(following);
        }
        Plan::Generate {
            ask: AlternativeAsk {
                excluded: self
                    .shown
                    .iter()
                    .filter_map(|shown| next_word(shown))
                    .collect(),
                // The first suggestion shows where the caret is: after a
                // finished word, it started the next one.
                new_word: self.shown[0].starts_with(char::is_whitespace),
            },
            fallback: following,
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
            let visible = session.visible.as_ref().ok_or(BrokerError::NoSuggestion)?;
            // A correction has no alternative continuation.
            if visible.payload.suggestion_id != suggestion_id
                || visible.payload.replace_before.is_some()
            {
                return Err(BrokerError::Stale);
            }
            let text = visible.payload.text.clone();
            session.alternatives.next(&payload.fingerprint, &text)
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
