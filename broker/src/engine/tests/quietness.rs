//! Per-app quietness: an app whose suggestions are seldom taken shows only
//! surer ones; an explicit request is never quieted.

use std::sync::atomic::AtomicI32;

use super::*;
use crate::provider::{
    ProviderOutcome, ProviderReply, ProviderStats, RequestTrigger, WritingProposal,
};
use crate::stats::StatsEvent;

/// Proposes ` revision` with a settable lowest token log-probability.
struct ScoredProvider {
    min_logprob_hundredths: AtomicI32,
}

#[async_trait]
impl CompletionProvider for ScoredProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        Ok(Some(" revision".to_owned()))
    }

    async fn propose_outcome(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
        _allow_replacement: bool,
        _trigger: RequestTrigger,
    ) -> Result<ProviderReply, ProviderError> {
        Ok(ProviderReply {
            outcome: ProviderOutcome::Proposal(WritingProposal {
                text: " revision".to_owned(),
                replace_before: None,
            }),
            stats: Some(ProviderStats {
                ttft: None,
                prompt_tokens: None,
                cache_tokens: None,
                generated_tokens: 1,
                min_logprob_hundredths: Some(self.min_logprob_hundredths.load(Ordering::Relaxed)),
            }),
        })
    }
}

fn controlled_identity() -> StableIdentity {
    StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Http,
        "localhost",
        Some(4173),
    )
    .expect("controlled identity")
}

async fn shown(broker: &Broker, explicit: bool) -> bool {
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    broker
        .open_session(
            coordinates(session_id, 0),
            SessionOpenPayload {
                target: controlled_target(),
                activation: Activation::Always,
            },
            controlled_authority(),
            event_sink(sink),
        )
        .await
        .expect("allowed session");
    let update = context(1, FieldPurpose::Normal);
    broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await
        .expect("context");
    broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit,
            },
            None,
        )
        .await
        .expect("request");
    loop {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { .. })) => break true,
            Ok(Some(BrokerEvent::SuggestionClear { .. }) | None) | Err(_) => break false,
            Ok(Some(_)) => {}
        }
    }
}

#[tokio::test]
async fn an_app_whose_suggestions_are_seldom_taken_shows_only_surer_ones() {
    let root = tempdir().expect("temporary root");
    let paths = StoragePaths::new(
        root.path().join("config/badi"),
        root.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow the controlled site");
    let revision = control_plane
        .snapshot()
        .expect("snapshot")
        .settings
        .revision;
    let identity = controlled_identity();
    for index in 0..60 {
        let event = if index < 4 {
            StatsEvent::Accepted { words: 1 }
        } else {
            StatsEvent::Shown
        };
        control_plane
            .record_stats_at_settings_revision(revision, &identity, event)
            .expect("count");
    }
    let provider = std::sync::Arc::new(ScoredProvider {
        min_logprob_hundredths: AtomicI32::new(-250),
    });
    let broker =
        Broker::with_control_plane(provider.clone(), BrokerConfig::default(), control_plane)
            .expect("controlled broker");

    // 4 of 56 shown were taken: a floor of -2.25 withholds a -2.50 word.
    assert!(!shown(&broker, false).await);
    let breakdown = broker
        .metrics()
        .snapshot()
        .no_suggestion
        .expect("breakdown");
    assert_eq!(breakdown.low_confidence, 1);
    assert!(
        shown(&broker, true).await,
        "an explicit request is never quieted"
    );
    provider
        .min_logprob_hundredths
        .store(-210, Ordering::Relaxed);
    assert!(
        shown(&broker, false).await,
        "a surer suggestion still shows"
    );
}

#[tokio::test]
async fn without_counts_an_app_keeps_the_default_floor() {
    let provider = std::sync::Arc::new(ScoredProvider {
        min_logprob_hundredths: AtomicI32::new(-270),
    });
    let broker = Broker::new(provider, BrokerConfig::default());
    let (session, mut events) = open_observed_session(&broker, "field-epoch-1").await;
    assert!(
        shown_after(&broker, session, &mut events, "Hello")
            .await
            .is_some()
    );
}
