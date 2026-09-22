//! Fixed six-target mechanism screen. No application text, scoring or settings
//! are accepted. Each arm owns a fresh verified runtime and one common primer.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::artifact::ModelArtifactOverride;
use crate::semantic::client::ClientError;
use crate::semantic::client::stop_token_probe::{
    Arm, Observation, REQUEST_MS, Step, TokenCheck, Word,
};
use crate::semantic::runtime::{RuntimeLifecycleObservation, StableRuntimeIdentity};

const MODEL_SHA: &str = "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5";
const MODEL_BYTES: u64 = 1_282_439_264;
const DISPATCH_BUDGET_MS: u64 = 120_000;
const PLAN: [(Word, Arm); 6] = [
    (Word::Copper, Arm::SpaceEos),
    (Word::Copper, Arm::StopToken),
    (Word::Garten, Arm::StopToken),
    (Word::Garten, Arm::SpaceEos),
    (Word::Window, Arm::SpaceEos),
    (Word::Window, Arm::StopToken),
];

pub fn artifact_from(args: &[OsString]) -> Result<ModelArtifactOverride, &'static str> {
    let [flag, path] = args else {
        return Err("invalid_arguments");
    };
    if flag != "--model-artifact" || !PathBuf::from(path).is_absolute() {
        return Err("invalid_arguments");
    }
    let artifact = ModelArtifactOverride::read(&PathBuf::from(path))?;
    validate_artifact(&artifact)?;
    Ok(artifact)
}

fn validate_artifact(artifact: &ModelArtifactOverride) -> Result<(), &'static str> {
    if artifact.weights.sha256() != MODEL_SHA || artifact.weights.size() != MODEL_BYTES {
        return Err("stop_probe_model_mismatch");
    }
    Ok(())
}

fn validate_identity(identity: &StableRuntimeIdentity) -> Result<(), &'static str> {
    if identity.model_sha256 != MODEL_SHA
        || identity.model_size != MODEL_BYTES
        || identity.launch_contract_id != super::LAUNCH_CONTRACT
        || identity.threads != 4
        || identity.context_size != 2048
        || identity.gpu_layers != 0
        || identity.batch_size != Some(16)
        || identity.ubatch_size != Some(16)
    {
        return Err("stop_probe_identity_mismatch");
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct RequestRecord {
    payload: Value,
    attempted: bool,
    observation: Option<Observation>,
    error: Option<&'static str>,
}

impl RequestRecord {
    fn planned(step: Step) -> Self {
        Self {
            payload: step.payload(),
            attempted: false,
            observation: None,
            error: None,
        }
    }
}

#[derive(Debug, Serialize)]
struct Group {
    ordinal: usize,
    word: Word,
    arm: Arm,
    attempted: bool,
    process_id: Option<u32>,
    identity: Option<StableRuntimeIdentity>,
    runtime_identity_sha256: Option<String>,
    token_check: Option<TokenCheck>,
    startup_ms: f64,
    primer: RequestRecord,
    target: RequestRecord,
    cleanup: Option<RuntimeLifecycleObservation>,
    error: Option<&'static str>,
}

impl Group {
    fn planned(ordinal: usize, (word, arm): (Word, Arm)) -> Self {
        Self {
            ordinal,
            word,
            arm,
            attempted: false,
            process_id: None,
            identity: None,
            runtime_identity_sha256: None,
            token_check: None,
            startup_ms: 0.0,
            primer: RequestRecord::planned(Step::Primer),
            target: RequestRecord::planned(Step::Target(word, arm)),
            cleanup: None,
            error: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Report {
    schema: &'static str,
    pub complete: bool,
    cancelled: bool,
    error: Option<&'static str>,
    dispatch_budget_ms: u64,
    request_budget_ms: u64,
    planned_targets: usize,
    planned_primers: usize,
    attempted_targets: usize,
    attempted_primers: usize,
    latency_ms: f64,
    limitations: [&'static str; 4],
    groups: Vec<Group>,
}

fn client_error(error: &ClientError) -> &'static str {
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

fn may_dispatch(cancellation: &CancellationToken, started: Instant) -> Result<(), &'static str> {
    if cancellation.is_cancelled() {
        Err("cancelled")
    } else if started.elapsed() >= Duration::from_millis(DISPATCH_BUDGET_MS) {
        Err("dispatch_deadline")
    } else {
        Ok(())
    }
}

/// Await directly under main's `block_on`. Activation retains the existing
/// bounded startup and RAII cleanup. Cancellation during successful activation
/// is handled immediately afterward, preserving its exact PID/shutdown receipt.
/// The dispatch deadline stops new work; startup, in-flight request and cleanup
/// retain their existing independent bounds rather than inventing a hard clock.
pub async fn run(
    directory: PathBuf,
    artifact: ModelArtifactOverride,
    cancellation: CancellationToken,
    mut event: impl FnMut(&Value) -> Result<(), ()>,
) -> Report {
    let started = Instant::now();
    let mut groups: Vec<_> = PLAN
        .into_iter()
        .enumerate()
        .map(|(i, plan)| Group::planned(i, plan))
        .collect();
    let mut error = validate_artifact(&artifact).err();
    if error.is_none()
        && event(&json!({"type":"stop_probe_plan","groups":groups,
        "model_sha256":MODEL_SHA,"model_bytes":MODEL_BYTES,"request_budget_ms":REQUEST_MS,
        "order":"AB/BA/AB","max_targets":6,"max_primers":6}))
        .is_err()
    {
        error = Some("output_closed");
    }
    let mut first_identity = None;
    for group in &mut groups {
        if error.is_some() {
            break;
        }
        if let Err(code) = may_dispatch(&cancellation, started) {
            error = Some(code);
            break;
        }
        group.attempted = true;
        let startup = Instant::now();
        // Deliberately await activation: dropping it on a signal would hide the
        // successful child's identity from this diagnostic's explicit receipt.
        let activation =
            super::activate_with_artifact(directory.clone(), Some(artifact.clone())).await;
        group.startup_ms = startup.elapsed().as_secs_f64() * 1000.0;
        let Ok(runtime) = activation else {
            group.error = Some("runtime_start_failed");
            error = group.error;
            break;
        };
        group.process_id = runtime.process_id();
        group.identity = Some(runtime.identity().clone());
        group.runtime_identity_sha256 = Some(runtime.identity().sha256());
        let measured = async {
            validate_identity(runtime.identity())?;
            if first_identity.as_ref().is_some_and(|identity| identity != runtime.identity()) {
                return Err("runtime_identity_changed");
            }
            first_identity.get_or_insert_with(|| runtime.identity().clone());
            // Journal ownership before any model protocol request, including a
            // cancellation immediately after startup. No credential is emitted.
            event(&json!({"type":"stop_probe_runtime","group":group})).map_err(|()| "output_closed")?;
            may_dispatch(&cancellation, started)?;
            group.token_check = Some(runtime.client().stop_token_round_trip(cancellation.clone()).await.map_err(|err| client_error(&err))?);
            event(&json!({"type":"stop_probe_verified","group":group})).map_err(|()| "output_closed")?;
            for (name, step, record) in [("primer", Step::Primer, &mut group.primer),
                ("target", Step::Target(group.word, group.arm), &mut group.target)] {
                may_dispatch(&cancellation, started)?;
                event(&json!({"type":"stop_probe_dispatch","ordinal":group.ordinal,"step":name,"payload":record.payload}))
                    .map_err(|()| "output_closed")?;
                record.attempted = true;
                match runtime.client().stop_token_completion(step, cancellation.clone()).await {
                    Ok(observation) => {
                        record.error = step.validate(&observation).err();
                        record.observation = Some(observation);
                    }
                    Err(err) => record.error = Some(client_error(&err)),
                }
                event(&json!({"type":"stop_probe_response","ordinal":group.ordinal,"step":name,"record":record}))
                    .map_err(|()| "output_closed")?;
                if let Some(code) = record.error { return Err(code); }
            }
            Ok(())
        }.await;
        group.error = measured.err();
        match runtime.shutdown() {
            Ok(receipt) => {
                if !receipt.reaped()
                    || !receipt.challenge_completed()
                    || receipt.exit_code() != Some(0)
                    || Some(receipt.process_id()) != group.process_id
                    || Some(receipt.runtime_identity_sha256())
                        != group.runtime_identity_sha256.as_deref()
                {
                    group.error = Some("runtime_shutdown_failed");
                }
                group.cleanup = Some(receipt);
            }
            Err(_) => group.error = Some("runtime_shutdown_failed"),
        }
        if event(&json!({"type":"stop_probe_group_done","group":group})).is_err() {
            group.error = Some("output_closed");
        }
        error = group.error;
    }
    finish(groups, started, cancellation.is_cancelled(), error)
}

fn finish(
    groups: Vec<Group>,
    started: Instant,
    cancelled: bool,
    mut error: Option<&'static str>,
) -> Report {
    if cancelled && error.is_none() {
        error = Some("cancelled");
    }
    let attempted_targets = groups.iter().filter(|g| g.target.attempted).count();
    let attempted_primers = groups.iter().filter(|g| g.primer.attempted).count();
    let complete = error.is_none()
        && attempted_targets == PLAN.len()
        && groups.iter().all(|g| g.cleanup.is_some());
    Report {
        schema: "badi.stop-token-probe.v1",
        complete,
        cancelled,
        error,
        dispatch_budget_ms: DISPATCH_BUDGET_MS,
        request_budget_ms: REQUEST_MS,
        planned_targets: 6,
        planned_primers: 6,
        attempted_targets,
        attempted_primers,
        latency_ms: started.elapsed().as_secs_f64() * 1000.0,
        limitations: [
            "mechanism_only_no_language_quality_or_delivery_claim",
            "three_pairs_AB_BA_AB_has_2_to_1_first_arm_imbalance",
            "returned_token_arrays_are_not_native_decode_step_counts",
            "startup_failure_uses_existing_RAII_cleanup_without_a_returned_PID_receipt",
        ],
        groups,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::provenance::FileExpectation;

    fn artifact() -> ModelArtifactOverride {
        ModelArtifactOverride {
            weights: FileExpectation::new(
                PathBuf::from("/missing-stop-probe-model"),
                MODEL_SHA,
                MODEL_BYTES,
            )
            .unwrap(),
            alias: "probe-fixture".into(),
        }
    }

    #[tokio::test]
    async fn cancellation_retains_all_six_planned_rows_without_activation_or_generation() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        let mut events = Vec::new();
        let report = run(
            PathBuf::from("/missing-stop-probe"),
            artifact(),
            cancel,
            |value| {
                events.push(value.clone());
                Ok(())
            },
        )
        .await;
        assert!(!report.complete && report.cancelled);
        assert_eq!(report.error, Some("cancelled"));
        assert_eq!(report.groups.len(), 6);
        assert_eq!(report.attempted_targets, 0);
        assert_eq!(report.attempted_primers, 0);
        assert!(
            report
                .groups
                .iter()
                .all(|g| !g.attempted && g.cleanup.is_none())
        );
        assert_eq!(events.len(), 1);
        let orders: Vec<_> = report.groups.iter().map(|g| json!(g.arm)).collect();
        assert_eq!(
            orders,
            json!([
                "space_eos",
                "stop_token",
                "stop_token",
                "space_eos",
                "space_eos",
                "stop_token"
            ])
            .as_array()
            .unwrap()
            .clone()
        );
        for pair in report.groups.chunks_exact(2) {
            assert_eq!(pair[0].primer.payload, pair[1].primer.payload);
            assert_eq!(pair[0].word.text(), pair[1].word.text());
        }
    }

    #[tokio::test]
    async fn wrong_model_and_failed_plan_journal_never_activate() {
        let mut wrong = artifact();
        wrong.weights =
            FileExpectation::new(PathBuf::from("/missing"), "a".repeat(64), MODEL_BYTES).unwrap();
        let report = run(
            PathBuf::from("/missing"),
            wrong,
            CancellationToken::new(),
            |_| panic!("must not emit unbound plan"),
        )
        .await;
        assert_eq!(report.error, Some("stop_probe_model_mismatch"));
        let report = run(
            PathBuf::from("/missing"),
            artifact(),
            CancellationToken::new(),
            |_| Err(()),
        )
        .await;
        assert_eq!(report.error, Some("output_closed"));
        assert!(report.groups.iter().all(|g| !g.attempted));
    }
}
