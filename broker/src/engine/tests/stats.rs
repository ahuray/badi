//! Content-free usage statistics: what counts, what never does, and that the
//! counts reach the disk the next broker reads.

use super::*;
use crate::stats::{STATS_FILE_NAME, StatsCounts, StatsStore, local_today};

fn controlled_identity() -> StableIdentity {
    StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Http,
        "localhost",
        Some(4173),
    )
    .expect("controlled identity")
}

fn controlled_broker(root: &std::path::Path) -> Broker {
    let paths =
        StoragePaths::new(root.join("config/badi"), root.join("data/badi")).expect("storage paths");
    let control_plane = std::sync::Arc::new(ControlPlane::open(paths).expect("control plane"));
    control_plane
        .replace_settings(0, controlled_settings(1, true, false))
        .expect("allow the controlled site");
    Broker::with_control_plane(
        std::sync::Arc::new(CountingProvider::new(Duration::ZERO)),
        BrokerConfig::default(),
        control_plane,
    )
    .expect("controlled broker")
}

async fn open_controlled_session(broker: &Broker) -> (SessionId, mpsc::Receiver<BrokerEvent>) {
    let session_id = SessionId::new();
    let (sink, events) = mpsc::channel(8);
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
    (session_id, events)
}

/// Opens a session and asks for a suggestion, ignoring refusals; true when
/// one was shown.
async fn try_to_show(broker: &Broker) -> bool {
    let session_id = SessionId::new();
    let (sink, mut events) = mpsc::channel(8);
    if broker
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
        .is_err()
    {
        return false;
    }
    let update = context(1, FieldPurpose::Normal);
    let _ = broker
        .update_context(coordinates(session_id, 1), update.clone())
        .await;
    let _ = broker
        .request_suggestion(
            coordinates(session_id, 1),
            SuggestRequestPayload {
                fingerprint: update.fingerprint,
                explicit: true,
            },
            None,
        )
        .await;
    loop {
        match timeout(Duration::from_millis(100), events.recv()).await {
            Ok(Some(BrokerEvent::SuggestionShow { .. })) => return true,
            Ok(Some(_)) => {}
            Ok(None) | Err(_) => return false,
        }
    }
}

#[tokio::test]
async fn shows_and_landed_acceptances_count_and_reach_the_disk() {
    let temporary = tempdir().expect("temporary directory");
    let broker = controlled_broker(temporary.path());
    let (session_id, mut events) = open_controlled_session(&broker).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptAll)
            .await;
    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint.clone(),
                suggestion_id,
                status: CommitStatus::Applied,
            },
        )
        .await
        .expect("applied");

    let status = broker.stats_status().await.expect("statistics");
    assert!(status.available);
    let saved = broker
        .metrics()
        .snapshot()
        .word_savings
        .expect("word savings")
        .applied;
    assert!(saved > 0);
    let expected = StatsCounts {
        shown: 1,
        accepted: 1,
        words: saved,
    };
    assert_eq!(status.summary.totals.today, expected);
    assert_eq!(status.summary.subjects.len(), 1);
    assert_eq!(status.summary.subjects[0].identity, controlled_identity());
    assert_eq!(status.summary.subjects[0].today, expected);

    // Shutdown fences the counts onto disk, where the next broker reads them.
    broker.shutdown().await;
    let path = temporary.path().join("data/badi").join(STATS_FILE_NAME);
    let persisted = std::fs::read_to_string(&path).expect("persisted statistics");
    assert!(!persisted.contains(&update.fingerprint));
    let mut keys = std::collections::BTreeSet::new();
    collect_keys(
        &serde_json::from_str(&persisted).expect("statistics JSON"),
        &mut keys,
    );
    let allowed = [
        "schema", "days", "day", "subjects", "other", "identity", "kind", "adapter", "scheme",
        "host", "port", "app_id", "shown", "accepted", "words",
    ];
    assert!(
        keys.iter().all(|key| allowed.contains(&key.as_str())),
        "{keys:?}"
    );
    let reopened = StatsStore::open(path).expect("readable after restart");
    assert_eq!(
        reopened.summary(local_today()).totals,
        status.summary.totals
    );
}

#[tokio::test]
async fn a_failed_acceptance_counts_only_the_show() {
    let temporary = tempdir().expect("temporary directory");
    let broker = controlled_broker(temporary.path());
    let (session_id, mut events) = open_controlled_session(&broker).await;
    let (update, suggestion_id) =
        prepare_commit_result_case(&broker, session_id, &mut events, ControlAction::AcceptWord)
            .await;
    broker
        .commit_result(
            coordinates(session_id, 1),
            CommitResultPayload {
                fingerprint: update.fingerprint,
                suggestion_id,
                status: CommitStatus::Failed,
            },
        )
        .await
        .expect("failed result");
    let status = broker.stats_status().await.expect("statistics");
    assert_eq!(
        status.summary.totals.today,
        StatsCounts {
            shown: 1,
            accepted: 0,
            words: 0,
        }
    );
}

#[tokio::test]
async fn nothing_counts_while_paused_or_for_a_blocked_site() {
    let temporary = tempdir().expect("temporary directory");
    let broker = controlled_broker(temporary.path());
    assert!(try_to_show(&broker).await, "the allowed site suggests");
    let counted = broker.stats_status().await.expect("statistics");
    assert_eq!(counted.summary.totals.today.shown, 1);

    // Runtime pause (badictl pause, or the session helper's hold).
    broker.set_paused(true).await;
    assert!(!try_to_show(&broker).await);
    broker.set_paused(false).await;

    // Persistent pause (badi pause).
    let paused = SettingsV2 {
        paused: true,
        ..controlled_settings(2, true, false)
    };
    broker.replace_settings(1, paused).await.expect("pause");
    assert!(!try_to_show(&broker).await);

    // The site turned off.
    broker
        .replace_settings(2, controlled_settings(3, false, false))
        .await
        .expect("block the site");
    assert!(!try_to_show(&broker).await);

    let status = broker.stats_status().await.expect("statistics");
    assert_eq!(status.summary.totals, counted.summary.totals);

    let forgotten = broker.forget_stats().await.expect("forget");
    assert_eq!(forgotten.summary.days, Vec::new());
    assert_eq!(
        broker
            .stats_status()
            .await
            .expect("statistics")
            .summary
            .totals,
        crate::stats::StatsTotals::default()
    );
}

fn collect_keys(value: &serde_json::Value, keys: &mut std::collections::BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                keys.insert(key.clone());
                collect_keys(value, keys);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_keys(item, keys);
            }
        }
        _ => {}
    }
}
