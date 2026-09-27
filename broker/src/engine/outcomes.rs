//! The content-free outcome recorder: a bounded queue to one worker thread
//! that owns every personalization-store write, so suggestion traffic never
//! waits for the disk.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::oneshot;
use tokio::time;

use super::{Broker, BrokerError};
use crate::control_plane::{ControlPlane, ControlPlaneError, ControlPlaneSnapshot};
use crate::personalization::{PersonalizationProvider, PersonalizationSignal};
use crate::protocol::TargetDescriptor;
use crate::settings::StableIdentity;

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

    pub(super) async fn flush_outcomes_before_pause_ack(&self) {
        let Some(recorder) = self.inner.outcome_recorder.as_ref() else {
            return;
        };
        if let Err(error) = recorder.flush().await {
            // A disconnected recorder cannot perform later writes. Keep the
            // pause effective and report the operational loss without logging
            // any context or suggestion content.
            eprintln!("badi-broker: outcome recorder pause fence failed: {error}");
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
                let _ = response.send(());
            }
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
    }
}

pub(super) fn current_unix_day() -> Option<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs() / 86_400)
}
