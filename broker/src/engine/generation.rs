//! Suggestion generation: admission, the provider call, and display of a
//! result that is still bound to the session's current context.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{MutexGuard, OwnedSemaphorePermit};
use tokio::time;
use tokio_util::sync::CancellationToken;

use super::outcomes::current_unix_day;
use super::session::evaluate_context;
use super::type_through::{CarryScope, TypeThrough};
use super::{
    Broker, BrokerError, BrokerEvent, BrokerState, SessionState, VisibleSuggestion, duration_millis,
};
use crate::debug_trace;
use crate::metrics::{CarryMiss, Metrics, NoSuggestionReason};
use crate::personalization::PersonalizationSignal;
use crate::policy::PolicyDecision;
use crate::protocol::{
    Coordinates, ProviderKind, ReasonCode, SuggestCancelPayload, SuggestRequestPayload,
    SuggestionShowPayload,
};
use crate::provider::{
    ProviderError, ProviderOutcome, ProviderReply, ProviderRequest, ProviderStats, RequestTrigger,
    WritingProposal,
};
use crate::segment::{
    accept_word, ends_at_word_boundary, sanitize_suggestion, validate_suggestion_shape,
};

type ProviderResult = Result<Result<ProviderReply, ProviderError>, time::error::Elapsed>;
/// A carried remainder with its deadline and outcome day, or its declining gate.
type CarryOutcome = (Option<(String, Instant, Option<u64>)>, Option<CarryMiss>);

/// A provider call admitted for one session generation.
struct AdmittedGeneration {
    request: ProviderRequest,
    allow_replacement: bool,
    trigger: RequestTrigger,
    binding: GenerationBinding,
    cancellation: CancellationToken,
    provider_permit: OwnedSemaphorePermit,
    /// Quiet battery scheduling: the model starts only after this wait.
    start_delay: Duration,
}

/// What a finished generation must still match before it may be shown.
#[derive(Clone)]
struct GenerationBinding {
    coordinates: Coordinates,
    fingerprint: String,
    generation: u64,
    request_id: Option<String>,
    deadline: Instant,
    before: String,
    after: String,
    language: Option<String>,
}

/// A type-through remainder keeps its original suggestion's deadline and
/// `Shown` aggregate, so typing never extends a display or counts it twice.
#[derive(Clone, Copy)]
struct Carried {
    expires_at: Instant,
    aggregate_day: Option<u64>,
}

/// Why a finished generation shows nothing.
enum Withheld {
    /// Tell the adapter, if the generation is still the session's current one.
    Clear(ReasonCode),
    /// The provider honored a cancellation; whatever superseded it owns the display.
    Silently,
    /// A rejected or late final leaves the streamed partial already shown:
    /// it was checked before display and counts as shown, not as a miss.
    /// Only an edit, focus change, Escape or expiry withdraws it.
    KeepPartial,
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
        if let Some(admitted) = self
            .admit_generation(coordinates, payload, request_id)
            .await?
        {
            tokio::spawn(self.clone().run_generation(admitted));
        }
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
    /// retires any older generation, and either shows a type-through
    /// remainder (`None`) or takes provider capacity for a new generation.
    async fn admit_generation(
        &self,
        coordinates: Coordinates,
        payload: SuggestRequestPayload,
        request_id: Option<String>,
    ) -> Result<Option<AdmittedGeneration>, BrokerError> {
        let metrics = &self.inner.metrics;
        let mut state = self.inner.state.lock().await;
        let paused = state.effective_paused();
        let session = state.session_with_data_access(coordinates.session_id)?;
        let trigger = session.trigger(payload.explicit);
        let generation_timeout = self.inner.config.generation_timeout_for(trigger);
        session.ensure_coordinates(coordinates)?;
        let context = session.context.clone().ok_or(BrokerError::NoContext)?;
        if context.fingerprint != payload.fingerprint {
            return Err(BrokerError::Stale);
        }
        admit_policy(
            metrics,
            evaluate_context(
                &context,
                payload.explicit,
                session.target.target.kind,
                paused,
            ),
        )?;

        // Supersession cancels any older generation before capacity is
        // tested. A saturated broker therefore fails closed without
        // allowing an obsolete suggestion to remain eligible.
        session.retire_generation(metrics, ReasonCode::Superseded);
        let scope = CarryScope::of(session);
        let carry = state.type_through.as_ref();
        let (carried, carry_miss) = consult_carry(carry, &scope, &context, payload.explicit);
        let mid_line = state
            .settings
            .as_ref()
            .is_some_and(|settings| settings.mid_line);
        let escape_quiet = state.quiet_request(coordinates, payload.explicit)?;
        let power_start_delay = state.power_start_delay(trigger);
        let session = state.session_with_data_access(coordinates.session_id)?;
        let mut binding = GenerationBinding {
            coordinates,
            fingerprint: payload.fingerprint,
            generation: session.generation.wrapping_add(1),
            request_id,
            deadline: Instant::now() + generation_timeout,
            before: context.before.clone(),
            after: context.after.clone(),
            language: context.language.clone(),
        };
        if escape_quiet || session.nothing_to_correct(payload.explicit) {
            self.abstain_admitted(state, binding, generation_timeout)
                .await;
            return Ok(None);
        }
        if let Some((text, expires_at, aggregate_day)) = carried {
            session.generation = binding.generation;
            self.renew_context_authority_lease(session, generation_timeout);
            let proposal = WritingProposal {
                text,
                replace_before: None,
            };
            let carried = Carried {
                expires_at,
                aggregate_day,
            };
            record_carry(metrics, &binding, None);
            let cancellation = CancellationToken::new();
            self.show_if_current(state, binding, &cancellation, proposal, Some(carried), true);
            return Ok(None);
        }
        // A carried remainder above needs no model call, so power never
        // delays or refuses it. A deferral extends the deadline so the model
        // keeps its whole writing budget once it starts.
        let word_follows = !mid_line && crate::writing::word_follows_caret(&binding.after);
        let Some(start_delay) = power_start_delay.filter(|_| !word_follows) else {
            self.abstain_admitted(state, binding, generation_timeout)
                .await;
            return Ok(None);
        };
        binding.deadline += start_delay;
        let generation_timeout = generation_timeout + start_delay;
        let provider_permit = Arc::clone(&self.inner.provider_admissions)
            .try_acquire_owned()
            .map_err(|_| {
                metrics.record_provider_error();
                metrics.record_no_suggestion(NoSuggestionReason::ProviderError);
                BrokerError::ProviderBusy
            })?;
        // The request reaches the provider although a carry existed; count
        // the gate that declined it. An abstention above calls no provider
        // and records nothing.
        if let Some(miss) = carry_miss {
            record_carry(metrics, &binding, Some(miss));
        }
        session.generation = binding.generation;
        let cancellation = CancellationToken::new();
        session.cancellation = Some(cancellation.clone());
        self.renew_context_authority_lease(session, generation_timeout);
        Ok(Some(AdmittedGeneration {
            allow_replacement: session.allows_text_replacement(),
            binding,
            request: ProviderRequest {
                before: context.before,
                after: context.after,
                language: context.language,
            },
            trigger,
            cancellation,
            provider_permit,
            start_delay,
        }))
    }

    /// Settles an admitted binding without a provider call: the adapter is
    /// told there is no suggestion, and the abstention is counted.
    async fn abstain_admitted(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        binding: GenerationBinding,
        generation_timeout: Duration,
    ) {
        if let Ok(session) = state.session_with_data_access(binding.coordinates.session_id) {
            session.generation = binding.generation;
            self.renew_context_authority_lease(session, generation_timeout);
        }
        drop(state);
        self.inner
            .metrics
            .record_no_suggestion(NoSuggestionReason::RequestAbstained);
        self.clear_failed_generation(binding, ReasonCode::NoSuggestion)
            .await;
    }

    #[allow(clippy::too_many_lines)]
    async fn run_generation(self, admitted: AdmittedGeneration) {
        let AdmittedGeneration {
            request,
            allow_replacement,
            trigger,
            binding,
            cancellation,
            provider_permit,
            start_delay,
        } = admitted;
        // Admission occurs before spawning, so there can never be more
        // generation tasks than permits. The permit is released on every
        // return path, including cancellation and timeout.
        let _provider_permit = provider_permit;
        let shutdown = &self.inner.shutdown;
        let delay = self.inner.config.debounce + start_delay;
        if delay != Duration::ZERO {
            tokio::select! {
                () = time::sleep(delay) => {}
                () = cancellation.cancelled() => {
                    self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                    return;
                }
                () = shutdown.cancelled() => {
                    self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                    return;
                }
            }
        }
        if cancellation.is_cancelled() || shutdown.is_cancelled() {
            self.inner
                .metrics
                .record_no_suggestion(NoSuggestionReason::Stale);
            return;
        }
        self.inner.metrics.record_provider_call(request.byte_len());
        let started = Instant::now();
        debug_trace::emit("model start", |entry| {
            entry.put("sid", debug_trace::session(binding.coordinates.session_id));
            entry.put("trigger", format!("{trigger:?}").to_lowercase());
            entry.put("input_bytes", request.byte_len());
            entry.put("waited_ms", duration_millis(delay));
        });
        let provider_deadline = Instant::now() + self.inner.config.provider_timeout_for(trigger);
        let deadline = binding.deadline.min(provider_deadline);
        let stream_partials = request.after.is_empty();
        let (partial_tx, mut partial_rx) = tokio::sync::mpsc::unbounded_channel();
        let provider = time::timeout_at(
            deadline.into(),
            self.inner.provider.propose_outcome_with_partials(
                request,
                cancellation.clone(),
                allow_replacement,
                trigger,
                stream_partials.then_some(partial_tx),
            ),
        );
        tokio::pin!(provider);
        let mut shown_partial = None;
        let mut carried = None;
        let mut partials_open = stream_partials;
        loop {
            tokio::select! {
                biased;
                () = cancellation.cancelled() => {
                    if shown_partial.is_none() {
                        self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                    }
                    // The provider future is dropped here, so its counts are
                    // gone; a shown partial proves a first token arrived.
                    trace_model_done(&binding, started, "cancelled", None, None,
                        shown_partial.is_none());
                    return;
                }
                () = shutdown.cancelled() => {
                    if shown_partial.is_none() {
                        self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                    }
                    trace_model_done(&binding, started, "shutdown", None, None,
                        shown_partial.is_none());
                    return;
                }
                partial = partial_rx.recv(), if partials_open => {
                    match partial {
                        Some(text) => {
                            let extends = shown_partial.as_deref().is_none_or(|previous: &str| {
                                text != previous && text.starts_with(previous)
                            });
                            if !extends {
                                continue;
                            }
                            let Some(proposal) = checked_proposal(
                                self.inner.provider_kind,
                                &binding.before,
                                &binding.after,
                                WritingProposal { text: text.clone(), replace_before: None },
                            ) else {
                                continue;
                            };
                            let state = self.inner.state.lock().await;
                            if let Some(updated) = self.show_if_current(
                                state,
                                binding.clone(),
                                &cancellation,
                                proposal,
                                carried,
                                false,
                            ) {
                                carried = Some(updated);
                                shown_partial = Some(text);
                            }
                        }
                        None => partials_open = false,
                    }
                }
                result = &mut provider => {
                    let (outcome, text, stats) = match &result {
                        Ok(Ok(ProviderReply {
                            outcome: ProviderOutcome::Proposal(raw),
                            stats,
                        })) => ("proposal".to_owned(), Some(raw.text.as_str()), *stats),
                        Ok(Ok(ProviderReply {
                            outcome: ProviderOutcome::NoSuggestion(reason),
                            stats,
                        })) => (trace_name(reason), None, *stats),
                        Ok(Err(ProviderError::Cancelled)) => {
                            ("cancelled".to_owned(), None, None)
                        }
                        Ok(Err(ProviderError::Unavailable)) => {
                            ("unavailable".to_owned(), None, None)
                        }
                        Err(_) => ("timeout".to_owned(), None, None),
                    };
                    let cancelled_before_first_token = matches!(
                        outcome.as_str(), "cancelled" | "timeout")
                        && stats.is_none_or(|stats| stats.ttft.is_none())
                        && shown_partial.is_none();
                    trace_model_done(&binding, started, &outcome, text, stats.as_ref(),
                        cancelled_before_first_token);
                    self.finish_generation(binding, &cancellation, result, shown_partial, carried)
                        .await;
                    return;
                }
            }
        }
    }

    /// Every path records exactly one no-suggestion class or one shown
    /// result, except a finished generation whose streamed partial stays:
    /// that partial already recorded the shown result.
    async fn finish_generation(
        &self,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        result: ProviderResult,
        shown_partial: Option<String>,
        carried: Option<Carried>,
    ) {
        let keep_partial = shown_partial.as_deref().is_some_and(ends_at_word_boundary);
        let proposal =
            match self.check_provider_result(&binding, cancellation, result, keep_partial) {
                Ok(proposal) => proposal,
                Err(Withheld::Clear(reason)) => {
                    return self.clear_failed_generation(binding, reason).await;
                }
                Err(Withheld::Silently) => return,
                Err(Withheld::KeepPartial) => {
                    self.release_finished_generation(&binding).await;
                    return;
                }
            };
        if shown_partial
            .as_ref()
            .is_some_and(|shown| shown == &proposal.text)
        {
            self.release_finished_generation(&binding).await;
            return;
        }
        let state = self.inner.state.lock().await;
        if Instant::now() >= binding.deadline {
            drop(state);
            // The extension is late, but the partial is already the shown
            // suggestion; dropping the extension must not withdraw it.
            if keep_partial {
                self.release_finished_generation(&binding).await;
                return;
            }
            self.record_generation_timeout(cancellation);
            return self
                .clear_failed_generation(binding, ReasonCode::ProviderTimeout)
                .await;
        }
        self.show_if_current(state, binding, cancellation, proposal, carried, true);
    }

    /// The stream finished on text already shown. Drop its cancel token without
    /// withdrawing that suggestion or counting it again.
    async fn release_finished_generation(&self, binding: &GenerationBinding) {
        let mut state = self.inner.state.lock().await;
        let Some(session) = state.sessions.get_mut(&binding.coordinates.session_id) else {
            return;
        };
        if session.generation == binding.generation && session.coordinates == binding.coordinates {
            session.cancellation = None;
        }
    }

    fn check_provider_result(
        &self,
        binding: &GenerationBinding,
        cancellation: &CancellationToken,
        result: ProviderResult,
        keep_partial: bool,
    ) -> Result<WritingProposal, Withheld> {
        let metrics = &self.inner.metrics;
        if Instant::now() >= binding.deadline {
            if keep_partial {
                cancellation.cancel();
                return Err(Withheld::KeepPartial);
            }
            self.record_generation_timeout(cancellation);
            return Err(Withheld::Clear(ReasonCode::ProviderTimeout));
        }
        let raw = match result {
            Ok(Ok(ProviderReply {
                outcome: ProviderOutcome::Proposal(raw),
                ..
            })) => raw,
            Ok(Ok(ProviderReply {
                outcome: ProviderOutcome::NoSuggestion(reason),
                ..
            })) => {
                // The provider's own rejection of its whole output (the
                // local model's `validate_proposal`, reported here rather
                // than as a proposal) must not withdraw a partial it had
                // already streamed and the broker had checked and shown.
                if keep_partial {
                    return Err(Withheld::KeepPartial);
                }
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
                if keep_partial {
                    cancellation.cancel();
                    return Err(Withheld::KeepPartial);
                }
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
            if keep_partial {
                return Withheld::KeepPartial;
            }
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

    #[allow(clippy::too_many_lines)]
    fn show_if_current(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        proposal: WritingProposal,
        carried: Option<Carried>,
        settle: bool,
    ) -> Option<Carried> {
        let metrics = &self.inner.metrics;
        let settings_revision = state.settings_revision;
        let Some(session) = session_awaiting(&mut state, &binding, cancellation) else {
            if settle {
                metrics.record_stale_result();
                metrics.record_no_suggestion(NoSuggestionReason::Stale);
            }
            return None;
        };
        // A commit or dismissal already took the suggestion. A later word must
        // not replace it.
        if !settle && session.pending.is_some() {
            return None;
        }
        if settle {
            session.cancellation = None;
        }
        let GenerationBinding {
            coordinates,
            fingerprint,
            generation,
            request_id,
            deadline,
            before,
            after,
            language,
        } = binding;
        if proposal.replace_before.is_none() && session.spelling_only() {
            metrics.record_no_suggestion(NoSuggestionReason::ModelAbstained);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                fingerprint,
                ReasonCode::NoSuggestion,
                request_id,
            ));
            return None;
        }
        if proposal.replace_before.is_some() && !session.allows_text_replacement() {
            metrics.record_provider_error();
            metrics.record_no_suggestion(NoSuggestionReason::OutputRejected);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                fingerprint,
                ReasonCode::InvalidCapability,
                request_id,
            ));
            return None;
        }
        let now = Instant::now();
        let full_ttl = self.inner.config.suggestion_ttl_for(&session.authority);
        let expires_at = carried.as_ref().map_or(now + full_ttl, |carried| {
            carried.expires_at.min(now + full_ttl)
        });
        let suggestion_ttl = expires_at.saturating_duration_since(now);
        let payload = self.suggestion_payload(fingerprint, proposal, suggestion_ttl);
        if Instant::now() >= deadline {
            self.record_generation_timeout(cancellation);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                payload.fingerprint,
                ReasonCode::ProviderTimeout,
                request_id,
            ));
            return None;
        }
        let show = BrokerEvent::SuggestionShow {
            coordinates,
            payload: payload.clone(),
            request_id: request_id.clone(),
        };
        if session.sink.send(show).is_err() {
            metrics.record_no_suggestion(NoSuggestionReason::Stale);
            return None;
        }
        // A carried remainder is the same suggestion, not another show.
        if carried.is_none() {
            metrics.record_suggestion_shown();
        }
        let aggregate_day = match carried {
            Some(carried) => carried.aggregate_day,
            None => current_unix_day().filter(|&event_day| {
                self.queue_outcome(
                    &session.target.target,
                    settings_revision,
                    event_day,
                    PersonalizationSignal::Shown,
                )
            }),
        };
        let suggestion_id = payload.suggestion_id.clone();
        // A correction replaces typed text, so typing cannot carry it.
        let type_through = payload.replace_before.is_none().then(|| TypeThrough {
            scope: CarryScope::of(session),
            before,
            after,
            language,
            text: payload.text.clone(),
            expires_at,
            aggregate_day,
        });
        let earlier = session.visible.take().map_or_else(Vec::new, |previous| {
            if previous.expires_at == expires_at
                && previous.payload.fingerprint == payload.fingerprint
                && previous.payload.replace_before.is_none()
                && payload.replace_before.is_none()
                && payload.text.len() > previous.payload.text.len()
                && payload.text.starts_with(&previous.payload.text)
            {
                // Each version adds at least one character, so the protocol's
                // 64-character suggestion limit also bounds this collection.
                let mut earlier = previous.earlier;
                earlier.push(previous.payload);
                earlier
            } else {
                Vec::new()
            }
        });
        session.visible = Some(VisibleSuggestion {
            payload,
            earlier,
            expires_at,
            request_id,
            aggregate_day,
        });
        state.type_through = type_through;
        drop(state);

        let broker = self.clone();
        tokio::spawn(async move {
            time::sleep(suggestion_ttl).await;
            broker
                .expire_suggestion(coordinates, generation, suggestion_id)
                .await;
        });
        Some(Carried {
            expires_at,
            aggregate_day,
        })
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
        session.visible = None;
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

/// The carried remainder for this admitted request (ADR 0004), or the gate
/// that declined the carry. Both are `None` when no carry exists.
/// A metrics class as the trace names it: its `snake_case` wire name.
fn trace_name(class: impl serde::Serialize) -> String {
    serde_json::to_value(class)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Counts and traces a type-through decision: carried, or the gate that
/// declined although the request then reached the provider.
fn record_carry(metrics: &Metrics, binding: &GenerationBinding, miss: Option<CarryMiss>) {
    match miss {
        None => metrics.record_type_through_carried(),
        Some(miss) => metrics.record_carry_miss(miss),
    }
    debug_trace::emit("carry", |entry| {
        entry.put("sid", debug_trace::session(binding.coordinates.session_id));
        entry.put(
            "outcome",
            miss.map_or_else(|| "carried".to_owned(), trace_name),
        );
    });
}

/// One provider call's end, with its latency, outcome class and the
/// provider's content-free counts when it measured the call.
fn trace_model_done(
    binding: &GenerationBinding,
    started: Instant,
    outcome: &str,
    text: Option<&str>,
    stats: Option<&ProviderStats>,
    cancelled_before_first_token: bool,
) {
    debug_trace::emit("model done", |entry| {
        entry.put("sid", debug_trace::session(binding.coordinates.session_id));
        entry.put("ms", duration_millis(started.elapsed()));
        entry.put("outcome", outcome);
        if let Some(stats) = stats {
            if let Some(ttft) = stats.ttft {
                entry.put("ttft_ms", duration_millis(ttft));
            }
            if let Some(prompt_tokens) = stats.prompt_tokens {
                entry.put("prompt_tokens", prompt_tokens);
            }
            if let Some(cache_tokens) = stats.cache_tokens {
                entry.put("cache_tokens", cache_tokens);
            }
            entry.put("generated_tokens", stats.generated_tokens);
        }
        if matches!(outcome, "cancelled" | "shutdown" | "timeout") {
            entry.put("cancelled_before_first_token", cancelled_before_first_token);
        }
        if let Some(text) = text {
            entry.put("chars", text.chars().count());
            entry.text("text", text);
        }
    });
}

fn consult_carry(
    carry: Option<&TypeThrough>,
    scope: &CarryScope,
    context: &crate::protocol::ContextChangedPayload,
    explicit: bool,
) -> CarryOutcome {
    match carry.map(|carry| {
        (
            carry.remainder(scope, context, Instant::now(), explicit),
            carry.expires_at,
            carry.aggregate_day,
        )
    }) {
        Some((Ok(text), expires_at, aggregate_day)) => {
            (Some((text, expires_at, aggregate_day)), None)
        }
        Some((Err(miss), _, _)) => (None, Some(miss)),
        None => (None, None),
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
        // A continuation may keep one trailing space; the shape check allows
        // it only where it separates the insertion from the next word.
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

/// Counts and maps a request's policy decision; only `Allow` admits it.
fn admit_policy(
    metrics: &crate::metrics::Metrics,
    decision: PolicyDecision,
) -> Result<(), BrokerError> {
    match decision {
        PolicyDecision::Allow(_) => Ok(()),
        PolicyDecision::ManualRequired(_) => {
            metrics.record_manual_required();
            Err(BrokerError::ManualRequired)
        }
        PolicyDecision::Deny(reason) => {
            metrics.record_denied();
            Err(BrokerError::Denied(reason))
        }
    }
}
