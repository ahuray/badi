//! Suggestion generation: admission, the provider call, and display of a
//! result that is still bound to the session's current context.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{MutexGuard, OwnedSemaphorePermit};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::outcomes::current_unix_day;
use super::session::evaluate_context;
use super::{
    Broker, BrokerError, BrokerEvent, BrokerState, SessionState, VisibleSuggestion, duration_millis,
};
use crate::metrics::NoSuggestionReason;
use crate::personalization::PersonalizationSignal;
use crate::policy::PolicyDecision;
use crate::protocol::{
    Coordinates, ProviderKind, ReasonCode, SuggestCancelPayload, SuggestRequestPayload,
    SuggestionShowPayload,
};
use crate::provider::{
    ProviderError, ProviderOutcome, ProviderRequest, RequestTrigger, WritingProposal,
};
use crate::segment::{accept_word, sanitize_suggestion, validate_suggestion_shape};

type ProviderResult = Result<Result<ProviderOutcome, ProviderError>, time::error::Elapsed>;

/// A provider call admitted for one session generation.
struct AdmittedGeneration {
    request: ProviderRequest,
    allow_replacement: bool,
    trigger: RequestTrigger,
    binding: GenerationBinding,
    cancellation: CancellationToken,
    provider_permit: OwnedSemaphorePermit,
}

/// What a finished generation must still match before it may be shown.
struct GenerationBinding {
    coordinates: Coordinates,
    fingerprint: String,
    generation: u64,
    request_id: Option<String>,
    deadline: Instant,
    before: String,
    after: String,
}

/// Why a finished generation shows nothing.
enum Withheld {
    /// Tell the adapter, if the generation is still the session's current one.
    Clear(ReasonCode),
    /// The provider honored a cancellation; whatever superseded it owns the display.
    Silently,
}

impl Broker {
    pub async fn request_suggestion(
        &self,
        coordinates: Coordinates,
        payload: SuggestRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_fingerprint(&payload.fingerprint)?;
        if request_id
            .as_deref()
            .is_some_and(|request_id| !crate::protocol::valid_opaque_id(request_id))
        {
            return Err(BrokerError::InvalidPayload);
        }
        let admitted = self
            .admit_generation(coordinates, payload, request_id)
            .await?;
        tokio::spawn(self.clone().run_generation(admitted));
        Ok(())
    }

    pub async fn cancel_suggestion(
        &self,
        coordinates: Coordinates,
        payload: SuggestCancelPayload,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_fingerprint(&payload.fingerprint)?;
        let mut state = self.inner.state.lock().await;
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        session.ensure_coordinates(coordinates)?;
        session.ensure_fingerprint(&payload.fingerprint)?;
        session.retire_generation(&self.inner.metrics, payload.reason);
        Ok(())
    }

    /// Checks the request against the session's current context and policy,
    /// retires any older generation, and takes provider capacity for this one.
    async fn admit_generation(
        &self,
        coordinates: Coordinates,
        payload: SuggestRequestPayload,
        request_id: Option<String>,
    ) -> Result<AdmittedGeneration, BrokerError> {
        let trigger = RequestTrigger::from_explicit(payload.explicit);
        let generation_timeout = self.inner.config.generation_timeout_for(trigger);
        let metrics = &self.inner.metrics;
        let mut state = self.inner.state.lock().await;
        let paused = state.effective_paused();
        let session = state.session_with_data_access(coordinates.session_id)?;
        session.ensure_coordinates(coordinates)?;
        let context = session.context.clone().ok_or(BrokerError::NoContext)?;
        if context.fingerprint != payload.fingerprint {
            return Err(BrokerError::Stale);
        }
        match evaluate_context(
            &context,
            payload.explicit,
            session.target.target.kind,
            paused,
        ) {
            PolicyDecision::Allow(_) => {}
            PolicyDecision::ManualRequired(_) => {
                metrics.record_manual_required();
                return Err(BrokerError::ManualRequired);
            }
            PolicyDecision::Deny(reason) => {
                metrics.record_denied();
                return Err(BrokerError::Denied(reason));
            }
        }

        // Supersession cancels any older generation before capacity is
        // tested. A saturated broker therefore fails closed without
        // allowing an obsolete suggestion to remain eligible.
        session.retire_generation(metrics, ReasonCode::Superseded);
        let provider_permit = Arc::clone(&self.inner.provider_admissions)
            .try_acquire_owned()
            .map_err(|_| {
                metrics.record_provider_error();
                BrokerError::ProviderBusy
            })?;
        session.generation = session.generation.wrapping_add(1);
        let cancellation = CancellationToken::new();
        session.cancellation = Some(cancellation.clone());
        self.renew_context_authority_lease(session, generation_timeout);
        Ok(AdmittedGeneration {
            allow_replacement: session.allows_text_replacement(),
            binding: GenerationBinding {
                coordinates,
                fingerprint: payload.fingerprint,
                generation: session.generation,
                request_id,
                deadline: Instant::now() + generation_timeout,
                before: context.before.clone(),
                after: context.after.clone(),
            },
            request: ProviderRequest {
                before: context.before,
                after: context.after,
                language: context.language,
            },
            trigger,
            cancellation,
            provider_permit,
        })
    }

    async fn run_generation(self, admitted: AdmittedGeneration) {
        let AdmittedGeneration {
            request,
            allow_replacement,
            trigger,
            binding,
            cancellation,
            provider_permit,
        } = admitted;
        // Admission occurs before spawning, so there can never be more
        // generation tasks than permits. The permit is released on every
        // return path, including cancellation and timeout.
        let _provider_permit = provider_permit;
        let shutdown = &self.inner.shutdown;
        if self.inner.config.debounce != Duration::ZERO {
            tokio::select! {
                () = time::sleep(self.inner.config.debounce) => {}
                () = cancellation.cancelled() => return,
                () = shutdown.cancelled() => return,
            }
        }
        if cancellation.is_cancelled() || shutdown.is_cancelled() {
            return;
        }
        self.inner.metrics.record_provider_call(request.byte_len());
        let provider_deadline = Instant::now() + self.inner.config.provider_timeout;
        let deadline = binding.deadline.min(provider_deadline);
        let result = tokio::select! {
            () = cancellation.cancelled() => {
                self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                return;
            }
            () = shutdown.cancelled() => {
                self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                return;
            }
            result = time::timeout_at(
                deadline.into(),
                self.inner.provider.propose_outcome(
                    request,
                    cancellation.clone(),
                    allow_replacement,
                    trigger,
                ),
            ) => result,
        };
        self.finish_generation(binding, &cancellation, result).await;
    }

    /// Every path records exactly one no-suggestion class or one shown result.
    async fn finish_generation(
        &self,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        result: ProviderResult,
    ) {
        let proposal = match self.check_provider_result(&binding, cancellation, result) {
            Ok(proposal) => proposal,
            Err(Withheld::Clear(reason)) => {
                return self.clear_failed_generation(binding, reason).await;
            }
            Err(Withheld::Silently) => return,
        };
        let state = self.inner.state.lock().await;
        if Instant::now() >= binding.deadline {
            drop(state);
            self.record_generation_timeout(cancellation);
            return self
                .clear_failed_generation(binding, ReasonCode::ProviderTimeout)
                .await;
        }
        self.show_if_current(state, binding, cancellation, proposal);
    }

    fn check_provider_result(
        &self,
        binding: &GenerationBinding,
        cancellation: &CancellationToken,
        result: ProviderResult,
    ) -> Result<WritingProposal, Withheld> {
        let metrics = &self.inner.metrics;
        if Instant::now() >= binding.deadline {
            self.record_generation_timeout(cancellation);
            return Err(Withheld::Clear(ReasonCode::ProviderTimeout));
        }
        let raw = match result {
            Ok(Ok(ProviderOutcome::Proposal(raw))) => raw,
            Ok(Ok(ProviderOutcome::NoSuggestion(reason))) => {
                metrics.record_no_suggestion(reason);
                return Err(Withheld::Clear(ReasonCode::NoSuggestion));
            }
            Ok(Err(ProviderError::Cancelled)) => {
                metrics.record_no_suggestion(NoSuggestionReason::Stale);
                return Err(Withheld::Silently);
            }
            Ok(Err(ProviderError::Unavailable)) => {
                metrics.record_provider_error();
                metrics.record_no_suggestion(NoSuggestionReason::ProviderError);
                return Err(Withheld::Clear(ReasonCode::ProviderError));
            }
            Err(_elapsed) => {
                self.record_generation_timeout(cancellation);
                return Err(Withheld::Clear(ReasonCode::ProviderTimeout));
            }
        };
        metrics.record_provider_output(raw.text.len());
        checked_proposal(
            self.inner.provider_kind,
            &binding.before,
            &binding.after,
            raw,
        )
        .ok_or_else(|| {
            metrics.record_provider_error();
            metrics.record_no_suggestion(NoSuggestionReason::OutputRejected);
            Withheld::Clear(ReasonCode::InvalidOutput)
        })
    }

    fn record_generation_timeout(&self, cancellation: &CancellationToken) {
        cancellation.cancel();
        self.inner.metrics.record_provider_error();
        self.inner
            .metrics
            .record_no_suggestion(NoSuggestionReason::Timeout);
    }

    fn show_if_current(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        proposal: WritingProposal,
    ) {
        let metrics = &self.inner.metrics;
        let settings_revision = state.settings_revision;
        let Some(session) = session_awaiting(&mut state, &binding, cancellation) else {
            metrics.record_stale_result();
            metrics.record_no_suggestion(NoSuggestionReason::Stale);
            return;
        };
        session.cancellation = None;
        let GenerationBinding {
            coordinates,
            fingerprint,
            generation,
            request_id,
            deadline,
            ..
        } = binding;
        if proposal.replace_before.is_some() && !session.allows_text_replacement() {
            metrics.record_provider_error();
            metrics.record_no_suggestion(NoSuggestionReason::OutputRejected);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                fingerprint,
                ReasonCode::InvalidCapability,
                request_id,
            ));
            return;
        }
        let suggestion_ttl = self.inner.config.suggestion_ttl_for(&session.authority);
        let expires_at = Instant::now() + suggestion_ttl;
        let payload = self.suggestion_payload(fingerprint, proposal, suggestion_ttl);
        if Instant::now() >= deadline {
            self.record_generation_timeout(cancellation);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                payload.fingerprint,
                ReasonCode::ProviderTimeout,
                request_id,
            ));
            return;
        }
        let show = BrokerEvent::SuggestionShow {
            coordinates,
            payload: payload.clone(),
            request_id: request_id.clone(),
        };
        if session.sink.send(show).is_err() {
            metrics.record_no_suggestion(NoSuggestionReason::Stale);
            return;
        }
        metrics.record_suggestion_shown();
        let aggregate_day = current_unix_day().filter(|&event_day| {
            self.queue_outcome(
                &session.target.target,
                settings_revision,
                event_day,
                PersonalizationSignal::Shown,
            )
        });
        let suggestion_id = payload.suggestion_id.clone();
        session.visible = Some(VisibleSuggestion {
            payload,
            expires_at,
            request_id,
            aggregate_day,
        });
        drop(state);

        let broker = self.clone();
        tokio::spawn(async move {
            time::sleep(suggestion_ttl).await;
            broker
                .expire_suggestion(coordinates, generation, suggestion_id)
                .await;
        });
    }

    fn suggestion_payload(
        &self,
        fingerprint: String,
        proposal: WritingProposal,
        ttl: Duration,
    ) -> SuggestionShowPayload {
        let WritingProposal {
            text,
            replace_before,
        } = proposal;
        SuggestionShowPayload {
            fingerprint,
            suggestion_id: format!("s:{}", uuid::Uuid::new_v4()),
            // A spelling correction is accepted whole, even word by word.
            accept_word: if replace_before.is_some() {
                text.clone()
            } else {
                accept_word(&text).accepted
            },
            text,
            replace_before,
            ttl_ms: duration_millis(ttl),
            provider: self.inner.provider_kind,
        }
    }

    /// Tells the adapter its request ended without a suggestion, unless a
    /// newer generation already owns the session.
    async fn clear_failed_generation(&self, binding: GenerationBinding, reason: ReasonCode) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&binding.coordinates.session_id) else {
            return;
        };
        if session.generation != binding.generation || session.coordinates != binding.coordinates {
            self.inner.metrics.record_stale_result();
            return;
        }
        session.cancellation = None;
        let _ = session.sink.send(BrokerEvent::generation_withdrawn(
            binding.coordinates,
            binding.fingerprint,
            reason,
            binding.request_id,
        ));
    }

    async fn expire_suggestion(
        &self,
        coordinates: Coordinates,
        generation: u64,
        suggestion_id: String,
    ) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&coordinates.session_id) else {
            return;
        };
        if session.generation != generation || session.coordinates != coordinates {
            return;
        }
        let Some(visible) = session.visible.take_if(|visible| {
            visible.payload.suggestion_id == suggestion_id && visible.expires_at <= Instant::now()
        }) else {
            return;
        };
        let _ = session
            .sink
            .send(visible.into_clear(coordinates, ReasonCode::Expired));
        self.inner.metrics.record_suggestion_expired();
    }
}

/// The session still awaiting exactly this generation, while policy lets it
/// receive suggestions.
fn session_awaiting<'state>(
    state: &'state mut BrokerState,
    binding: &GenerationBinding,
    cancellation: &CancellationToken,
) -> Option<&'state mut SessionState> {
    if state.effective_paused() {
        return None;
    }
    let session = state
        .session_with_data_access(binding.coordinates.session_id)
        .ok()?;
    let is_current = session.generation == binding.generation
        && session.coordinates == binding.coordinates
        && session.context_fingerprint() == Some(binding.fingerprint.as_str())
        && !cancellation.is_cancelled();
    is_current.then_some(session)
}

/// Display-side safety and shape checks shared by generation and diagnostic
/// probes. A replacement must own the exact typed suffix at a word boundary.
pub(super) fn checked_proposal(
    provider_kind: ProviderKind,
    before: &str,
    after: &str,
    raw: WritingProposal,
) -> Option<WritingProposal> {
    let (text, replace_before) = if let Some(original) = raw.replace_before.as_deref() {
        // A correction may preserve the single space typed after its word.
        // General continuation sanitization stays strict.
        crate::protocol::valid_spelling_replacement(original, &raw.text)
            .then_some((raw.text, raw.replace_before))?
    } else {
        (sanitize_suggestion(&raw.text).ok()?, None)
    };
    let shape_valid = match replace_before.as_deref() {
        Some(original) => {
            crate::protocol::valid_spelling_replacement(original, &text)
                && before.ends_with(original)
                && after.is_empty()
                && before[..before.len() - original.len()]
                    .chars()
                    .next_back()
                    .is_none_or(char::is_whitespace)
        }
        None => continuation_shape_valid(provider_kind, before, after, &text),
    };
    shape_valid.then_some(WritingProposal {
        text,
        replace_before,
    })
}

#[cfg(feature = "local-model")]
fn continuation_shape_valid(
    provider_kind: ProviderKind,
    before: &str,
    after: &str,
    text: &str,
) -> bool {
    if provider_kind == ProviderKind::LocalModel {
        crate::writing::validate_suggestion_shape(before, after, text).is_ok()
    } else {
        validate_suggestion_shape(before, after, text).is_ok()
    }
}

#[cfg(not(feature = "local-model"))]
fn continuation_shape_valid(
    _provider_kind: ProviderKind,
    before: &str,
    after: &str,
    text: &str,
) -> bool {
    validate_suggestion_shape(before, after, text).is_ok()
}
