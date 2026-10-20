//! The content-free outcome recorder: a bounded queue to one worker thread
//! that owns every personalization, style, learned-word and usage-statistics
//! write, so suggestion traffic never waits for the disk.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::oneshot;
use tokio::time;

use super::{Broker, BrokerError};
use crate::control_plane::{ControlPlane, ControlPlaneError, ControlPlaneSnapshot};
use crate::personalization::{PersonalizationProvider, PersonalizationSignal};
use crate::protocol::{StatsStatusPayload, TargetDescriptor};
use crate::settings::StableIdentity;
use crate::stats::{StatsEvent, StatsStore, StatsSummary, local_today};
use crate::style::{StyleKey, StyleSample, StyleStatus};
use crate::vocabulary::learned::LearnedStatus;

const OUTCOME_QUEUE_CAPACITY: usize = 256;
const OUTCOME_FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
const PERSONALIZATION_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OutcomeRecorderHealth {
    pub available: bool,
    pub dropped_signals: u64,
    pub write_failures: u64,
}

#[derive(Clone)]
pub(super) struct OutcomeRecorder {
    sender: std_mpsc::SyncSender<OutcomeCommand>,
    available: Arc<AtomicBool>,
    dropped_signals: Arc<AtomicU64>,
    write_failures: Arc<AtomicU64>,
}

type ClearResult = Result<(bool, ControlPlaneSnapshot), ControlPlaneError>;

pub(super) enum OutcomeCommand {
    Signal {
        expected_settings_revision: u64,
        event_day: u64,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        signal: PersonalizationSignal,
    },
    Clear {
        response: oneshot::Sender<ClearResult>,
    },
    Snapshot {
        response: oneshot::Sender<Result<ControlPlaneSnapshot, ControlPlaneError>>,
    },
    Flush {
        response: oneshot::Sender<()>,
    },
    /// One authored sentence to seal, if the settings it was admitted under
    /// are still current.
    Style {
        expected_settings_revision: u64,
        identity: StableIdentity,
        text: String,
    },
    /// Ordered behind every queued sample, so none is sealed after a forget.
    StyleForget {
        response: oneshot::Sender<Result<StyleStatus, ControlPlaneError>>,
    },
    StyleExport {
        response: oneshot::Sender<Result<Vec<StyleSample>, ControlPlaneError>>,
    },
    /// Words one field newly shows, to count if the settings they were
    /// admitted under are still current (ADR 0010).
    Learned {
        expected_settings_revision: u64,
        identity: StableIdentity,
        words: Vec<String>,
    },
    /// Ordered behind every queued word, so none is kept after a forget.
    LearnedForget {
        response: oneshot::Sender<Result<LearnedStatus, ControlPlaneError>>,
    },
    /// One content-free usage count, if the settings it was admitted under
    /// are still current.
    Stats {
        expected_settings_revision: u64,
        identity: StableIdentity,
        event: StatsEvent,
    },
    /// Ordered behind every queued count, so the summary includes them.
    StatsQuery {
        response: oneshot::Sender<Result<Option<StatsSummary>, ControlPlaneError>>,
    },
    /// Ordered behind every queued count, so none is kept after a forget.
    StatsForget {
        response: oneshot::Sender<Result<StatsSummary, ControlPlaneError>>,
    },
}

impl Broker {
    #[must_use]
    pub fn outcome_recorder_health(&self) -> OutcomeRecorderHealth {
        self.inner.outcome_recorder.as_ref().map_or(
            OutcomeRecorderHealth {
                available: false,
                dropped_signals: 0,
                write_failures: 0,
            },
            OutcomeRecorder::health,
        )
    }

    /// Must be called under the broker state lock: a memory clear relies on
    /// it to order every outcome before or after its barrier.
    pub(super) fn queue_outcome(
        &self,
        target: &TargetDescriptor,
        expected_settings_revision: u64,
        event_day: u64,
        signal: PersonalizationSignal,
    ) -> bool {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return false;
        };
        let Ok(identity) = StableIdentity::from_target(target) else {
            return false;
        };
        recorder.try_signal(OutcomeCommand::Signal {
            expected_settings_revision,
            event_day,
            identity,
            provider: PersonalizationProvider::from(self.inner.provider_kind),
            signal,
        })
    }

    /// Must be called under the broker state lock, like [`Self::queue_outcome`].
    pub(super) fn queue_style_sample(
        &self,
        target: &TargetDescriptor,
        expected_settings_revision: u64,
        text: String,
    ) -> bool {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return false;
        };
        let Ok(identity) = StableIdentity::from_target(target) else {
            return false;
        };
        recorder.try_signal(OutcomeCommand::Style {
            expected_settings_revision,
            identity,
            text,
        })
    }

    /// Must be called under the broker state lock, like [`Self::queue_outcome`].
    pub(super) fn queue_learned_words(
        &self,
        identity: StableIdentity,
        expected_settings_revision: u64,
        words: Vec<String>,
    ) -> bool {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return false;
        };
        recorder.try_signal(OutcomeCommand::Learned {
            expected_settings_revision,
            identity,
            words,
        })
    }

    /// Must be called under the broker state lock, like [`Self::queue_outcome`].
    pub(super) fn queue_stats(
        &self,
        target: &TargetDescriptor,
        expected_settings_revision: u64,
        event: StatsEvent,
    ) -> bool {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return false;
        };
        let Ok(identity) = StableIdentity::from_target(target) else {
            return false;
        };
        recorder.try_signal(OutcomeCommand::Stats {
            expected_settings_revision,
            identity,
            event,
        })
    }

    /// Usage statistics, including every count queued before the call.
    pub async fn stats_status(&self) -> Result<StatsStatusPayload, BrokerError> {
        let recorder = self
            .inner
            .outcome_recorder
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        let (response, receiver) = oneshot::channel();
        recorder
            .send_waiting(OutcomeCommand::StatsQuery { response })
            .await?;
        let summary = receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)?;
        Ok(StatsStatusPayload {
            available: summary.is_some(),
            summary: summary.unwrap_or_else(|| StatsStore::empty_summary(local_today())),
        })
    }

    /// Deletes every usage count, including those queued before the call.
    pub async fn forget_stats(&self) -> Result<StatsStatusPayload, BrokerError> {
        let recorder = self
            .inner
            .outcome_recorder
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        let (response, receiver) = oneshot::channel();
        recorder
            .send_waiting(OutcomeCommand::StatsForget { response })
            .await?;
        let summary = receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)?;
        Ok(StatsStatusPayload {
            available: true,
            summary,
        })
    }

    /// The learned words `identity` offers now; empty without a control
    /// plane, key or readable file.
    pub(super) fn learned_words(&self, identity: &StableIdentity) -> Vec<String> {
        self.inner
            .control_plane
            .as_ref()
            .and_then(|control_plane| control_plane.learned_words(identity).ok())
            .unwrap_or_default()
    }

    /// Content-free learned-word counts; `None` without a control plane or
    /// while the sealed file is unreadable.
    #[must_use]
    pub fn learned_status(&self) -> Option<LearnedStatus> {
        self.inner
            .control_plane
            .as_ref()?
            .learned_status()
            .ok()
            .flatten()
    }

    pub async fn forget_learned(&self) -> Result<LearnedStatus, BrokerError> {
        let recorder = self
            .inner
            .outcome_recorder
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        let (response, receiver) = oneshot::channel();
        recorder
            .send_waiting(OutcomeCommand::LearnedForget { response })
            .await?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    /// Holds the keyring's style key; it is never written anywhere.
    pub fn deliver_style_key(&self, key: StyleKey) -> Result<StyleStatus, BrokerError> {
        let control_plane = self
            .inner
            .control_plane
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        Ok(control_plane.deliver_style_key(key)?)
    }

    /// Content-free style counts; `None` without a control plane or while the
    /// sealed store is unreadable.
    #[must_use]
    pub fn style_status(&self) -> Option<StyleStatus> {
        self.inner
            .control_plane
            .as_ref()?
            .style_status()
            .ok()
            .flatten()
    }

    pub async fn forget_style(&self) -> Result<StyleStatus, BrokerError> {
        let recorder = self
            .inner
            .outcome_recorder
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        let (response, receiver) = oneshot::channel();
        recorder
            .send_waiting(OutcomeCommand::StyleForget { response })
            .await?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    pub async fn export_style(&self) -> Result<Vec<StyleSample>, BrokerError> {
        let recorder = self
            .inner
            .outcome_recorder
            .as_ref()
            .ok_or(BrokerError::ControlPlaneUnavailable)?;
        let (response, receiver) = oneshot::channel();
        recorder
            .send_waiting(OutcomeCommand::StyleExport { response })
            .await?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    pub(super) async fn flush_outcomes_before_pause_ack(&self) -> Result<(), BrokerError> {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return Ok(());
        };
        // Same bound as shutdown. A hung recorder must not hold the pause
        // acknowledgement open, and a failed fence is not success.
        match time::timeout(OUTCOME_FLUSH_TIMEOUT, recorder.flush()).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => {
                eprintln!("badi-broker: outcome recorder pause fence failed: {error}");
                Err(error)
            }
            Err(_) => {
                eprintln!("badi-broker: outcome recorder pause fence timed out");
                Err(BrokerError::ControlPlaneTimeout)
            }
        }
    }

    pub(super) async fn flush_outcomes_before_shutdown(&self) {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return;
        };
        match time::timeout(OUTCOME_FLUSH_TIMEOUT, recorder.flush()).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("badi-broker: outcome aggregate flush failed: {error}"),
            Err(_) => eprintln!("badi-broker: outcome aggregate flush timed out"),
        }
    }
}

impl OutcomeRecorder {
    pub(super) fn new(control_plane: Arc<ControlPlane>) -> Result<Self, std::io::Error> {
        let (sender, receiver) = std_mpsc::sync_channel(OUTCOME_QUEUE_CAPACITY);
        let recorder = Self {
            sender,
            available: Arc::new(AtomicBool::new(true)),
            dropped_signals: Arc::new(AtomicU64::new(0)),
            write_failures: Arc::new(AtomicU64::new(0)),
        };
        let worker = OutcomeWorker {
            control_plane,
            receiver,
            available: Arc::clone(&recorder.available),
            dropped_signals: Arc::clone(&recorder.dropped_signals),
            write_failures: Arc::clone(&recorder.write_failures),
            reported_dropped: 0,
        };
        std::thread::Builder::new()
            .name("badi-outcomes".to_owned())
            .spawn(move || worker.run())?;
        Ok(recorder)
    }

    /// A recorder whose queue nobody drains.
    #[cfg(test)]
    pub(super) fn stalled(sender: std_mpsc::SyncSender<OutcomeCommand>) -> Self {
        Self {
            sender,
            available: Arc::default(),
            dropped_signals: Arc::default(),
            write_failures: Arc::default(),
        }
    }

    fn try_signal(&self, command: OutcomeCommand) -> bool {
        match self.sender.try_send(command) {
            Ok(()) => true,
            Err(std_mpsc::TrySendError::Full(_)) => {
                self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                false
            }
            Err(std_mpsc::TrySendError::Disconnected(_)) => {
                self.available.store(false, Ordering::Relaxed);
                self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                false
            }
        }
    }

    fn health(&self) -> OutcomeRecorderHealth {
        OutcomeRecorderHealth {
            available: self.available.load(Ordering::Relaxed),
            dropped_signals: self.dropped_signals.load(Ordering::Relaxed),
            write_failures: self.write_failures.load(Ordering::Relaxed),
        }
    }

    /// Queues a clear behind every outcome already queued, without waiting
    /// for queue space or for the disk.
    pub(super) fn begin_clear(&self) -> Result<oneshot::Receiver<ClearResult>, BrokerError> {
        let (response, receiver) = oneshot::channel();
        match self.sender.try_send(OutcomeCommand::Clear { response }) {
            Ok(()) => Ok(receiver),
            Err(std_mpsc::TrySendError::Full(_)) => Err(BrokerError::ControlPlaneUnavailable),
            Err(std_mpsc::TrySendError::Disconnected(_)) => {
                self.available.store(false, Ordering::Relaxed);
                Err(BrokerError::ControlPlaneUnavailable)
            }
        }
    }

    pub(super) async fn snapshot(&self) -> Result<ControlPlaneSnapshot, BrokerError> {
        let (response, receiver) = oneshot::channel();
        self.send_waiting(OutcomeCommand::Snapshot { response })
            .await?;
        receiver
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(BrokerError::from)
    }

    async fn flush(&self) -> Result<(), BrokerError> {
        let (response, receiver) = oneshot::channel();
        self.send_waiting(OutcomeCommand::Flush { response })
            .await?;
        receiver.await.map_err(|_| BrokerError::ControlPlaneTask)
    }

    /// Queues `command` behind every earlier one, waiting for queue space off
    /// the async runtime.
    async fn send_waiting(&self, command: OutcomeCommand) -> Result<(), BrokerError> {
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || sender.send(command))
            .await
            .map_err(|_| BrokerError::ControlPlaneTask)?
            .map_err(|_| BrokerError::ControlPlaneUnavailable)
    }
}

struct OutcomeWorker {
    control_plane: Arc<ControlPlane>,
    receiver: std_mpsc::Receiver<OutcomeCommand>,
    available: Arc<AtomicBool>,
    dropped_signals: Arc<AtomicU64>,
    write_failures: Arc<AtomicU64>,
    reported_dropped: u64,
}

impl OutcomeWorker {
    /// Executes commands in queue order and reconciles retention at least once
    /// per sweep interval, until every sender is gone.
    fn run(mut self) {
        let mut next_sweep = Instant::now() + PERSONALIZATION_SWEEP_INTERVAL;
        loop {
            let wait = next_sweep.saturating_duration_since(Instant::now());
            let sweep_due = match self.receiver.recv_timeout(wait) {
                Ok(command) => {
                    self.execute(command);
                    self.report_dropped_signals();
                    Instant::now() >= next_sweep
                }
                Err(std_mpsc::RecvTimeoutError::Timeout) => true,
                Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
            };
            if sweep_due {
                self.sweep_retention();
                next_sweep = Instant::now() + PERSONALIZATION_SWEEP_INTERVAL;
            }
        }
        self.available.store(false, Ordering::Relaxed);
    }

    fn execute(&self, command: OutcomeCommand) {
        match command {
            OutcomeCommand::Signal {
                expected_settings_revision,
                event_day,
                identity,
                provider,
                signal,
            } => {
                match self.control_plane.record_signal_at_settings_revision(
                    expected_settings_revision,
                    event_day,
                    identity,
                    provider,
                    signal,
                ) {
                    Ok(mutation) if mutation.signal_dropped => {
                        self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                    }
                    Ok(_) => {}
                    Err(error) => {
                        self.write_failures.fetch_add(1, Ordering::Relaxed);
                        eprintln!("badi-broker: outcome aggregate write failed: {error}");
                    }
                }
            }
            OutcomeCommand::Clear { response } => {
                let result = self
                    .control_plane
                    .clear_personalization()
                    .and_then(|mutation| {
                        self.control_plane
                            .snapshot()
                            .map(|snapshot| (mutation.changed, snapshot))
                    });
                let _ = response.send(result);
            }
            OutcomeCommand::Snapshot { response } => {
                let _ = response.send(self.control_plane.snapshot());
            }
            OutcomeCommand::Flush { response } => {
                // A pause or shutdown fence also puts the counts on disk.
                self.persist_stats();
                let _ = response.send(());
            }
            OutcomeCommand::Style {
                expected_settings_revision,
                identity,
                text,
            } => {
                if let Err(error) = self.control_plane.record_style_at_settings_revision(
                    expected_settings_revision,
                    &identity,
                    &text,
                ) {
                    // No key yet, a replaced key or a corrupt store: the
                    // sample is dropped and the reason is content-free.
                    self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                    eprintln!("badi-broker: style sample not kept: {error}");
                }
            }
            OutcomeCommand::StyleForget { response } => {
                let _ = response.send(self.control_plane.forget_style());
            }
            OutcomeCommand::StyleExport { response } => {
                let _ = response.send(self.control_plane.export_style());
            }
            OutcomeCommand::Learned {
                expected_settings_revision,
                identity,
                words,
            } => {
                if let Err(error) = self.control_plane.record_learned_at_settings_revision(
                    expected_settings_revision,
                    &identity,
                    &words,
                ) {
                    // No key yet or an unreadable file: the words are dropped
                    // and the reason is content-free.
                    self.dropped_signals.fetch_add(1, Ordering::Relaxed);
                    eprintln!("badi-broker: learned words not kept: {error}");
                }
            }
            OutcomeCommand::LearnedForget { response } => {
                let _ = response.send(self.control_plane.forget_learned());
            }
            OutcomeCommand::Stats {
                expected_settings_revision,
                identity,
                event,
            } => self.record_stats(expected_settings_revision, &identity, event),
            OutcomeCommand::StatsQuery { response } => {
                let _ = response.send(self.control_plane.stats_summary());
            }
            OutcomeCommand::StatsForget { response } => {
                let _ = response.send(self.control_plane.forget_stats());
            }
        }
    }

    fn record_stats(
        &self,
        expected_settings_revision: u64,
        identity: &StableIdentity,
        event: StatsEvent,
    ) {
        if let Err(error) = self.control_plane.record_stats_at_settings_revision(
            expected_settings_revision,
            identity,
            event,
        ) {
            // An unreadable statistics file: the count is dropped.
            self.dropped_signals.fetch_add(1, Ordering::Relaxed);
            eprintln!("badi-broker: usage count not kept: {error}");
        }
    }

    /// Counts reach the disk at most a sweep interval late, and at every
    /// pause or shutdown fence; a failed write keeps them for the next try.
    fn persist_stats(&self) {
        if let Err(error) = self.control_plane.persist_stats() {
            self.write_failures.fetch_add(1, Ordering::Relaxed);
            eprintln!("badi-broker: usage statistics write failed: {error}");
        }
    }

    fn report_dropped_signals(&mut self) {
        let dropped = self.dropped_signals.load(Ordering::Relaxed);
        if dropped > self.reported_dropped {
            let rejected = dropped - self.reported_dropped;
            self.reported_dropped = dropped;
            eprintln!(
                "badi-broker: dropped {rejected} outcome aggregate signal(s) because the recorder queue or bounded store could not admit them"
            );
        }
    }

    fn sweep_retention(&self) {
        if let Err(error) = self.control_plane.reconcile_personalization_now() {
            self.write_failures.fetch_add(1, Ordering::Relaxed);
            eprintln!(
                "badi-broker: periodic personalization retention reconciliation failed: {error}"
            );
        }
        self.persist_stats();
    }
}

pub(super) fn current_unix_day() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs() / 86_400)
}
