//! The private probe request: provider and display checks without a session.

use super::*;

#[tokio::test]
async fn probe_uses_the_provider_and_display_checks_without_session_or_counters() {
    let broker = Broker::new(
        std::sync::Arc::new(crate::provider::DeterministicPhraseProvider::default()),
        BrokerConfig::default(),
    );
    let suggested = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(suggested.outcome, ProbeOutcome::Suggested);
    assert_eq!(suggested.text.as_deref(), Some(" for your time"));
    assert_eq!(suggested.provider, ProviderKind::PhraseV1);
    assert!(suggested.latency_ms.is_some());
    suggested.validate().expect("valid probe result");
    for (before, language, reason) in [
        ("Thank you", "de", NoSuggestionReason::RequestAbstained),
        ("Unmatched", "en", NoSuggestionReason::ModelAbstained),
    ] {
        let result = broker
            .probe(probe_request(before, language, false))
            .await
            .expect("probe");
        assert_eq!(result.outcome, ProbeOutcome::NoSuggestion);
        assert_eq!(result.reason, Some(reason));
        assert_eq!(result.text, None);
    }
    assert!(matches!(
        broker
            .probe(probe_request(&"a".repeat(513), "en", false))
            .await,
        Err(BrokerError::Protocol(_))
    ));
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.provider_calls, 0);
    assert_eq!(metrics.suggestions_shown, 0);
    assert_eq!(metrics.no_suggestion.expect("breakdown").total(), 0);
    assert_eq!(broker.session_count().await, 0);
    assert!(broker.active_locator().await.is_none());

    broker.set_paused(true).await.expect("pause");
    let paused = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("paused probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(paused.text, None);
    paused.validate().expect("valid paused result");
}

#[tokio::test]
async fn probe_times_out_like_generation_and_cancels_provider_work() {
    let provider = std::sync::Arc::new(PendingProvider::new());
    let provider_view = std::sync::Arc::clone(&provider);
    let broker = Broker::new(
        provider,
        BrokerConfig {
            provider_timeout: Duration::from_millis(5),
            ..BrokerConfig::default()
        },
    );
    let result = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(result.outcome, ProbeOutcome::NoSuggestion);
    assert_eq!(result.reason, Some(NoSuggestionReason::Timeout));
    assert!(
        provider_view
            .cancellation()
            .expect("provider token")
            .is_cancelled()
    );
}

#[tokio::test]
async fn probe_replacement_requires_explicit_authority_and_a_bound_suffix() {
    let broker = Broker::new(
        std::sync::Arc::new(SpellingProvider("teh ", "the ")),
        BrokerConfig::default(),
    );
    let denied = broker
        .probe(probe_request("This is teh ", "en", false))
        .await
        .expect("probe");
    assert_eq!(denied.reason, Some(NoSuggestionReason::OutputRejected));
    let allowed = broker
        .probe(probe_request("This is teh ", "en", true))
        .await
        .expect("probe");
    assert_eq!(allowed.outcome, ProbeOutcome::Suggested);
    assert_eq!(allowed.text.as_deref(), Some("the "));
    assert_eq!(allowed.replace_before.as_deref(), Some("teh "));
    allowed.validate().expect("valid replacement result");
    let unbound = broker
        .probe(probe_request("This is ten ", "en", true))
        .await
        .expect("probe");
    assert_eq!(unbound.reason, Some(NoSuggestionReason::OutputRejected));
    assert_eq!(broker.metrics().snapshot().commits_prepared, 0);
}

/// Holds each completion until the test releases it.
struct GatedProvider {
    calls: AtomicU64,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

#[async_trait]
impl CompletionProvider for GatedProvider {
    fn kind(&self) -> ProviderKind {
        ProviderKind::PhraseV1
    }

    async fn complete(
        &self,
        _request: ProviderRequest,
        _cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.entered.notify_one();
        self.release.notified().await;
        Ok(Some(" for your time".to_owned()))
    }
}

#[tokio::test]
async fn probe_withholds_its_result_when_a_pause_arrives_during_inference() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("initial settings");
    let provider = std::sync::Arc::new(GatedProvider {
        calls: AtomicU64::new(0),
        entered: tokio::sync::Notify::new(),
        release: tokio::sync::Notify::new(),
    });
    let broker = Broker::with_control_plane(
        std::sync::Arc::clone(&provider) as std::sync::Arc<dyn CompletionProvider>,
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker");
    let start = || {
        let broker = broker.clone();
        tokio::spawn(async move { broker.probe(probe_request("Thank you", "en", false)).await })
    };

    let pending = start();
    provider.entered.notified().await;
    provider.release.notify_one();
    let shown = pending.await.expect("probe task").expect("probe");
    assert_eq!(shown.outcome, ProbeOutcome::Suggested);
    assert_eq!(shown.text.as_deref(), Some(" for your time"));

    // Runtime pause (`badictl pause`) acknowledged while the provider runs.
    let pending = start();
    provider.entered.notified().await;
    assert!(broker.set_paused(true).await.expect("pause"));
    provider.release.notify_one();
    let paused = pending.await.expect("probe task").expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!((paused.text.as_deref(), paused.reason), (None, None));
    paused.validate().expect("valid paused result");
    assert!(!broker.set_paused(false).await.expect("resume"));

    // Persisted pause (`badi pause`) committed while the provider runs.
    let pending = start();
    provider.entered.notified().await;
    broker
        .replace_settings(
            1,
            SettingsV2 {
                paused: true,
                ..controlled_settings(2, true, false)
            },
        )
        .await
        .expect("persisted pause");
    provider.release.notify_one();
    let paused = pending.await.expect("probe task").expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(paused.text, None);
    paused.validate().expect("valid paused result");

    // While persisted pause holds, a probe never reaches the provider.
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
    let paused = broker
        .probe(probe_request("Thank you", "en", false))
        .await
        .expect("probe");
    assert_eq!(paused.outcome, ProbeOutcome::Paused);
    assert_eq!(provider.calls.load(Ordering::Relaxed), 3);
    let metrics = broker.metrics().snapshot();
    assert_eq!(metrics.provider_calls, 0);
    assert_eq!(metrics.no_suggestion.expect("breakdown").total(), 0);
}
