//! The on-demand revision runtime (ADR 0012): the catalog's instruction
//! model in its own pinned llama.cpp process and socket, started by the
//! first request and stopped after ten idle minutes. Its failure or exit
//! never stops suggestions; the next request starts it again.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::Deserialize;
use tokio::sync::{Mutex, oneshot};
use tokio_util::sync::CancellationToken;

use super::service::{Reviser, RevisionCall, RevisionError, RevisionReply};
use super::{chatml_prompt, messages};
use crate::model_selection::{
    ModelArtifact, ModelUseCase, catalog, current_memory, required_memory_mib,
};
use crate::protocol::ProviderKind;
use crate::provider::CompletionProvider as _;
use crate::semantic::runtime::{LlamaCppLaunch, OwnedRuntime, RuntimeError, WritingProfile};
use crate::semantic::wire::{StatusCode, read_bounded_body};

/// The catalog model revisions use: Qwen3 1.7B (Apache-2.0) follows a
/// proofreading instruction in Badi's five languages.
pub const REVISION_MODEL_ID: &str = "qwen3-1.7b";
/// A runtime nobody asked for this long is stopped, returning its memory.
pub const IDLE_STOP: Duration = Duration::from_secs(600);
/// The runtime's reply echoes its settings beside the answer.
const MAX_REPLY_BYTES: usize = 64 * 1_024;

/// The revision runtime's launch: room for the prompt, the longest
/// selection and its answer, and a batch that reads a selection quickly.
pub const PROFILE: WritingProfile = WritingProfile {
    launch_contract_id: "badi-revision-qwen3-v1",
    context_size: 1_024,
    batch_size: 64,
    context_checkpoints: None,
    memory_map: false,
    full_swa_cache: false,
    model_origin: None,
};

/// The revision model of the catalog.
#[must_use]
pub fn revision_model() -> Option<ModelArtifact> {
    catalog(ModelUseCase::Writing)
        .iter()
        .copied()
        .find(|model| model.id == REVISION_MODEL_ID)
}

/// The token budget for a selection of `scalars`: a refinement may be
/// longer than its selection, and a token is about three letters.
#[must_use]
pub fn token_budget(scalars: usize) -> u16 {
    let budget = (scalars * 2 / 3 + 48).clamp(64, 512);
    u16::try_from(budget).unwrap_or(512)
}

type Launched = Result<OwnedRuntime, RuntimeError>;

struct Slot {
    /// Verified once; a later start only confirms the same files.
    launch: Option<LlamaCppLaunch>,
    running: Option<Arc<OwnedRuntime>>,
    last_used: Instant,
}

/// Revisions from the installed revision model under `directory`.
pub struct LocalReviser {
    directory: PathBuf,
    model: ModelArtifact,
    contained: bool,
    /// Every runtime is started on one thread that lives as long as the
    /// broker: a contained runtime is killed when its starting thread exits.
    launches: std::sync::mpsc::Sender<(LlamaCppLaunch, oneshot::Sender<Launched>)>,
    slot: Arc<Mutex<Slot>>,
}

impl LocalReviser {
    /// A reviser that starts its runtime on first use. `contained` launches
    /// through the broker's parent-death helper (`badi-broker` only).
    pub fn new(directory: PathBuf, contained: bool) -> std::io::Result<Self> {
        let model = revision_model().ok_or_else(|| std::io::Error::other("revision model"))?;
        let (launches, requests) =
            std::sync::mpsc::channel::<(LlamaCppLaunch, oneshot::Sender<Launched>)>();
        std::thread::Builder::new()
            .name("badi-revision".to_owned())
            .spawn(move || {
                let Ok(executor) = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                else {
                    return;
                };
                while let Ok((launch, reply)) = requests.recv() {
                    // A start nobody waits for any more is dropped, and its
                    // drop stops the process.
                    let _ = reply.send(executor.block_on(launch.spawn()));
                }
            })?;
        Ok(Self {
            directory,
            model,
            contained,
            launches,
            slot: Arc::new(Mutex::new(Slot {
                launch: None,
                running: None,
                last_used: Instant::now(),
            })),
        })
    }

    /// Whether the revision model is installed (its file is present).
    #[must_use]
    pub fn installed(&self) -> bool {
        self.directory
            .join("models")
            .join(self.model.filename)
            .is_file()
    }

    /// The running runtime, started now if there is none, and whether this
    /// call started it.
    async fn runtime(&self) -> Result<(Arc<OwnedRuntime>, bool), RevisionError> {
        let mut slot = self.slot.lock().await;
        if let Some(running) = slot.running.take() {
            if running.is_alive() {
                slot.running = Some(Arc::clone(&running));
                return Ok((running, false));
            }
            eprintln!("badi-broker: revision runtime exited; starting it again");
            stop_in_background(running);
        }
        if !self.installed() {
            return Err(RevisionError::Unavailable);
        }
        // The service's own memory limit counts too: the writing runtime
        // shares it, and crossing it would end that runtime.
        let headroom =
            cgroup_headroom_mib(Path::new("/proc/self/cgroup"), Path::new("/sys/fs/cgroup"));
        if !crate::writing::memory_fits(self.model, current_memory().available_mib)
            || headroom.is_some_and(|headroom| headroom < required_memory_mib(self.model))
        {
            eprintln!("badi-broker: not enough available memory for the revision model");
            return Err(RevisionError::Unavailable);
        }
        let launch = if let Some(launch) = slot.launch.clone() {
            launch
        } else {
            let launch = self.verified_launch().await?;
            slot.launch = Some(launch.clone());
            launch
        };
        let (reply, started) = oneshot::channel();
        self.launches
            .send((launch, reply))
            .map_err(|_| RevisionError::Failed)?;
        let runtime = started.await.map_err(|_| RevisionError::Failed)?;
        let runtime = Arc::new(runtime.map_err(|error| {
            eprintln!("badi-broker: revision runtime did not start: {error}");
            // A changed file must be verified again before the next start.
            slot.launch = None;
            RevisionError::Failed
        })?);
        slot.running = Some(Arc::clone(&runtime));
        Ok((runtime, true))
    }

    /// Hashes the installed revision model and the pinned runtime, once.
    async fn verified_launch(&self) -> Result<LlamaCppLaunch, RevisionError> {
        let threads = crate::writing::writing_threads().map_err(|_| RevisionError::Unavailable)?;
        let launch = crate::writing::verify_launch(self.directory.clone(), self.model, threads)
            .await
            .map_err(|error| {
                eprintln!("badi-broker: revision model not verified: {error}");
                RevisionError::Unavailable
            })?
            .with_writing_profile(PROFILE)
            .map_err(|_| RevisionError::Failed)?
            .for_revision();
        Ok(if self.contained {
            launch.contained()
        } else {
            launch
        })
    }

    /// Records a use and stops the runtime once it has been idle for
    /// [`IDLE_STOP`] with no request in flight.
    async fn touch(&self) {
        let used = Instant::now();
        self.slot.lock().await.last_used = used;
        let slot = Arc::clone(&self.slot);
        tokio::spawn(async move {
            tokio::time::sleep(IDLE_STOP).await;
            let mut slot = slot.lock().await;
            let idle = slot.last_used <= used
                && slot
                    .running
                    .as_ref()
                    .is_some_and(|running| Arc::strong_count(running) == 1);
            if idle {
                if let Some(running) = slot.running.take() {
                    stop_in_background(running);
                }
            }
        });
    }
}

/// What the process's cgroup (v2) may still hold before its `memory.max`, in
/// MiB: the limit less what it holds that the kernel cannot drop (all but
/// its file cache). `None` without a limit.
fn cgroup_headroom_mib(membership: &Path, root: &Path) -> Option<u64> {
    const MIB: u64 = 1_024 * 1_024;
    let membership = std::fs::read_to_string(membership).ok()?;
    let group = membership
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    let directory = root.join(group.trim().trim_start_matches('/'));
    let read = |name: &str| std::fs::read_to_string(directory.join(name)).ok();
    let limit = read("memory.max")?.trim().parse::<u64>().ok()?;
    let current = read("memory.current")?.trim().parse::<u64>().ok()?;
    let file = read("memory.stat")?
        .lines()
        .find_map(|line| line.strip_prefix("file ")?.trim().parse::<u64>().ok())
        .unwrap_or(0);
    Some(limit.saturating_sub(current.saturating_sub(file)) / MIB)
}

/// Ends a runtime off the async workers: stopping waits for the process.
fn stop_in_background(runtime: Arc<OwnedRuntime>) {
    tokio::task::spawn_blocking(move || drop(runtime));
}

#[derive(Deserialize)]
struct CompletionReply {
    content: String,
    #[serde(default)]
    stop_type: Option<String>,
    #[serde(default)]
    tokens_predicted: Option<u32>,
}

#[async_trait]
impl Reviser for LocalReviser {
    fn provider(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    async fn revise(
        &self,
        call: RevisionCall<'_>,
        cancellation: CancellationToken,
    ) -> Result<RevisionReply, RevisionError> {
        let deadline = tokio::time::Instant::now() + call.timeout;
        let (runtime, cold) = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(RevisionError::Cancelled),
            started = tokio::time::timeout_at(deadline, self.runtime()) => {
                started.map_err(|_| RevisionError::Timeout)??
            }
        };
        let reply = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(RevisionError::Cancelled),
            reply = complete(
                &runtime,
                call,
                deadline.saturating_duration_since(tokio::time::Instant::now()),
            ) => reply,
        };
        drop(runtime);
        self.touch().await;
        Ok(RevisionReply { cold, ..reply? })
    }
}

/// Asks `runtime` for one revision of `call` within `timeout`: the request
/// the broker sends, which the Prediction Lab measures through this same
/// function.
pub async fn complete(
    runtime: &OwnedRuntime,
    call: RevisionCall<'_>,
    timeout: Duration,
) -> Result<RevisionReply, RevisionError> {
    let prompt = chatml_prompt(&messages(
        call.mode,
        call.language,
        call.before,
        call.selected,
    ));
    let body = serde_json::to_vec(&serde_json::json!({
        "prompt": prompt,
        "n_predict": token_budget(call.selected.chars().count()),
        "temperature": 0.0,
        "top_k": 1,
        "seed": 1,
        "stop": ["<|im_end|>", "<|endoftext|>"],
        "cache_prompt": true,
        "stream": false,
    }))
    .map_err(|_| RevisionError::Failed)?;
    let exchange = async {
        let response = runtime
            .client()
            .post_runtime("/completion", body, Some(timeout))
            .await
            .map_err(|_| RevisionError::Failed)?;
        if response.status() != StatusCode::OK {
            return Err(RevisionError::Failed);
        }
        let body = read_bounded_body(response, MAX_REPLY_BYTES)
            .await
            .map_err(|_| RevisionError::Failed)?;
        serde_json::from_slice::<CompletionReply>(&body).map_err(|_| RevisionError::Failed)
    };
    let reply = tokio::time::timeout(timeout, exchange)
        .await
        .unwrap_or(Err(RevisionError::Timeout))?;
    Ok(RevisionReply {
        output: reply.content,
        truncated: reply.stop_type.as_deref() == Some("limit"),
        cold: false,
        generated_tokens: reply.tokens_predicted,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{cgroup_headroom_mib, token_budget};

    #[test]
    fn the_service_limit_counts_what_the_kernel_cannot_drop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let group = directory.path().join("user.slice/badi-broker.service");
        std::fs::create_dir_all(&group).expect("cgroup directory");
        let membership = directory.path().join("cgroup");
        std::fs::write(&membership, "0::/user.slice/badi-broker.service\n").expect("membership");
        let mib = 1_024 * 1_024_u64;
        std::fs::write(group.join("memory.max"), format!("{}\n", 3_746 * mib)).expect("limit");
        std::fs::write(group.join("memory.current"), format!("{}\n", 2_000 * mib))
            .expect("current");
        std::fs::write(
            group.join("memory.stat"),
            format!("anon 1\nfile {}\n", 1_000 * mib),
        )
        .expect("stat");
        assert_eq!(
            cgroup_headroom_mib(&membership, directory.path()),
            Some(2_746)
        );
        std::fs::write(group.join("memory.max"), "max\n").expect("no limit");
        assert_eq!(cgroup_headroom_mib(&membership, directory.path()), None);
        assert_eq!(
            cgroup_headroom_mib(Path::new("/nonexistent"), directory.path()),
            None
        );
    }

    #[test]
    fn a_longer_selection_may_use_more_tokens_within_the_runtime_context() {
        assert_eq!(token_budget(0), 64);
        assert_eq!(token_budget(300), 248);
        assert_eq!(token_budget(600), 448);
        assert_eq!(token_budget(5_000), 512);
    }
}
