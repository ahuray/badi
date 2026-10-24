//! Revision of one selection on an explicit request (ADR 0012): admission
//! under the same policy, pause and holds as a suggestion, one model call at
//! a time, the gate of [`crate::revision::accept`], and the one-shot grant
//! that replaces exactly the revised selection.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::{Broker, BrokerError, BrokerEvent, SessionState, duration_millis};
use crate::debug_trace;
use crate::policy::{PolicyDecision, PolicyInput, PolicyReason, evaluate};
use crate::protocol::{
    Acceptance, Capability, CommitPreparePayload, Coordinates, FieldPurpose, ReasonCode,
    RevisionClearPayload, RevisionRequestPayload, RevisionShowPayload, Selection, TargetKind,
};
use crate::revision::service::{Reviser, RevisionCall, RevisionError, RevisionReply};
use crate::revision::{RevisionAbstain, RevisionLanguage, RevisionMode, accept};

/// The bound on one revision, a cold start of the revision runtime included.
pub const REVISION_TIMEOUT: Duration = Duration::from_secs(40);
/// How long a shown revision may be applied.
pub const REVISION_TTL: Duration = Duration::from_millis(crate::revision::REVISION_TTL_MS);

/// A session's revision, from its request until it is applied or retired.
pub(super) struct SessionRevision {
    revision_id: String,
    fingerprint: String,
    selection: Selection,
    request_id: Option<String>,
    cancellation: CancellationToken,
    /// The replacement once shown, and until when it may be applied.
    shown: Option<(String, Instant)>,
}

impl SessionRevision {
    pub(super) fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Cancels the model call and tells the adapter the revision ended.
    pub(super) fn retire(self, coordinates: Coordinates, reason: ReasonCode) -> BrokerEvent {
        self.cancellation.cancel();
        clear(
            coordinates,
            self.fingerprint,
            Some(self.revision_id),
            reason,
            self.request_id,
        )
    }
}

fn clear(
    coordinates: Coordinates,
    fingerprint: String,
    revision_id: Option<String>,
    reason: ReasonCode,
    request_id: Option<String>,
) -> BrokerEvent {
    BrokerEvent::RevisionClear {
        coordinates,
        payload: RevisionClearPayload {
            fingerprint,
            revision_id,
            reason,
        },
        request_id,
    }
}

/// What a running revision must still match before it is shown.
struct AdmittedRevision {
    coordinates: Coordinates,
    revision_id: String,
    fingerprint: String,
    mode: RevisionMode,
    language: &'static RevisionLanguage,
    before: String,
    selected: String,
    after: String,
    request_id: Option<String>,
    cancellation: CancellationToken,
    reviser: Arc<dyn Reviser>,
}

impl Broker {
    /// Adds the model behind revisions; without one, a request is answered
    /// with `revision_unavailable`. Set once, before serving.
    pub fn install_reviser(&self, reviser: Arc<dyn Reviser>) {
        let _ = self.inner.reviser.set(reviser);
    }

    /// Admits an explicit revision of the selection in `payload`, which is a
    /// context snapshot of its own, and starts its model call.
    pub async fn request_revision(
        &self,
        coordinates: Coordinates,
        mut payload: RevisionRequestPayload,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        crate::protocol::validate_coordinate_bounds(coordinates.focus_epoch, coordinates.revision)?;
        if request_id
            .as_deref()
            .is_some_and(|request_id| !crate::protocol::valid_opaque_id(request_id))
        {
            return Err(BrokerError::InvalidPayload);
        }
        let metrics = &self.inner.metrics;
        let mut state = self.inner.state.lock().await;
        let paused = state.effective_paused();
        let (language_override, preferred) = state.revision_language_hints(coordinates.session_id);
        // A hold, snooze or setting that withholds this target refuses the
        // request in its answer: an adapter treats an error as a broken link.
        let withheld = state.session_with_data_access(coordinates.session_id).err();
        let session = state
            .sessions
            .get_mut(&coordinates.session_id)
            .ok_or(BrokerError::UnknownSession)?;
        if !session.authority.grants(Capability::SelectionRevision) {
            return Err(BrokerError::InvalidCapability);
        }
        session.ensure_newer_context(coordinates)?;
        payload.activation =
            super::session::restrictive_activation(session.target.activation, payload.activation);
        payload.validate()?;
        // A request is a new snapshot of the field: whatever was derived from
        // the earlier one ends first, as with a new context.
        session.revoke_context_authority(metrics, ReasonCode::Superseded);
        session.coordinates = coordinates;
        session.context_seen_at_coordinates = true;
        let refused = match withheld {
            Some(BrokerError::Denied(reason)) => Some(reason),
            Some(error) => return Err(error),
            None => revision_policy(&payload, session.target.target.kind, paused).err(),
        };
        let language = language_override
            .or_else(|| detect_selection_language(&payload))
            .or(payload.language.as_deref())
            .or(preferred)
            .and_then(crate::revision::language);
        let reviser = self.inner.reviser.get().cloned();
        let (language, reviser) = match (refused, language, reviser) {
            (None, Some(language), Some(reviser)) => (language, reviser),
            (refused, language, _) => {
                let reason = refused.map_or(
                    if language.is_none() {
                        ReasonCode::UnsupportedLanguage
                    } else {
                        ReasonCode::RevisionUnavailable
                    },
                    PolicyReason::refusal_code,
                );
                trace_outcome(coordinates, payload.mode, None, reason_name(reason), None);
                let _ = session.sink.send(clear(
                    coordinates,
                    payload.fingerprint,
                    None,
                    reason,
                    request_id,
                ));
                return Ok(());
            }
        };
        let cancellation = CancellationToken::new();
        let revision_id = format!("r:{}", uuid::Uuid::new_v4());
        session.revision = Some(SessionRevision {
            revision_id: revision_id.clone(),
            fingerprint: payload.fingerprint.clone(),
            selection: payload.selection,
            request_id: request_id.clone(),
            cancellation: cancellation.clone(),
            shown: None,
        });
        // The snapshot's authority lasts through the call and the reading
        // window; an edit, a focus change or a hold still ends it at once.
        self.arm_context_authority_lease(session, REVISION_TIMEOUT + REVISION_TTL);
        debug_trace::emit("revision start", |entry| {
            entry.put("sid", debug_trace::session(coordinates.session_id));
            entry.put("mode", payload.mode.name());
            entry.put("language", language.tag);
            entry.put("chars", payload.selected.chars().count());
        });
        let admitted = AdmittedRevision {
            coordinates,
            revision_id,
            fingerprint: payload.fingerprint,
            mode: payload.mode,
            language,
            before: payload.before,
            selected: payload.selected,
            after: payload.after,
            request_id,
            cancellation,
            reviser,
        };
        drop(state);
        tokio::spawn(self.clone().run_revision(admitted));
        Ok(())
    }

    async fn run_revision(self, admitted: AdmittedRevision) {
        let started = Instant::now();
        // One revision runs at a time across the broker: the runtime has one
        // slot, and a second request would only wait inside it.
        let permit = tokio::select! {
            biased;
            () = admitted.cancellation.cancelled() => return,
            permit = Arc::clone(&self.inner.revision_admissions).acquire_owned() => permit,
        };
        let Ok(permit) = permit else {
            return;
        };
        let call = RevisionCall {
            mode: admitted.mode,
            language: admitted.language,
            before: &admitted.before,
            selected: &admitted.selected,
            timeout: REVISION_TIMEOUT.saturating_sub(started.elapsed()),
        };
        let reply = admitted
            .reviser
            .revise(call, admitted.cancellation.clone())
            .await;
        drop(permit);
        let cold = reply.as_ref().is_ok_and(|reply| reply.cold);
        if reply == Err(RevisionError::Cancelled) {
            // Whatever cancelled it owns the display.
            return;
        }
        let outcome = gate(&admitted, reply);
        let mut state = self.inner.state.lock().await;
        // As for a suggestion: nothing is shown while paused or withheld (the
        // pause or hold already retired this revision).
        if state.effective_paused() {
            return;
        }
        let Ok(session) = state.session_with_data_access(admitted.coordinates.session_id) else {
            return;
        };
        let current = session.coordinates == admitted.coordinates
            && session.revision.as_ref().is_some_and(|revision| {
                revision.revision_id == admitted.revision_id && revision.shown.is_none()
            });
        if !current {
            return;
        }
        trace_outcome(
            admitted.coordinates,
            admitted.mode,
            Some((started, cold)),
            outcome.as_ref().map_or_else(|(_, name)| *name, |_| "shown"),
            outcome.as_ref().ok().map(String::as_str),
        );
        match outcome {
            Ok(text) => {
                let expires_at = Instant::now() + REVISION_TTL;
                if let Some(revision) = session.revision.as_mut() {
                    revision.shown = Some((text.clone(), expires_at));
                }
                let show = BrokerEvent::RevisionShow {
                    coordinates: admitted.coordinates,
                    payload: RevisionShowPayload {
                        fingerprint: admitted.fingerprint,
                        revision_id: admitted.revision_id,
                        mode: admitted.mode,
                        original: admitted.selected,
                        text,
                        ttl_ms: crate::revision::REVISION_TTL_MS,
                        provider: admitted.reviser.provider(),
                    },
                    request_id: admitted.request_id,
                };
                self.arm_context_authority_lease(
                    session,
                    REVISION_TTL + self.inner.config.commit_result_lease,
                );
                if session.sink.send(show).is_err() {
                    session.revision = None;
                }
            }
            Err((reason, _)) => {
                if let Some(revision) = session.revision.take() {
                    let _ = session
                        .sink
                        .send(revision.retire(admitted.coordinates, reason));
                }
            }
        }
    }
}

/// The text to show, or why nothing is shown and its trace name.
fn gate(
    admitted: &AdmittedRevision,
    reply: Result<RevisionReply, RevisionError>,
) -> Result<String, (ReasonCode, &'static str)> {
    match reply {
        Err(RevisionError::Timeout) => Err((ReasonCode::ProviderTimeout, "timeout")),
        Err(RevisionError::Unavailable) => Err((ReasonCode::RevisionUnavailable, "unavailable")),
        Err(RevisionError::Failed | RevisionError::Cancelled) => {
            Err((ReasonCode::ProviderError, "failed"))
        }
        Ok(reply) if reply.truncated => Err((ReasonCode::InvalidOutput, "truncated")),
        Ok(reply) => accept(
            admitted.mode,
            admitted.language,
            &admitted.before,
            &admitted.selected,
            &admitted.after,
            &reply.output,
        )
        .map_err(|abstain| {
            let reason = if abstain == RevisionAbstain::Unchanged {
                ReasonCode::NoChange
            } else {
                ReasonCode::InvalidOutput
            };
            (reason, abstain.name())
        }),
    }
}

impl super::BrokerState {
    /// The app's or site's language override, and the first writing language.
    fn revision_language_hints(
        &self,
        session_id: crate::protocol::SessionId,
    ) -> (Option<&'static str>, Option<&'static str>) {
        let settings = self.settings.as_ref();
        let language_override = self.sessions.get(&session_id).and_then(|session| {
            settings?
                .language_override(&session.target.target)
                .map(crate::settings::PredictionLanguage::tag)
        });
        let preferred = settings
            .and_then(|settings| settings.writing_languages.first())
            .map(|language| language.tag());
        (language_override, preferred)
    }
}

/// Policy as for an explicit request at a caret (the request validated its
/// own selection), except that a terminal's line is never replaced.
fn revision_policy(
    payload: &RevisionRequestPayload,
    target_kind: TargetKind,
    paused: bool,
) -> Result<(), PolicyReason> {
    if target_kind == TargetKind::Terminal || payload.field.purpose == FieldPurpose::Terminal {
        return Err(PolicyReason::FieldNotEditable);
    }
    match evaluate(PolicyInput {
        activation: payload.activation,
        explicit: true,
        field: payload.field,
        target_kind,
        paused,
        selection_collapsed: true,
    }) {
        PolicyDecision::Allow(_) => Ok(()),
        PolicyDecision::ManualRequired(reason) | PolicyDecision::Deny(reason) => Err(reason),
    }
}

/// The selection's language from its own text and what precedes it.
fn detect_selection_language(payload: &RevisionRequestPayload) -> Option<&'static str> {
    crate::language::detect_field_language(&format!("{}{}", payload.before, payload.selected))
}

impl SessionState {
    /// Whether `suggestion_id` names this session's revision.
    pub(super) fn has_revision(&self, suggestion_id: Option<&str>) -> bool {
        self.revision
            .as_ref()
            .is_some_and(|revision| Some(revision.revision_id.as_str()) == suggestion_id)
    }

    /// Applies or dismisses the shown revision: `acceptance` `None` is
    /// Escape, `All` grants one commit over exactly the revised selection.
    pub(super) fn control_revision(
        &mut self,
        broker: &Broker,
        fingerprint: &str,
        acceptance: Option<Acceptance>,
        request_id: Option<String>,
    ) -> Result<(), BrokerError> {
        let coordinates = self.coordinates;
        let revision = self.revision.take().ok_or(BrokerError::NoSuggestion)?;
        if revision.fingerprint != fingerprint || acceptance == Some(Acceptance::Word) {
            self.revision = Some(revision);
            return Err(BrokerError::Stale);
        }
        let Some((text, expires_at)) = revision.shown.clone() else {
            self.revision = Some(revision);
            return Err(BrokerError::NoSuggestion);
        };
        if acceptance.is_none() {
            trace_control(coordinates, "dismissed");
            let _ = self
                .sink
                .send(revision.retire(coordinates, ReasonCode::Dismissed));
            return Ok(());
        }
        if expires_at <= Instant::now() {
            let _ = self
                .sink
                .send(revision.retire(coordinates, ReasonCode::Expired));
            return Err(BrokerError::NoSuggestion);
        }
        trace_control(coordinates, "accepted");
        let commit = CommitPreparePayload {
            fingerprint: revision.fingerprint,
            suggestion_id: revision.revision_id,
            text,
            replace_before: None,
            replace_selection: Some(revision.selection),
            acceptance: Acceptance::All,
        };
        broker.grant_commit(self, commit, request_id, 0, true)?;
        broker.arm_context_authority_lease(self, broker.inner.config.commit_result_lease);
        Ok(())
    }
}

const fn reason_name(reason: ReasonCode) -> &'static str {
    match reason {
        ReasonCode::UnsupportedLanguage => "unsupported_language",
        ReasonCode::RevisionUnavailable => "unavailable",
        _ => "refused",
    }
}

/// Content-free unless the run records text: the mode, how long the model
/// took, and what came of it.
fn trace_outcome(
    coordinates: Coordinates,
    mode: RevisionMode,
    timing: Option<(Instant, bool)>,
    outcome: &str,
    text: Option<&str>,
) {
    debug_trace::emit("revision done", |entry| {
        entry.put("sid", debug_trace::session(coordinates.session_id));
        entry.put("mode", mode.name());
        entry.put("outcome", outcome);
        if let Some((started, cold)) = timing {
            entry.put("ms", duration_millis(started.elapsed()));
            entry.put("cold", cold);
        }
        if let Some(text) = text {
            entry.put("chars", text.chars().count());
            entry.text("text", text);
        }
    });
}

fn trace_control(coordinates: Coordinates, action: &str) {
    debug_trace::emit("revision control", |entry| {
        entry.put("sid", debug_trace::session(coordinates.session_id));
        entry.put("action", action);
    });
}
