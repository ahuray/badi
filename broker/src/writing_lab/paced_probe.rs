//! Fixed-contract paced context experiment. This diagnostic owns one runtime
//! for one trace/arm and grants no application editing authority.

mod scheduler;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::semantic::client::PrefillMetrics;
use crate::semantic::runtime::{RuntimeLifecycleObservation, StableRuntimeIdentity};

use super::{Config, Mode, Request, ResultRecord};

pub const MAX_INPUT_BYTES: usize = 256 * 1024;
const PRELUDE_MS: u64 = 1500;
const EVENT_TIMES: [u64; 4] = [0, 100, 250, 500];
const STARTUP_BUDGET: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    Cold,
    Primed,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventInput {
    pub at_ms: u64,
    pub request: Request,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeInput {
    schema: String,
    arm: Arm,
    prelude_ms: u64,
    events: Vec<EventInput>,
}

impl ProbeInput {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != "badi.paced-probe.request.v1"
            || self.prelude_ms != PRELUDE_MS
            || self.events.len() != EVENT_TIMES.len()
        {
            return Err("invalid_paced_request");
        }
        let config = Config {
            mode: Mode::Healed,
            budget_ms: 550,
            max_tokens: 8,
            cache_prompt: true,
            temperature: 0.0,
            seed: 42,
        };
        let initial = &self.events[0].request;
        let mut ids = std::collections::HashSet::new();
        for (event, at_ms) in self.events.iter().zip(EVENT_TIMES) {
            let request = &event.request;
            if request.validate().is_err() && !transient_joiner(request) {
                return Err("invalid_paced_request");
            }
            let id = uuid::Uuid::parse_str(&request.id).map_err(|_| "invalid_paced_request")?;
            if event.at_ms != at_ms
                || id.to_string() != request.id
                || !ids.insert(id)
                || request.config != config
                || request.context != initial.context
                || request.style_examples != initial.style_examples
                || request.language != initial.language
            {
                return Err("invalid_paced_request");
            }
        }
        if self
            .events
            .windows(2)
            .any(|pair| !appends_one_scalar(&pair[0].request.before, &pair[1].request.before))
        {
            return Err("invalid_paced_request");
        }
        if initial.context.trim().is_empty() {
            return Err("missing_paced_context");
        }
        Ok(())
    }
}

fn appends_one_scalar(previous: &str, next: &str) -> bool {
    next.strip_prefix(previous)
        .is_some_and(|addition| addition.chars().count() == 1)
}

// This invalid intermediate draft is still a typing revision. Only the paced
// scheduler may observe it, without inference. The validation sentinel proves
// the unchanged Arabic-letter joiner rule and is never used as model input.
fn transient_joiner(request: &Request) -> bool {
    if request.before.chars().count() > 2048 {
        return false;
    }
    let Some(before) = request.before.strip_suffix('\u{200c}') else {
        return false;
    };
    let mut precursor = request.clone();
    precursor.before.truncate(before.len());
    precursor.validate().is_ok()
        && crate::segment::valid_orthographic_joiners(&format!("{}ا", request.before))
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(try_from = "ControlMessage")]
pub enum Control {
    Cancel,
    Clear,
    ContextChange,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlMessage {
    r#type: String,
}

impl TryFrom<ControlMessage> for Control {
    type Error = &'static str;

    fn try_from(message: ControlMessage) -> Result<Self, Self::Error> {
        match message.r#type.as_str() {
            "cancel" => Ok(Self::Cancel),
            "clear" => Ok(Self::Clear),
            "context_change" => Ok(Self::ContextChange),
            _ => Err("invalid_control"),
        }
    }
}

#[derive(Clone, Default)]
pub struct ProbeCancellation {
    token: CancellationToken,
    cause: Arc<AtomicU8>,
}

impl ProbeCancellation {
    pub async fn cancelled(&self) {
        self.token.cancelled().await;
    }

    pub fn cancel(&self, control: Control) {
        let value = match control {
            Control::Cancel => 1,
            Control::Clear => 2,
            Control::ContextChange => 3,
        };
        let _ = self
            .cause
            .compare_exchange(0, value, Ordering::SeqCst, Ordering::SeqCst);
        self.token.cancel();
    }

    fn reason(&self) -> &'static str {
        match self.cause.load(Ordering::SeqCst) {
            2 => "cleared",
            3 => "context_changed",
            _ => "cancelled",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Prelude {
    duration_ms: u64,
    elapsed_ms: f64,
    preflight_ms: Option<f64>,
    priming_ms: Option<f64>,
    metrics: Option<PrefillMetrics>,
}

#[derive(Debug, Serialize)]
pub struct EventRecord {
    #[serde(skip)]
    dispatchable: bool,
    request_id: String,
    at_ms: u64,
    observed_at_ms: Option<f64>,
    deadline_ms: u64,
    started_at_ms: Option<f64>,
    finished_at_ms: Option<f64>,
    queue_ms: Option<f64>,
    scheduling_lateness_ms: Option<f64>,
    delivered_at_ms: Option<f64>,
    disposition: &'static str,
    reason: &'static str,
    result: Option<ResultRecord>,
}

#[derive(Debug, Serialize)]
pub struct ProbeReport {
    schema: &'static str,
    arm: Arm,
    pub complete: bool,
    cancelled: bool,
    error: Option<&'static str>,
    startup_ms: f64,
    prelude: Prelude,
    latency_ms: f64,
    identity: Option<StableRuntimeIdentity>,
    runtime_identity_sha256: Option<String>,
    cleanup: Option<RuntimeLifecycleObservation>,
    cleanup_ms: Option<f64>,
    events: Vec<EventRecord>,
}

#[derive(Serialize)]
pub struct Ready<'a> {
    r#type: &'static str,
    schema: &'static str,
    runtime_pid: Option<u32>,
    identity: &'a StableRuntimeIdentity,
    runtime_identity_sha256: String,
}

fn milliseconds(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

impl ProbeReport {
    fn new(input: &ProbeInput) -> Self {
        Self {
            schema: "badi.paced-probe.report.v1",
            arm: input.arm,
            complete: false,
            cancelled: false,
            error: None,
            startup_ms: 0.0,
            prelude: Prelude {
                duration_ms: PRELUDE_MS,
                elapsed_ms: 0.0,
                preflight_ms: None,
                priming_ms: None,
                metrics: None,
            },
            latency_ms: 0.0,
            identity: None,
            runtime_identity_sha256: None,
            cleanup: None,
            cleanup_ms: None,
            events: scheduler::records(&input.events),
        }
    }

    fn failure(&mut self, reason: &'static str) {
        self.error = Some(reason);
        for event in &mut self.events {
            if event.reason == "not_observed" {
                event.reason = reason;
            }
        }
    }
}

// The primer future is never polled in the cold arm. The epoch remains anchored
// to the original deadline even if the timer wakes late.
async fn prelude<Operation>(
    arm: Arm,
    started: Instant,
    cancellation: &ProbeCancellation,
    prime: Operation,
) -> Result<Instant, &'static str>
where
    Operation: std::future::Future<Output = Result<(), &'static str>>,
{
    let deadline = started + Duration::from_millis(PRELUDE_MS);
    if arm == Arm::Primed {
        tokio::select! {
            biased;
            () = cancellation.token.cancelled() => return Err(cancellation.reason()),
            () = tokio::time::sleep_until(deadline) => return Err("prelude_deadline"),
            result = prime => result?,
        }
        if Instant::now() > deadline {
            return Err("prelude_deadline");
        }
    }
    tokio::select! {
        biased;
        () = cancellation.token.cancelled() => Err(cancellation.reason()),
        () = tokio::time::sleep_until(deadline) => Ok(deadline),
    }
}

async fn prime(
    client: &crate::semantic::client::SemanticClient,
    prefix: &str,
    deadline: Instant,
    cancellation: &ProbeCancellation,
    record: &mut Prelude,
) -> Result<(), &'static str> {
    let started = Instant::now();
    let budget = deadline.saturating_duration_since(started);
    let count = client
        .lab_token_count(
            prefix,
            u64::try_from(budget.as_millis()).unwrap_or(0),
            cancellation.token.clone(),
        )
        .await;
    record.preflight_ms = Some(milliseconds(started));
    let count = count.map_err(|error| match error {
        crate::semantic::client::ClientError::Timeout => "prelude_deadline",
        _ => "primer_preflight_failed",
    })?;
    if count + 2 > usize::from(super::CONTEXT_TOKENS) {
        return Err("primer_context_overflow");
    }
    let started = Instant::now();
    let metrics = client
        .prime_context(
            prefix,
            deadline.saturating_duration_since(started),
            cancellation.token.clone(),
        )
        .await;
    record.priming_ms = Some(milliseconds(started));
    let metrics = metrics.map_err(|error| match error {
        crate::semantic::client::ClientError::Timeout => "prelude_deadline",
        _ => "primer_request_failed",
    })?;
    let verified = metrics.resolved_n_predict == Some(1)
        && metrics.returned_token_count == Some(1)
        && metrics.tokens_predicted == Some(1)
        && metrics.predicted_n == Some(1)
        && metrics.truncated == Some(false);
    record.metrics = Some(metrics);
    if verified {
        Ok(())
    } else {
        Err("primer_generation_unverified")
    }
}

/// Own one runtime from activation through verified shutdown. Call directly
/// under main's `block_on`: Linux parent-death protection belongs to that thread.
pub async fn run(
    directory: PathBuf,
    input: ProbeInput,
    cancellation: ProbeCancellation,
    ready: impl FnMut(&Ready<'_>) -> Result<(), ()>,
) -> Result<ProbeReport, &'static str> {
    run_with_artifact(directory, None, input, cancellation, ready).await
}

pub async fn run_with_artifact(
    directory: PathBuf,
    artifact: Option<super::artifact::ModelArtifactOverride>,
    input: ProbeInput,
    cancellation: ProbeCancellation,
    ready: impl FnMut(&Ready<'_>) -> Result<(), ()>,
) -> Result<ProbeReport, &'static str> {
    run_with_options(
        directory,
        artifact,
        super::PrefillBatch::default(),
        input,
        cancellation,
        ready,
    )
    .await
}

pub async fn run_with_options(
    directory: PathBuf,
    artifact: Option<super::artifact::ModelArtifactOverride>,
    prefill_batch: super::PrefillBatch,
    input: ProbeInput,
    cancellation: ProbeCancellation,
    mut ready: impl FnMut(&Ready<'_>) -> Result<(), ()>,
) -> Result<ProbeReport, &'static str> {
    input.validate()?;
    let started = Instant::now();
    let mut report = ProbeReport::new(&input);
    let activation = tokio::select! {
        biased;
        () = cancellation.token.cancelled() => Err(cancellation.reason()),
        result = tokio::time::timeout(STARTUP_BUDGET, super::activate_with_options(directory, artifact, prefill_batch)) => {
            match result {
                Ok(Ok(runtime)) => Ok(runtime),
                Ok(Err(_)) => Err("runtime_start_failed"),
                Err(_) => Err("runtime_start_deadline"),
            }
        },
    };
    report.startup_ms = milliseconds(started);
    let runtime = match activation {
        Ok(runtime) => runtime,
        Err(reason) => {
            report.failure(reason);
            report.cancelled = cancellation.token.is_cancelled();
            report.latency_ms = milliseconds(started);
            return Ok(report);
        }
    };
    report.identity = Some(runtime.identity().clone());
    report.runtime_identity_sha256 = Some(runtime.identity().sha256());
    let ready_result = ready(&Ready {
        r#type: "ready",
        schema: "badi.prediction-lab.worker.v1",
        runtime_pid: runtime.process_id(),
        identity: runtime.identity(),
        runtime_identity_sha256: runtime.identity().sha256(),
    });
    if ready_result.is_err() {
        report.failure("output_closed");
    } else if report.startup_ms > STARTUP_BUDGET.as_secs_f64() * 1000.0 {
        report.failure("runtime_start_deadline");
    } else {
        // Only stable context/style enter the primer. All drafts remain private
        // to the scheduler until the corresponding typing event is observed.
        let initial = &input.events[0].request;
        let prefix = super::context_prefix(&initial.context, &initial.style_examples);
        let prelude_start = Instant::now();
        let epoch = prelude(
            input.arm,
            prelude_start,
            &cancellation,
            prime(
                runtime.client(),
                &prefix,
                prelude_start + Duration::from_millis(PRELUDE_MS),
                &cancellation,
                &mut report.prelude,
            ),
        )
        .await;
        report.prelude.elapsed_ms = milliseconds(prelude_start);
        match epoch {
            Ok(epoch) => {
                let schedule =
                    scheduler::run(&input.events, epoch, &cancellation, |request, token| {
                        super::run(&runtime, request, token)
                    })
                    .await;
                report.events = schedule.events;
                report.error = schedule.error;
            }
            Err(reason) => report.failure(reason),
        }
    }
    // Any deadline may leave HTTP work active. Shutdown is mandatory before
    // reporting completion; no request is restarted in this owned runtime.
    let cleanup_start = Instant::now();
    match runtime.shutdown() {
        Ok(receipt) => report.cleanup = Some(receipt),
        Err(_) => report.failure("runtime_shutdown_failed"),
    }
    report.cleanup_ms = Some(milliseconds(cleanup_start));
    report.cancelled = cancellation.token.is_cancelled();
    if report.cancelled {
        report.failure(cancellation.reason());
    }
    report.complete = report.cleanup.is_some()
        && !report.cancelled
        && matches!(
            report.error,
            None | Some("active_deadline" | "prelude_deadline")
        );
    report.latency_ms = milliseconds(started);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn input() -> ProbeInput {
        ProbeInput {
            schema: "badi.paced-probe.request.v1".to_owned(),
            arm: Arm::Cold,
            prelude_ms: 1500,
            events: EVENT_TIMES
                .into_iter()
                .enumerate()
                .map(|(index, at_ms)| EventInput {
                    at_ms,
                    request: Request {
                        schema: super::super::REQUEST_SCHEMA.to_owned(),
                        id: uuid::Uuid::new_v4().to_string(),
                        before: format!("Disposable draft snapshot {}", &"abc"[..index]),
                        language: "en".to_owned(),
                        context: "Supplied stable context.".to_owned(),
                        style_examples: vec!["Supplied style example.".to_owned()],
                        config: Config {
                            mode: Mode::Healed,
                            budget_ms: 550,
                            max_tokens: 8,
                            cache_prompt: true,
                            temperature: 0.0,
                            seed: 42,
                        },
                    },
                })
                .collect(),
        }
    }

    pub(super) fn result(request: &Request) -> ResultRecord {
        let identity = StableRuntimeIdentity {
            launch_contract_id: "fixture-no-runtime",
            binary_sha256: "0".repeat(64),
            runtime_bundle_manifest_sha256: None,
            model_sha256: "0".repeat(64),
            model_size: 0,
            model_alias: "fixture-no-runtime".to_owned(),
            model_origin: None,
            threads: 1,
            context_size: 2048,
            gpu_layers: 0,
            batch_size: None,
            ubatch_size: None,
        };
        let mut result = request.result(&identity);
        result.outcome = "suggestion";
        result.text = Some("continues here".to_owned());
        result.word_complete = true;
        result.shape_valid = true;
        result.terminal_received = Some(true);
        result
    }

    #[test]
    fn strict_paced_input_keeps_fixed_schedule_configuration_and_context() {
        input().validate().unwrap();
        let mut changed = input();
        changed.events[1].at_ms += 1;
        assert!(changed.validate().is_err());
        changed = input();
        changed.events[1].request.id = changed.events[0].request.id.clone();
        assert!(changed.validate().is_err());
        changed = input();
        changed.events[2].request.context.push('x');
        assert!(changed.validate().is_err());
        changed = input();
        changed.events[3].request.config.budget_ms = 551;
        assert!(changed.validate().is_err());
        changed = input();
        changed.events[3].request.language = "de".to_owned();
        assert!(changed.validate().is_err());
        assert!(
            serde_json::from_str::<Control>(r#"{"type":"clear","context":"new text"}"#).is_err()
        );
    }

    #[test]
    fn primer_contains_only_identical_stable_prefix_and_preserves_all_separators() {
        let input = input();
        let request = &input.events[0].request;
        let prefix = super::super::context_prefix(&request.context, &request.style_examples);
        assert_eq!(
            prefix,
            "Supplied stable context.\n\nSupplied style example.\n\n"
        );
        for event in &input.events {
            let prompt = super::super::prepare_prompt(&event.request);
            assert!(
                prompt.payload["prompt"]
                    .as_str()
                    .unwrap()
                    .starts_with(&prefix)
            );
            assert!(!prefix.contains(&event.request.before));
        }
    }

    #[test]
    fn paced_additions_count_unicode_scalars_and_preserve_joiner_guards() {
        let mut input = input();
        for (event, before) in input
            .events
            .iter_mut()
            .zip(["می‌ر", "می‌رو", "می‌روم", "می‌روم "])
        {
            event.request.before = before.to_owned();
            event.request.language = "fa".to_owned();
        }
        input.validate().unwrap();
        assert!(appends_one_scalar("می", "می\u{200c}"));
        assert!(!appends_one_scalar("می", "می\u{200c}ر"));
        assert!(!appends_one_scalar("abc", "abx"));
        assert!(!appends_one_scalar("abc", "abc"));
        input.events[1].request.before.push('م');
        assert!(input.validate().is_err());
        input.events[0].request.before = "می\u{200c}".to_owned();
        assert!(input.events[0].request.validate().is_err());
    }

    #[test]
    fn only_paced_transient_arabic_joiner_is_accepted_as_an_unavailable_revision() {
        let mut input = input();
        for (event, before) in input.events.iter_mut().zip(["می", "می‌", "می‌ر", "می‌رو"])
        {
            event.request.before = before.to_owned();
            event.request.language = "fa".to_owned();
        }
        assert!(input.events[1].request.validate().is_err());
        assert!(transient_joiner(&input.events[1].request));
        input.validate().unwrap();
        let mut invalid = input.events[1].request.clone();
        for before in ["a\u{200c}", "می\u{200c}\u{200c}", "\u{200c}"] {
            invalid.before = before.to_owned();
            assert!(!transient_joiner(&invalid));
        }
        invalid = input.events[1].request.clone();
        invalid.before = format!("{}\u{200c}", "ا".repeat(2047));
        assert!(transient_joiner(&invalid));
        invalid.before = format!("{}\u{200c}", "ا".repeat(2048));
        assert!(!transient_joiner(&invalid));
        invalid = input.events[1].request.clone();
        invalid.context = "<|invalid|>".to_owned();
        assert!(!transient_joiner(&invalid));
    }

    #[tokio::test(start_paused = true)]
    async fn both_arms_use_fixed_prelude_and_cold_never_polls_primer() {
        for arm in [Arm::Cold, Arm::Primed] {
            let started = Instant::now();
            let mut polled = false;
            let epoch = prelude(arm, started, &ProbeCancellation::default(), async {
                polled = true;
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(())
            })
            .await
            .unwrap();
            assert_eq!(epoch, started + Duration::from_millis(1500));
            assert_eq!(Instant::now(), epoch);
            assert_eq!(polled, arm == Arm::Primed);
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unfinished_prelude_stops_at_fixed_epoch_without_typing() {
        let started = Instant::now();
        let result = prelude(Arm::Primed, started, &ProbeCancellation::default(), async {
            tokio::time::sleep(Duration::from_millis(1600)).await;
            Ok(())
        })
        .await;
        assert_eq!(result, Err("prelude_deadline"));
        assert_eq!(started.elapsed(), Duration::from_millis(1500));
    }

    #[tokio::test(start_paused = true)]
    async fn all_controls_interrupt_prelude_and_retain_cause() {
        for (control, reason) in [
            (Control::Cancel, "cancelled"),
            (Control::Clear, "cleared"),
            (Control::ContextChange, "context_changed"),
        ] {
            let cancellation = ProbeCancellation::default();
            let trigger = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                trigger.cancel(control);
            });
            let started = Instant::now();
            let result = prelude(Arm::Primed, started, &cancellation, async {
                tokio::time::sleep(Duration::from_millis(400)).await;
                Ok(())
            })
            .await;
            assert_eq!(result, Err(reason));
            assert_eq!(started.elapsed(), Duration::from_millis(20));
        }
    }

    #[tokio::test]
    async fn cancelled_trace_never_activates_and_preserves_all_opportunities() {
        let cancellation = ProbeCancellation::default();
        cancellation.cancel(Control::Clear);
        let report = run(
            PathBuf::from("/missing-paced-fixture"),
            input(),
            cancellation,
            |_| panic!("cancelled trace must not become ready"),
        )
        .await
        .unwrap();
        assert!(!report.complete && report.cancelled);
        assert!(report.identity.is_none() && report.cleanup.is_none());
        assert_eq!(report.events.len(), 4);
        assert!(
            report
                .events
                .iter()
                .all(|row| row.disposition == "unavailable" && row.reason == "cleared")
        );
    }
}
