//! Explicit revision of one selection (ADR 0012): shown under the same
//! policy as a suggestion, applied once over exactly that selection, and
//! retired by Escape, an edit, a hold or expiry.

use std::sync::{Arc, Mutex};

use super::*;
use crate::protocol::{RevisionRequestPayload, SuggestCancelPayload};
use crate::provider::DeterministicPhraseProvider;
use crate::revision::RevisionMode;
use crate::revision::service::{
    FixtureReviser, Reviser, RevisionCall, RevisionError, RevisionReply,
};

/// Answers every revision with the same text.
struct ScriptedReviser(&'static str);

#[async_trait]
impl Reviser for ScriptedReviser {
    fn provider(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    async fn revise(
        &self,
        _call: RevisionCall<'_>,
        _cancellation: CancellationToken,
    ) -> Result<RevisionReply, RevisionError> {
        Ok(RevisionReply {
            output: self.0.to_owned(),
            ..RevisionReply::default()
        })
    }
}

/// Answers only once cancelled, and keeps the cancellation it was given.
#[derive(Default)]
struct PendingReviser(Mutex<Option<CancellationToken>>);

#[async_trait]
impl Reviser for PendingReviser {
    fn provider(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    async fn revise(
        &self,
        _call: RevisionCall<'_>,
        cancellation: CancellationToken,
    ) -> Result<RevisionReply, RevisionError> {
        *self.0.lock().expect("token") = Some(cancellation.clone());
        cancellation.cancelled().await;
        Err(RevisionError::Cancelled)
    }
}

fn revision_capabilities() -> Vec<Capability> {
    vec![
        Capability::Context,
        Capability::Suggestion,
        Capability::CommitDispatchedUnverified,
        Capability::SelectionRevision,
    ]
}

/// A protocol v2 Fcitx session that negotiated revisions.
async fn revising(
    reviser: Option<Arc<dyn Reviser>>,
) -> (Broker, SessionId, mpsc::Receiver<BrokerEvent>) {
    let broker = Broker::new(
        Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    if let Some(reviser) = reviser {
        broker.install_reviser(reviser);
    }
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(32);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: TargetDescriptor {
                    kind: TargetKind::DesktopApplication,
                    app_id: "org.gnome.texteditor".to_owned(),
                    target_id: "ic:42".to_owned(),
                    origin: None,
                },
                activation: Activation::Always,
            },
            SessionAuthority {
                adapter_kind: AdapterKind::Fcitx,
                capabilities: revision_capabilities(),
            },
            event_sink(sink),
        )
        .await
        .expect("open session");
    (broker, session_id, events)
}

fn request(revision: u64, before: &str, selected: &str, after: &str) -> RevisionRequestPayload {
    let anchor = u64::try_from(before.chars().count()).expect("small fixture");
    let head = anchor + u64::try_from(selected.chars().count()).expect("small fixture");
    let field = context(revision, FieldPurpose::Normal).field;
    RevisionRequestPayload {
        fingerprint: format!("revision_fingerprint_{revision:08}"),
        mode: RevisionMode::Grammar,
        before: before.to_owned(),
        selected: selected.to_owned(),
        after: after.to_owned(),
        selection: Selection {
            anchor,
            head,
            unit: OffsetUnit::UnicodeScalarValues,
        },
        field,
        activation: Activation::Always,
        language: Some("en".to_owned()),
    }
}

async fn next(events: &mut mpsc::Receiver<BrokerEvent>) -> BrokerEvent {
    timeout(Duration::from_millis(500), events.recv())
        .await
        .expect("event deadline")
        .expect("event")
}

fn control(action: ControlAction, fingerprint: &str, id: &str) -> SessionControlRequestPayload {
    SessionControlRequestPayload {
        action,
        fingerprint: fingerprint.to_owned(),
        suggestion_id: Some(id.to_owned()),
    }
}

#[tokio::test]
async fn a_revision_is_shown_then_applied_once_over_exactly_its_selection() {
    let (broker, id, mut events) = revising(Some(Arc::new(FixtureReviser))).await;
    let payload = request(2, "Yesterday ", "i has went to the shop", " with Anna.");
    broker
        .request_revision(
            coordinates(id, 2),
            payload.clone(),
            Some("revise".to_owned()),
        )
        .await
        .expect("admitted");
    let BrokerEvent::RevisionShow {
        payload: shown,
        request_id,
        ..
    } = next(&mut events).await
    else {
        panic!("expected a revision")
    };
    assert_eq!(request_id.as_deref(), Some("revise"));
    assert_eq!(shown.original, "i has went to the shop");
    assert_eq!(
        shown.text, "i went to the shop",
        "mid-sentence case is kept"
    );
    assert_eq!(shown.mode, RevisionMode::Grammar);
    assert_eq!(shown.provider, ProviderKind::PhraseV1);
    shown.validate().expect("a valid revision payload");

    // Accepting one word of a revision means nothing.
    assert!(
        broker
            .session_control(
                coordinates(id, 2),
                control(
                    ControlAction::AcceptWord,
                    &payload.fingerprint,
                    &shown.revision_id
                ),
                None,
            )
            .await
            .is_err()
    );
    broker
        .session_control(
            coordinates(id, 2),
            control(
                ControlAction::AcceptAll,
                &payload.fingerprint,
                &shown.revision_id,
            ),
            Some("apply".to_owned()),
        )
        .await
        .expect("accepted");
    let BrokerEvent::CommitPrepare { payload: grant, .. } = next(&mut events).await else {
        panic!("expected a grant")
    };
    assert_eq!(grant.suggestion_id, shown.revision_id);
    assert_eq!(grant.text, "i went to the shop");
    assert_eq!(grant.replace_selection, Some(payload.selection));
    assert_eq!(grant.replace_before, None);
    // The grant is one-shot: the revision is gone.
    assert!(
        broker
            .session_control(
                coordinates(id, 2),
                control(
                    ControlAction::AcceptAll,
                    &payload.fingerprint,
                    &shown.revision_id
                ),
                None,
            )
            .await
            .is_err()
    );
    broker
        .commit_result(
            coordinates(id, 2),
            CommitResultPayload {
                fingerprint: payload.fingerprint,
                suggestion_id: shown.revision_id,
                status: CommitStatus::DispatchedUnverified,
            },
        )
        .await
        .expect("result");
    let savings = broker.metrics().snapshot().word_savings;
    assert!(
        savings.is_none_or(|savings| savings.dispatched_unverified == 0),
        "a revision saves no words"
    );
}

#[tokio::test]
async fn escape_retires_a_shown_revision() {
    let (broker, id, mut events) = revising(Some(Arc::new(FixtureReviser))).await;
    let payload = request(2, "", "I has went home.", "");
    broker
        .request_revision(coordinates(id, 2), payload.clone(), None)
        .await
        .expect("admitted");
    let BrokerEvent::RevisionShow { payload: shown, .. } = next(&mut events).await else {
        panic!("expected a revision")
    };
    broker
        .session_control(
            coordinates(id, 2),
            control(
                ControlAction::Dismiss,
                &payload.fingerprint,
                &shown.revision_id,
            ),
            None,
        )
        .await
        .expect("dismissed");
    let BrokerEvent::RevisionClear {
        payload: cleared, ..
    } = next(&mut events).await
    else {
        panic!("expected a clear")
    };
    assert_eq!(cleared.reason, ReasonCode::Dismissed);
    assert_eq!(
        cleared.revision_id.as_deref(),
        Some(shown.revision_id.as_str())
    );
    assert!(
        broker
            .session_control(
                coordinates(id, 2),
                control(
                    ControlAction::AcceptAll,
                    &payload.fingerprint,
                    &shown.revision_id
                ),
                None,
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_edit_or_a_cancel_ends_a_running_revision() {
    for cancel in [false, true] {
        let reviser = Arc::new(PendingReviser::default());
        let (broker, id, mut events) = revising(Some(reviser.clone())).await;
        let payload = request(2, "", "I has went home.", "");
        broker
            .request_revision(coordinates(id, 2), payload.clone(), None)
            .await
            .expect("admitted");
        let token = timeout(Duration::from_millis(200), async {
            loop {
                if let Some(token) = reviser.0.lock().expect("token").clone() {
                    break token;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("the model was called");
        let reason = if cancel {
            broker
                .cancel_suggestion(
                    coordinates(id, 2),
                    SuggestCancelPayload {
                        fingerprint: payload.fingerprint.clone(),
                        reason: ReasonCode::Cancelled,
                    },
                )
                .await
                .expect("cancelled");
            ReasonCode::Cancelled
        } else {
            broker
                .update_context(coordinates(id, 3), context(3, FieldPurpose::Normal))
                .await
                .expect("an edit");
            ReasonCode::Superseded
        };
        let BrokerEvent::RevisionClear {
            payload: cleared, ..
        } = next(&mut events).await
        else {
            panic!("expected a clear")
        };
        assert_eq!(cleared.reason, reason);
        assert!(token.is_cancelled(), "the model call is cancelled");
        assert!(
            timeout(Duration::from_millis(50), events.recv())
                .await
                .is_err(),
            "nothing is shown afterwards"
        );
    }
}

#[tokio::test]
async fn answers_that_change_nothing_or_too_much_are_not_shown() {
    for (output, reason) in [
        ("Thanks for the reply.", ReasonCode::NoChange),
        (
            "Danke für die Antwort, sie war sehr hilfreich.",
            ReasonCode::InvalidOutput,
        ),
        (
            "Thanks for the reply.\nLet me know.",
            ReasonCode::InvalidOutput,
        ),
    ] {
        let (broker, id, mut events) = revising(Some(Arc::new(ScriptedReviser(output)))).await;
        broker
            .request_revision(
                coordinates(id, 2),
                request(2, "", "Thanks for the reply.", ""),
                None,
            )
            .await
            .expect("admitted");
        let BrokerEvent::RevisionClear {
            payload: cleared, ..
        } = next(&mut events).await
        else {
            panic!("expected a clear for {output:?}")
        };
        assert_eq!(cleared.reason, reason, "{output:?}");
    }
}

#[tokio::test]
async fn revisions_are_refused_where_suggestions_are_and_in_terminals() {
    let (broker, id, mut events) = revising(None).await;
    broker
        .request_revision(coordinates(id, 2), request(2, "", "I has went.", ""), None)
        .await
        .expect("admitted");
    let BrokerEvent::RevisionClear {
        payload: cleared, ..
    } = next(&mut events).await
    else {
        panic!("expected a clear")
    };
    assert_eq!(cleared.reason, ReasonCode::RevisionUnavailable);

    let (broker, id, mut events) = revising(Some(Arc::new(FixtureReviser))).await;
    let mut persian = request(2, "", "سلام دوست من، حالت چطور است؟", "");
    persian.language = Some("fa".to_owned());
    broker
        .request_revision(coordinates(id, 2), persian, None)
        .await
        .expect("admitted");
    let BrokerEvent::RevisionClear {
        payload: cleared, ..
    } = next(&mut events).await
    else {
        panic!("expected a clear")
    };
    assert_eq!(cleared.reason, ReasonCode::UnsupportedLanguage);

    let mut sensitive = request(3, "", "hunter2 is mine", "");
    sensitive.field.sensitive = true;
    let mut terminal = request(4, "", "ls -la", "");
    terminal.field.purpose = FieldPurpose::Terminal;
    let mut composing = request(5, "", "I has went.", "");
    composing.field.composing = true;
    // A refusal is answered, never an error: an adapter treats an error as
    // a broken link.
    for (revision, payload, reason) in [
        (3, sensitive, ReasonCode::FieldSensitive),
        (4, terminal, ReasonCode::FieldNotEditable),
        (5, composing, ReasonCode::FieldAmbiguous),
    ] {
        broker
            .request_revision(coordinates(id, revision), payload, None)
            .await
            .expect("answered");
        let BrokerEvent::RevisionClear {
            payload: cleared, ..
        } = next(&mut events).await
        else {
            panic!("expected a refusal")
        };
        assert_eq!(cleared.reason, reason);
    }
    broker.set_paused(true).await;
    broker
        .request_revision(coordinates(id, 6), request(6, "", "I has went.", ""), None)
        .await
        .expect("answered");
    let refusal = timeout(Duration::from_millis(500), async {
        loop {
            if let BrokerEvent::RevisionClear { payload, .. } = next(&mut events).await {
                break payload.reason;
            }
        }
    })
    .await
    .expect("a refusal");
    assert!(
        matches!(refusal, ReasonCode::Paused | ReasonCode::PolicyNever),
        "{refusal:?}"
    );

    // Without the negotiated capability there is no revision at all.
    let (broker, id, _events) = setup(
        Arc::new(DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    )
    .await;
    broker.install_reviser(Arc::new(FixtureReviser));
    assert!(matches!(
        broker
            .request_revision(coordinates(id, 2), request(2, "", "I has went.", ""), None)
            .await,
        Err(BrokerError::InvalidCapability)
    ));
}

#[tokio::test]
async fn a_request_must_span_exactly_its_one_line_selection() {
    let (broker, id, _events) = revising(Some(Arc::new(FixtureReviser))).await;
    let mut short = request(2, "", "I has went.", "");
    short.selection.head -= 1;
    let mut two_lines = request(3, "", "I has\nwent.", "");
    two_lines.selection.head = two_lines.selection.anchor + 11;
    let mut empty = request(4, "", " ", "");
    empty.selection.head = empty.selection.anchor + 1;
    for (revision, payload) in [(2, short), (3, two_lines), (4, empty)] {
        assert!(
            broker
                .request_revision(coordinates(id, revision), payload, None)
                .await
                .is_err(),
            "revision {revision}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn a_shown_revision_expires() {
    let (broker, id, mut events) = revising(Some(Arc::new(FixtureReviser))).await;
    broker
        .request_revision(
            coordinates(id, 2),
            request(2, "", "I has went home.", ""),
            None,
        )
        .await
        .expect("admitted");
    assert!(matches!(
        next(&mut events).await,
        BrokerEvent::RevisionShow { .. }
    ));
    tokio::time::advance(crate::engine::REVISION_TTL + Duration::from_secs(2)).await;
    let BrokerEvent::RevisionClear {
        payload: cleared, ..
    } = next(&mut events).await
    else {
        panic!("expected expiry")
    };
    assert_eq!(cleared.reason, ReasonCode::Expired);
}
