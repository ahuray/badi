//! Explicit, fixed-input diagnostic for the pinned runtime's `n_predict=0`
//! behavior. It is deliberately outside all UI and JSON request modes.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::semantic::client::{ClientError, PrefillMetrics};
use crate::semantic::runtime::{RuntimeLifecycleObservation, StableRuntimeIdentity};

const TOTAL_BUDGET: Duration = Duration::from_secs(60);
const REQUEST_BUDGET: Duration = Duration::from_secs(5);
const PREFIX: &str = "The disposable Badi cache probe has a blue notebook on the desk.";
const APPENDED: &str = "The disposable Badi cache probe has a blue notebook on the desk. Another";
const STEPS: [(&str, &str); 3] = [
    ("cold", PREFIX),
    ("identical", PREFIX),
    ("appended", APPENDED),
];

#[derive(Debug, Serialize)]
pub struct ProbeStep {
    name: &'static str,
    prompt_bytes: usize,
    requested_n_predict: u8,
    outcome: &'static str,
    error: Option<&'static str>,
    latency_ms: f64,
    metrics: Option<PrefillMetrics>,
}

#[derive(Debug, Serialize)]
pub struct ProbeGroup {
    stream: bool,
    identity: Option<StableRuntimeIdentity>,
    startup_ms: f64,
    steps: Vec<ProbeStep>,
    cleanup: Option<RuntimeLifecycleObservation>,
    error: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub struct ProbeReport {
    schema: &'static str,
    pub complete: bool,
    cancelled: bool,
    error: Option<&'static str>,
    total_budget_ms: u64,
    request_budget_ms: u64,
    latency_ms: f64,
    groups: Vec<ProbeGroup>,
}

fn error_code(error: &ClientError) -> &'static str {
    match error {
        ClientError::Cancelled => "cancelled",
        ClientError::Timeout => "deadline",
        ClientError::ResponseTooLarge => "response_too_large",
        ClientError::MalformedStream => "malformed_response",
        ClientError::UnexpectedStatus(_) => "http_error",
        ClientError::UnexpectedContentType => "content_type_error",
        _ => "transport_error",
    }
}

/// Runs two fresh owned runtimes and always attempts explicit shutdown after
/// activation. Keep this future directly under the main thread's `block_on`.
pub async fn run(directory: PathBuf, cancellation: CancellationToken) -> ProbeReport {
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + TOTAL_BUDGET;
    let mut groups = Vec::new();
    for stream in [false, true] {
        if cancellation.is_cancelled() || tokio::time::Instant::now() >= deadline {
            break;
        }
        let mut group = ProbeGroup {
            stream,
            identity: None,
            startup_ms: 0.0,
            steps: Vec::new(),
            cleanup: None,
            error: None,
        };
        let startup = Instant::now();
        let activation = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err("cancelled"),
            result = tokio::time::timeout_at(deadline, super::activate(directory.clone())) => {
                match result {
                    Err(_) => Err("deadline"),
                    Ok(Err(_)) => Err("runtime_start_failed"),
                    Ok(Ok(runtime)) => Ok(runtime),
                }
            },
        };
        group.startup_ms = startup.elapsed().as_secs_f64() * 1000.0;
        let runtime = match activation {
            Ok(runtime) => runtime,
            Err(error) => {
                group.error = Some(error);
                groups.push(group);
                break;
            }
        };
        group.identity = Some(runtime.identity().clone());
        for (name, prompt) in STEPS {
            let budget = deadline
                .saturating_duration_since(tokio::time::Instant::now())
                .min(REQUEST_BUDGET);
            let request_start = Instant::now();
            let result = if budget.is_zero() {
                Err(ClientError::Timeout)
            } else {
                runtime
                    .client()
                    .prefill_probe(prompt, stream, budget, cancellation.clone())
                    .await
            };
            let (metrics, error) = match result {
                Ok(metrics) => (Some(metrics), None),
                Err(error) => (None, Some(error_code(&error))),
            };
            group.steps.push(ProbeStep {
                name,
                prompt_bytes: prompt.len(),
                requested_n_predict: 0,
                outcome: if error.is_some() { "error" } else { "observed" },
                error,
                latency_ms: request_start.elapsed().as_secs_f64() * 1000.0,
                metrics,
            });
            if error.is_some() {
                group.error = error;
                break;
            }
        }
        // Cleanup is outside the inference budget, using the existing bounded
        // graceful shutdown and kill/reap fallback. Never hide a cleanup error.
        match runtime.shutdown() {
            Ok(receipt) => group.cleanup = Some(receipt),
            Err(_) => group.error = Some("runtime_shutdown_failed"),
        }
        let failed = group.error.is_some();
        groups.push(group);
        if failed {
            break;
        }
    }
    ProbeReport::finish(groups, started, cancellation.is_cancelled())
}

impl ProbeReport {
    fn finish(groups: Vec<ProbeGroup>, started: Instant, cancelled: bool) -> Self {
        let complete = groups.len() == 2
            && groups.iter().all(|group| {
                group.steps.len() == 3 && group.cleanup.is_some() && group.error.is_none()
            });
        let error = if complete {
            None
        } else {
            Some(
                groups
                    .last()
                    .and_then(|group| group.error)
                    .unwrap_or(if cancelled { "cancelled" } else { "deadline" }),
            )
        };
        Self {
            schema: "badi.prefill-probe.v1",
            complete,
            cancelled,
            error,
            total_budget_ms: 60_000,
            request_budget_ms: 5_000,
            latency_ms: started.elapsed().as_secs_f64() * 1000.0,
            groups,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_prefill_probe_never_activates_a_runtime() {
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let report = run(PathBuf::from("/nonexistent-prefill-fixture"), cancellation).await;
        assert!(!report.complete && report.cancelled);
        assert!(report.groups.is_empty());
    }
}
