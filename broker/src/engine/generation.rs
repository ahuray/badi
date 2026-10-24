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
    ContextChangedPayload, Coordinates, FieldPurpose, ProviderKind, ReasonCode,
    SuggestCancelPayload, SuggestRequestPayload, SuggestionShowPayload, TargetDescriptor,
    TargetKind,
};
use crate::provider::{
    ProviderError, ProviderOutcome, ProviderReply, ProviderRequest, ProviderStats, RequestTrigger,
    WritingProposal, continuation_shape_valid,
};
use crate::segment::{accept_word, sanitize_suggestion};
use crate::settings::{SettingsV2, StableIdentity};
use crate::stats::StatsEvent;

pub(super) type ProviderResult = Result<Result<ProviderReply, ProviderError>, time::error::Elapsed>;

/// What one provider call produced, classified once for generation, its
/// trace and diagnostic probes.
pub(super) enum ProviderAnswer {
    Proposal(WritingProposal),
    NoSuggestion(NoSuggestionReason),
    Cancelled,
    Unavailable,
    TimedOut,
}

impl ProviderAnswer {
    /// The answer and the provider's counts, when it reported any.
    pub(super) fn classify(result: ProviderResult) -> (Self, Option<ProviderStats>) {
        match result {
            Ok(Ok(ProviderReply {
                outcome: ProviderOutcome::Proposal(raw),
                stats,
            })) => (Self::Proposal(raw), stats),
            Ok(Ok(ProviderReply {
                outcome: ProviderOutcome::NoSuggestion(reason),
                stats,
            })) => (Self::NoSuggestion(reason), stats),
            Ok(Err(ProviderError::Cancelled)) => (Self::Cancelled, None),
            Ok(Err(ProviderError::Unavailable)) => (Self::Unavailable, None),
            Err(_elapsed) => (Self::TimedOut, None),
        }
    }

    fn trace_name(&self) -> String {
        match self {
            Self::Proposal(_) => "proposal".to_owned(),
            Self::NoSuggestion(reason) => trace_name(reason),
            Self::Cancelled => "cancelled".to_owned(),
            Self::Unavailable => "unavailable".to_owned(),
            Self::TimedOut => "timeout".to_owned(),
        }
    }
}
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
    /// With context on: the subject whose learned words apply and the text
    /// near the field. Learned words are read after the state lock is gone.
    vocabulary: Option<(StableIdentity, String)>,
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
    /// Letters were typed at a word start in front of `after`'s first word,
    /// so the suggestion ends in the space that keeps them apart.
    separates_next_word: bool,
    /// The app or site, for words the writer turned down there.
    scope: CarryScope,
    /// The writer asked: turned-down words and quiet floors do not apply.
    explicit: bool,
    /// The app's stricter confidence floor while its suggestions are seldom
    /// taken; `None` keeps the provider's own.
    quiet_floor: Option<f64>,
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
    #[allow(clippy::too_many_lines)]
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
        let target = session.target.target.clone();
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
        let separates_next_word = state.typed_at_word_start(&scope, &context);
        let carry = state.type_through.as_ref();
        let (carried, carry_miss) = consult_carry(carry, &scope, &context, payload.explicit);
        if carry_miss == Some(CarryMiss::PrefixMismatch) {
            state.record_typed_over(&scope, &context);
        }
        let mid_line = state
            .settings
            .as_ref()
            .is_some_and(|settings| settings.mid_line);
        let context_on = context_on(state.settings.as_ref(), &target);
        if let Some((identity, revision)) = &context_on {
            let fresh = state.fresh_written_words(&target, &context);
            if !fresh.is_empty() {
                self.queue_learned_words(identity.clone(), *revision, fresh);
            }
        }
        let escape_quiet = state.quiet_request(coordinates, payload.explicit)?;
        let power_start_delay = state.power_start_delay(trigger);
        let quiet_floor = if payload.explicit {
            None
        } else {
            state.quiet_floor(self.inner.control_plane.as_deref(), &target)
        };
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
            separates_next_word,
            scope: scope.clone(),
            explicit: payload.explicit,
            quiet_floor,
        };
        if escape_quiet || session.nothing_to_correct(payload.explicit) {
            self.abstain_admitted(state, binding, generation_timeout)
                .await;
            return Ok(None);
        }
        if let Some(carried) = carried {
            self.show_carried(state, binding, carried, generation_timeout)?;
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
            // Before `request` moves the context's text.
            vocabulary: context_on
                .map(|(identity, _)| (identity, context.window_text.clone().unwrap_or_default())),
            allow_replacement: session.allows_text_replacement(),
            binding,
            request: ProviderRequest {
                before: context.before,
                after: provider_after(context.after, separates_next_word),
                language: context.language,
                vocabulary: None,
            },
            trigger,
            cancellation,
            provider_permit,
            start_delay,
        }))
    }

    /// Shows a type-through remainder at once: it needs no model call.
    fn show_carried(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        binding: GenerationBinding,
        (text, expires_at, aggregate_day): (String, Instant, Option<u64>),
        generation_timeout: Duration,
    ) -> Result<(), BrokerError> {
        let session = state.session_with_data_access(binding.coordinates.session_id)?;
        session.generation = binding.generation;
        self.renew_context_authority_lease(session, generation_timeout);
        record_carry(&self.inner.metrics, &binding, None);
        let proposal = WritingProposal {
            text,
            replace_before: None,
        };
        let carried = Carried {
            expires_at,
            aggregate_day,
        };
        let cancellation = CancellationToken::new();
        self.show_if_current(state, binding, &cancellation, proposal, Some(carried));
        Ok(())
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
            mut request,
            allow_replacement,
            trigger,
            binding,
            cancellation,
            provider_permit,
            start_delay,
            vocabulary,
        } = admitted;
        if let Some((identity, window_text)) = vocabulary {
            let mut words = crate::vocabulary::Vocabulary::from_texts(&window_text, &[] as &[&str]);
            words.learned = self.learned_words(&identity);
            request.vocabulary = Some(words);
        }
        // Admission occurs before spawning, so there can never be more
        // generation tasks than permits. The permit is released on every
        // return path, including cancellation and timeout.
        let _provider_permit = provider_permit;
        let shutdown = &self.inner.shutdown;
        let delay = start_delay;
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
        let provider = time::timeout_at(
            deadline.into(),
            self.inner.provider.propose_outcome(
                request,
                cancellation.clone(),
                allow_replacement,
                trigger,
            ),
        );
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                // The provider future is dropped here, and its counts with it.
                trace_model_done(&binding, started, "cancelled", None, None, true);
                return;
            }
            () = shutdown.cancelled() => {
                self.inner.metrics.record_no_suggestion(NoSuggestionReason::Stale);
                trace_model_done(&binding, started, "shutdown", None, None, true);
                return;
            }
            result = provider => result,
        };
        let (answer, stats) = ProviderAnswer::classify(result);
        let text = match &answer {
            ProviderAnswer::Proposal(raw) => Some(raw.text.as_str()),
            _ => None,
        };
        let cancelled_before_first_token =
            matches!(answer, ProviderAnswer::Cancelled | ProviderAnswer::TimedOut)
                && stats.is_none_or(|stats| stats.ttft.is_none());
        trace_model_done(
            &binding,
            started,
            &answer.trace_name(),
            text,
            stats.as_ref(),
            cancelled_before_first_token,
        );
        self.finish_generation(binding, &cancellation, answer, stats)
            .await;
    }

    /// Every path records exactly one no-suggestion class or one shown result.
    async fn finish_generation(
        &self,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        answer: ProviderAnswer,
        provider_stats: Option<ProviderStats>,
    ) {
        let proposal =
            match self.check_provider_answer(&binding, cancellation, answer, provider_stats) {
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
        self.show_if_current(state, binding, cancellation, proposal, None);
    }

    fn check_provider_answer(
        &self,
        binding: &GenerationBinding,
        cancellation: &CancellationToken,
        answer: ProviderAnswer,
        provider_stats: Option<ProviderStats>,
    ) -> Result<WritingProposal, Withheld> {
        let metrics = &self.inner.metrics;
        if Instant::now() >= binding.deadline {
            self.record_generation_timeout(cancellation);
            return Err(Withheld::Clear(ReasonCode::ProviderTimeout));
        }
        let raw = match answer {
            ProviderAnswer::Proposal(raw) => raw,
            ProviderAnswer::NoSuggestion(reason) => {
                metrics.record_no_suggestion(reason);
                return Err(Withheld::Clear(ReasonCode::NoSuggestion));
            }
            ProviderAnswer::Cancelled => {
                metrics.record_no_suggestion(NoSuggestionReason::Stale);
                return Err(Withheld::Silently);
            }
            ProviderAnswer::Unavailable => {
                metrics.record_provider_error();
                metrics.record_no_suggestion(NoSuggestionReason::ProviderError);
                return Err(Withheld::Clear(ReasonCode::ProviderError));
            }
            ProviderAnswer::TimedOut => {
                self.record_generation_timeout(cancellation);
                return Err(Withheld::Clear(ReasonCode::ProviderTimeout));
            }
        };
        metrics.record_provider_output(raw.text.len());
        // An app whose suggestions are seldom taken shows only surer ones.
        // A correction, or a proposal the model did not score, keeps its own.
        if let (Some(floor), Some(minimum)) = (
            binding.quiet_floor,
            provider_stats.and_then(|stats| stats.min_logprob_hundredths),
        ) {
            if raw.replace_before.is_none() && f64::from(minimum) / 100.0 < floor {
                metrics.record_no_suggestion(NoSuggestionReason::LowConfidence);
                debug_trace::emit("suggestion withheld", |entry| {
                    entry.put("sid", debug_trace::session(binding.coordinates.session_id));
                    entry.put("reason", "quiet_app");
                    entry.put("floor", floor);
                });
                return Err(Withheld::Clear(ReasonCode::NoSuggestion));
            }
        }
        let checked = if binding.separates_next_word && raw.replace_before.is_none() {
            separated_proposal(self.inner.provider_kind, binding, raw)
        } else {
            checked_proposal(
                self.inner.provider_kind,
                &binding.before,
                &binding.after,
                raw,
            )
        };
        checked.ok_or_else(|| {
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

    /// Shows `proposal` if the session still awaits exactly this generation
    /// and still lets it display; `carried` is the type-through remainder's
    /// original expiry and aggregate.
    #[allow(clippy::too_many_lines)]
    fn show_if_current(
        &self,
        mut state: MutexGuard<'_, BrokerState>,
        binding: GenerationBinding,
        cancellation: &CancellationToken,
        proposal: WritingProposal,
        carried: Option<Carried>,
    ) {
        let metrics = &self.inner.metrics;
        let settings_revision = state.settings_revision;
        // A remainder the writer is typing through, a correction or an asked
        // suggestion is never a word they turned down.
        let turned_down = carried.is_none()
            && proposal.replace_before.is_none()
            && !binding.explicit
            && state
                .rejections
                .withholds(&binding.scope, &binding.before, &proposal.text);
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
            before,
            after,
            language,
            ..
        } = binding;
        if turned_down {
            metrics.record_no_suggestion(NoSuggestionReason::Repeated);
            debug_trace::emit("suggestion withheld", |entry| {
                entry.put("sid", debug_trace::session(coordinates.session_id));
                entry.put("reason", "repeated");
            });
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                fingerprint,
                ReasonCode::NoSuggestion,
                request_id,
            ));
            return;
        }
        if proposal.replace_before.is_none() && session.spelling_only() {
            metrics.record_no_suggestion(NoSuggestionReason::ModelAbstained);
            let _ = session.sink.send(BrokerEvent::generation_withdrawn(
                coordinates,
                fingerprint,
                ReasonCode::NoSuggestion,
                request_id,
            ));
            return;
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
            return;
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
        // A carried remainder is the same suggestion, not another show.
        if carried.is_none() {
            metrics.record_suggestion_shown();
            self.queue_stats(&session.target.target, settings_revision, StatsEvent::Shown);
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
            turned_down: false,
        });
        session.visible = Some(VisibleSuggestion {
            payload,
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
            if let Some(minimum) = stats.min_logprob_hundredths {
                entry.put("min_logprob", f64::from(minimum) / 100.0);
            }
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

/// The carried remainder for this admitted request (ADR 0004), or the gate
/// that declined the carry. Both are `None` when no carry exists.
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

/// The text after the caret as the provider is asked about it: after letters
/// typed at a word start, as if a space already followed them;
/// [`separated_proposal`] ends the suggestion with that space.
fn provider_after(after: String, separates_next_word: bool) -> String {
    if separates_next_word {
        format!(" {after}")
    } else {
        after
    }
}

/// A suggestion for letters typed at a word start in front of a word
/// (`the fin|report`): checked as the provider was asked, with a space after
/// those letters, and then ended with that space so it stays apart from the
/// next word (`al ` makes `the final report`).
fn separated_proposal(
    provider_kind: ProviderKind,
    binding: &GenerationBinding,
    raw: WritingProposal,
) -> Option<WritingProposal> {
    let spaced = format!(" {}", binding.after);
    let checked = checked_proposal(provider_kind, &binding.before, &spaced, raw)?;
    let text = format!("{} ", checked.text.trim_end());
    crate::segment::validate_completion_shape(&binding.before, &binding.after, &text, true)
        .is_ok()
        .then_some(WritingProposal {
            text,
            replace_before: None,
        })
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

/// The subject and settings revision when context is on for `target`.
fn context_on(
    settings: Option<&SettingsV2>,
    target: &TargetDescriptor,
) -> Option<(StableIdentity, u64)> {
    let settings = settings?;
    let identity = StableIdentity::from_target(target).ok()?;
    settings
        .context_words_for(&identity)
        .then_some((identity, settings.revision))
}

/// Field visits whose counted words are remembered.
const MAX_LEARNED_VISITS: usize = 16;

impl BrokerState {
    /// The finished words of `context` not yet counted in this field visit,
    /// marked counted now. Prose fields only: a terminal's typed line may
    /// hold a passphrase.
    fn fresh_written_words(
        &mut self,
        target: &TargetDescriptor,
        context: &ContextChangedPayload,
    ) -> Vec<String> {
        if context.field.purpose != FieldPurpose::Normal || target.kind == TargetKind::Terminal {
            return Vec::new();
        }
        let key = serde_json::to_string(target).unwrap_or_default();
        let found = self
            .learned_visits
            .iter()
            .position(|(visit, _)| *visit == key);
        let index = if let Some(index) = found {
            index
        } else {
            if self.learned_visits.len() >= MAX_LEARNED_VISITS {
                self.learned_visits.pop_front();
            }
            self.learned_visits
                .push_back((key, std::collections::BTreeSet::new()));
            self.learned_visits.len() - 1
        };
        let counted = &mut self.learned_visits[index].1;
        crate::vocabulary::written_words(&context.before)
            .into_iter()
            .filter(|word| counted.insert(word.clone()))
            .collect()
    }
}
