use std::fmt;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use rustix::process::{Pid, Signal, kill_process_group};
use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::protocol::ProviderKind;
use crate::provider::{
    CompletionProvider, ProviderError, ProviderOutcome, ProviderRequest, RequestTrigger,
    WritingProposal,
};

use super::client::{
    ClientError, HealthStatus, RuntimeAddress, SemanticClient, SemanticClientConfig,
};
use super::provenance::{
    ProvenanceError, VerifiedDirectoryManifest, VerifiedFile, encode_lower_hex,
};
use super::wire::StatusCode;

pub const CONTEXT_SIZE: u16 = 512;
pub const GPU_LAYERS: u16 = 0;
const MAX_WRITING_CONTEXT_SIZE: u16 = 8_192;
const MAX_CONTEXT_CHECKPOINTS: u16 = 64;
const MAX_LAUNCH_CONTRACT_BYTES: usize = 128;
const MAX_MODEL_ORIGIN_BYTES: usize = 64;
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(20);
/// Bound for the single warm-up completion after readiness.
pub const WARM_UP_TIMEOUT: Duration = Duration::from_secs(2);
// A cancelled cold prefill may still be unwinding inside the runtime. Allow
// that work and its HTTP readers to stop before the process-group kill fallback.
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const FORCE_SHUTDOWN_WAIT: Duration = Duration::from_secs(1);
const FIXTURE_ARGUMENT: &str = "__fixture-backend";
const EXIT_POLL_FALLBACK_INTERVAL: Duration = Duration::from_secs(1);
pub const FIXTURE_TOKEN_CANARY: &str =
    "e7a36b6a81bc4d0eb1d73a86f79959c9f588143cb5044d35a187988428cfd32f";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixtureBehavior {
    Ready,
    MalformedHealth,
    NoBind,
    EarlyExit,
}

impl FixtureBehavior {
    #[must_use]
    pub const fn environment_value(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::MalformedHealth => "malformed_health",
            Self::NoBind => "no_bind",
            Self::EarlyExit => "early_exit",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Launcher {
    Direct,
    ParentDeathHelper,
}

/// The settings of a writing launch. The runtime identity records every field
/// except the checkpoint limit, so a profile that differs from production in
/// any way must name its own launch contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WritingProfile {
    /// Names the whole launch contract in the runtime identity.
    pub launch_contract_id: &'static str,
    /// Context window in tokens.
    pub context_size: u16,
    /// Prompt-processing batch and micro-batch in tokens. The runtime
    /// observes cancellation only between batches.
    pub batch_size: u16,
    /// Limit on the runtime's context checkpoints; `None` keeps its default.
    pub context_checkpoints: Option<u16>,
    /// Map model weights instead of copying them into owned memory.
    pub memory_map: bool,
    /// Where the model came from when it is not the installed catalog model.
    pub model_origin: Option<&'static str>,
}

impl WritingProfile {
    /// The installed writing provider's launch.
    pub const PRODUCTION: Self = Self {
        launch_contract_id: crate::writing::WRITING_CONTRACT,
        context_size: CONTEXT_SIZE,
        batch_size: 16,
        context_checkpoints: None,
        // Copy once: CPU repacking otherwise retains both mapped and owned
        // weights. The independent loading comparison preserved every output.
        memory_map: false,
        model_origin: None,
    };

    fn validate(&self) -> Result<(), RuntimeError> {
        let contract = self.launch_contract_id;
        if contract.is_empty()
            || contract.len() > MAX_LAUNCH_CONTRACT_BYTES
            || !contract.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(RuntimeError::InvalidConfig("launch_contract_id"));
        }
        if self.context_size == 0 || self.context_size > MAX_WRITING_CONTEXT_SIZE {
            return Err(RuntimeError::InvalidConfig("context_size"));
        }
        if self.batch_size == 0 || self.batch_size > self.context_size {
            return Err(RuntimeError::InvalidConfig("batch_size"));
        }
        if self
            .context_checkpoints
            .is_some_and(|checkpoints| checkpoints > MAX_CONTEXT_CHECKPOINTS)
        {
            return Err(RuntimeError::InvalidConfig("context_checkpoints"));
        }
        if self.model_origin.is_some_and(|origin| {
            origin.is_empty()
                || origin.len() > MAX_MODEL_ORIGIN_BYTES
                || !origin
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(RuntimeError::InvalidConfig("model_origin"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct LlamaCppLaunch {
    binary: VerifiedFile,
    runtime_bundle: Option<VerifiedDirectoryManifest>,
    model: VerifiedFile,
    model_alias: String,
    startup_timeout: Duration,
    threads: usize,
    fixture_behavior: Option<FixtureBehavior>,
    profile: WritingProfile,
    launcher: Launcher,
    /// Private directory that will hold `llama-<pid>.sock`. `None` uses
    /// `$XDG_RUNTIME_DIR/badi`, which the service unit creates mode 0700.
    runtime_directory: Option<PathBuf>,
}

impl LlamaCppLaunch {
    pub fn new(
        binary: VerifiedFile,
        runtime_bundle: VerifiedDirectoryManifest,
        model: VerifiedFile,
        model_alias: impl Into<String>,
        threads: usize,
    ) -> Result<Self, RuntimeError> {
        let launch = Self {
            binary,
            runtime_bundle: Some(runtime_bundle),
            model,
            model_alias: model_alias.into(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            threads,
            fixture_behavior: None,
            profile: WritingProfile::PRODUCTION,
            launcher: Launcher::Direct,
            runtime_directory: None,
        };
        launch.validate()?;
        Ok(launch)
    }

    /// Tests place the socket in a private directory. Production leaves this
    /// unset and uses the service runtime directory.
    #[must_use]
    pub fn with_runtime_directory(mut self, directory: PathBuf) -> Self {
        self.runtime_directory = Some(directory);
        self
    }

    pub fn with_startup_timeout(mut self, startup_timeout: Duration) -> Result<Self, RuntimeError> {
        self.startup_timeout = startup_timeout;
        self.validate()?;
        Ok(self)
    }

    pub fn for_fixture(
        binary: VerifiedFile,
        model: VerifiedFile,
        behavior: FixtureBehavior,
    ) -> Result<Self, RuntimeError> {
        let launch = Self {
            binary,
            runtime_bundle: None,
            model,
            model_alias: "fixture-en-v1".to_owned(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            threads: 1,
            fixture_behavior: Some(behavior),
            profile: WritingProfile::PRODUCTION,
            launcher: Launcher::Direct,
            runtime_directory: None,
        };
        launch.validate()?;
        Ok(launch)
    }

    pub async fn spawn(self) -> Result<OwnedRuntime, RuntimeError> {
        self.spawn_with_post_spawn_hook(|_| {}).await
    }

    /// Launches with `profile` instead of [`WritingProfile::PRODUCTION`].
    pub fn with_writing_profile(mut self, profile: WritingProfile) -> Result<Self, RuntimeError> {
        self.profile = profile;
        self.validate()?;
        Ok(self)
    }

    /// Launches through this executable's parent-death helper, so the runtime
    /// cannot outlive it. The executable must dispatch the helper's flag
    /// before it starts Tokio or any other thread. Linux only.
    #[must_use]
    pub const fn contained(mut self) -> Self {
        self.launcher = Launcher::ParentDeathHelper;
        self
    }

    async fn spawn_with_post_spawn_hook(
        self,
        post_spawn_hook: impl FnOnce(u32),
    ) -> Result<OwnedRuntime, RuntimeError> {
        self.validate()?;

        let socket_path = prepare_runtime_socket(&self.runtime_directory_path()?)?;
        let token = if self.fixture_behavior.is_some() {
            SecretToken::fixture()
        } else {
            SecretToken::new()
        };
        let negative_token = SecretToken::new();
        let client = SemanticClient::new(SemanticClientConfig::unix(
            &socket_path,
            &self.model_alias,
            token.expose(),
        )?)?;
        let negative_client = SemanticClient::new(SemanticClientConfig::unix(
            &socket_path,
            &self.model_alias,
            negative_token.expose(),
        )?)?;
        let identity = self.identity();
        let mut command = self.runtime_command()?;
        if self.fixture_behavior.is_some() {
            command.arg(FIXTURE_ARGUMENT);
        }
        command
            .env_clear()
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("LLAMA_ARG_HOST", &socket_path)
            .env("LLAMA_ARG_MODEL", self.model.path())
            .env("LLAMA_ARG_ALIAS", &self.model_alias)
            .env("LLAMA_API_KEY", token.expose())
            // Two slots split the 512-token context into 256. One slot keeps it.
            .env("LLAMA_ARG_N_PARALLEL", "1")
            .env("LLAMA_ARG_THREADS", self.threads.to_string())
            .env("LLAMA_ARG_THREADS_BATCH", self.threads.to_string())
            .env("LLAMA_ARG_N_GPU_LAYERS", GPU_LAYERS.to_string())
            .env("LLAMA_ARG_UI", "0")
            .env("LLAMA_ARG_OFFLINE", "1")
            .env("LLAMA_ARG_LOG_DISABLE", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(
                self.model
                    .path()
                    .parent()
                    .ok_or(RuntimeError::InvalidConfig("model_parent"))?,
            )
            .process_group(0);
        self.configure_mode(&mut command);
        self.confirm_artifacts_unchanged()?;
        let child = command.spawn().map_err(RuntimeError::Spawn)?;
        // Only this handle reaps the child, so the PID still names it here.
        #[cfg(target_os = "linux")]
        let exit_notice = rustix::process::pidfd_open(
            Pid::from_child(&child),
            rustix::process::PidfdFlags::empty(),
        )
        .ok();
        #[cfg(not(target_os = "linux"))]
        let exit_notice = None;
        let mut runtime = OwnedRuntime {
            child: Mutex::new(Some(child)),
            exit_notice,
            client,
            endpoint: RuntimeAddress::Unix(socket_path),
            token_credential: token,
            identity,
        };
        post_spawn_hook(runtime.process_id().ok_or(RuntimeError::MissingChild)?);
        if let Err(error) = self.confirm_artifacts_unchanged() {
            let _ = runtime.terminate();
            return Err(error.into());
        }
        if let Err(error) = runtime
            .wait_until_ready(self.startup_timeout, &negative_client)
            .await
        {
            let _ = runtime.terminate();
            return Err(error);
        }
        if let Err(error) = self.confirm_artifacts_unchanged() {
            let _ = runtime.terminate();
            return Err(error.into());
        }
        if let RuntimeAddress::Unix(path) = &runtime.endpoint {
            if let Err(error) = tighten_runtime_socket(path) {
                let _ = runtime.terminate();
                return Err(error);
            }
        }
        Ok(runtime)
    }

    fn runtime_directory_path(&self) -> Result<PathBuf, RuntimeError> {
        if let Some(directory) = &self.runtime_directory {
            return Ok(directory.clone());
        }
        let runtime = std::env::var_os("XDG_RUNTIME_DIR")
            .ok_or(RuntimeError::InvalidConfig("runtime_dir"))?;
        let runtime = PathBuf::from(runtime);
        if !runtime.is_absolute() {
            return Err(RuntimeError::InvalidConfig("runtime_dir"));
        }
        Ok(runtime.join("badi"))
    }

    fn runtime_command(&self) -> Result<Command, RuntimeError> {
        if self.launcher == Launcher::ParentDeathHelper {
            #[cfg(target_os = "linux")]
            return super::process::launch_command(self.binary.path()).map_err(RuntimeError::Spawn);
            #[cfg(not(target_os = "linux"))]
            return Err(RuntimeError::InvalidConfig("parent_death_unavailable"));
        }
        Ok(Command::new(self.binary.path()))
    }

    /// The identity this launch's runtime will report.
    #[must_use]
    pub fn identity(&self) -> StableRuntimeIdentity {
        let profile = self.profile;
        StableRuntimeIdentity {
            launch_contract_id: profile.launch_contract_id,
            binary_sha256: self.binary.sha256().to_owned(),
            runtime_bundle_manifest_sha256: self
                .runtime_bundle
                .as_ref()
                .map(|bundle| bundle.sha256().to_owned()),
            model_sha256: self.model.sha256().to_owned(),
            model_size: self.model.identity().size,
            model_alias: self.model_alias.clone(),
            model_origin: profile.model_origin,
            threads: self.threads,
            context_size: profile.context_size,
            gpu_layers: GPU_LAYERS,
            batch_size: Some(profile.batch_size),
            ubatch_size: Some(profile.batch_size),
            memory_map: profile.memory_map,
        }
    }

    fn configure_mode(&self, command: &mut Command) {
        if let Some(behavior) = self.fixture_behavior {
            command.env("BADI_FIXTURE_BEHAVIOR", behavior.environment_value());
        }
        let profile = self.profile;
        // The host cache keeps the previous sentence when the prompt anchor
        // moves. A matching run of at least 16 tokens can be shifted instead
        // of recomputed. No slot-save path or disk cache.
        command
            .env("LLAMA_ARG_CTX_SIZE", profile.context_size.to_string())
            .env("LLAMA_ARG_CACHE_PROMPT", "1")
            .env("LLAMA_ARG_CACHE_RAM", "128")
            .env("LLAMA_ARG_CACHE_REUSE", "16")
            // The runtime observes cancellation between prefill batches, so
            // the batch bounds the cost of a cancelled request.
            .env("LLAMA_ARG_BATCH", profile.batch_size.to_string())
            .env("LLAMA_ARG_UBATCH", profile.batch_size.to_string());
        command.env("LLAMA_ARG_MMAP", if profile.memory_map { "1" } else { "0" });
        if let Some(checkpoints) = profile.context_checkpoints {
            command.env("LLAMA_ARG_CTX_CHECKPOINTS", checkpoints.to_string());
        }
    }

    fn validate(&self) -> Result<(), RuntimeError> {
        if self.model_alias.is_empty()
            || self.model_alias.len() > 256
            || self.model_alias.chars().any(char::is_control)
        {
            return Err(RuntimeError::InvalidConfig("model_alias"));
        }
        if self.threads == 0 || self.threads > 256 {
            return Err(RuntimeError::InvalidConfig("threads"));
        }
        if self.startup_timeout.is_zero() || self.startup_timeout > MAX_STARTUP_TIMEOUT {
            return Err(RuntimeError::InvalidConfig("startup_timeout"));
        }
        self.profile.validate()?;
        match (&self.runtime_bundle, self.fixture_behavior) {
            (Some(bundle), None) if self.binary.path().parent() == Some(bundle.path()) => {}
            // The parent-death helper passes no fixture arguments.
            (None, Some(_)) if self.launcher == Launcher::Direct => {}
            _ => return Err(RuntimeError::InvalidConfig("runtime_bundle")),
        }
        Ok(())
    }

    /// Checkpoints around spawn and readiness. The caller hashed every artifact
    /// once; these confirm the same filesystem objects without re-reading them.
    fn confirm_artifacts_unchanged(&self) -> Result<(), ProvenanceError> {
        if let Some(bundle) = &self.runtime_bundle {
            bundle.confirm_unchanged()?;
        }
        self.binary.confirm_unchanged()?;
        self.model.confirm_unchanged()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StableRuntimeIdentity {
    pub launch_contract_id: &'static str,
    pub binary_sha256: String,
    pub runtime_bundle_manifest_sha256: Option<String>,
    pub model_sha256: String,
    pub model_size: u64,
    pub model_alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_origin: Option<&'static str>,
    pub threads: usize,
    pub context_size: u16,
    pub gpu_layers: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ubatch_size: Option<u16>,
    /// Absence in older serialized identities means the pinned mapped default.
    #[serde(skip_serializing_if = "memory_mapping_enabled")]
    pub memory_map: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)] // Serde's required signature.
fn memory_mapping_enabled(enabled: &bool) -> bool {
    *enabled
}

pub struct OwnedRuntime {
    child: Mutex<Option<Child>>,
    /// Pidfd of the owned child; it becomes readable once the child exits.
    exit_notice: Option<OwnedFd>,
    client: SemanticClient,
    endpoint: RuntimeAddress,
    #[allow(dead_code)]
    token_credential: SecretToken,
    identity: StableRuntimeIdentity,
}

impl OwnedRuntime {
    #[must_use]
    pub const fn client(&self) -> &SemanticClient {
        &self.client
    }

    #[must_use]
    pub const fn address(&self) -> &RuntimeAddress {
        &self.endpoint
    }

    #[must_use]
    pub const fn identity(&self) -> &StableRuntimeIdentity {
        &self.identity
    }

    #[must_use]
    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().ok()?.as_ref().map(Child::id)
    }

    fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        // Poll only the owned handle. Once reaped, Child remembers its status;
        // cleanup cannot mistake a subsequently reused PID for this runtime.
        self.child
            .lock()
            .map_err(|_| io::Error::other("owned child lock poisoned"))?
            .as_mut()
            .ok_or_else(|| io::Error::other("owned child missing"))?
            .try_wait()
    }

    /// Send one bounded warm-up completion. A failure is reported, never
    /// raised: the runtime stays usable and only its first request is cold.
    pub async fn warm_up(&self) -> WarmUpReport {
        self.warm_up_within(WARM_UP_TIMEOUT).await
    }

    async fn warm_up_within(&self, timeout: Duration) -> WarmUpReport {
        let started = Instant::now();
        let failure = self
            .client
            .warm_up(timeout, CancellationToken::new())
            .await
            .err()
            .map(|error| error.class());
        WarmUpReport::new(started.elapsed(), failure)
    }

    pub fn shutdown(mut self) -> Result<RuntimeLifecycleObservation, RuntimeError> {
        let process_id = self.process_id().ok_or(RuntimeError::MissingChild)?;
        let status = self.terminate()?;
        Ok(RuntimeLifecycleObservation {
            process_id,
            runtime_identity_sha256: self.identity.sha256(),
            challenge_completed: true,
            reaped: true,
            exit_code: status.code(),
        })
    }

    async fn wait_until_ready(
        &mut self,
        timeout: Duration,
        negative_client: &SemanticClient,
    ) -> Result<(), RuntimeError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.try_wait().map_err(RuntimeError::Wait)? {
                return Err(RuntimeError::EarlyExit(status.code()));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(RuntimeError::StartupTimeout);
            }
            let probe = self.client.probe_health(CancellationToken::new());
            match tokio::time::timeout(remaining, probe).await {
                Ok(Ok(HealthStatus::Ready)) => {
                    self.client
                        .probe_authorization_challenge(CancellationToken::new())
                        .await
                        .map_err(RuntimeError::Health)?;
                    match negative_client
                        .probe_authorization_challenge(CancellationToken::new())
                        .await
                    {
                        Err(ClientError::UnexpectedStatus(StatusCode::UNAUTHORIZED)) => {
                            return Ok(());
                        }
                        Ok(()) => return Err(RuntimeError::AuthenticationNotEnforced),
                        Err(error) => return Err(RuntimeError::NegativeChallenge(error)),
                    }
                }
                Ok(Ok(HealthStatus::Loading)) => {}
                Ok(Err(error)) if error.retryable_during_startup() => {}
                Ok(Err(error)) => return Err(RuntimeError::Health(error)),
                Err(_) => return Err(RuntimeError::StartupTimeout),
            }
            tokio::time::sleep(
                HEALTH_POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now())),
            )
            .await;
        }
    }

    fn terminate(&mut self) -> Result<ExitStatus, RuntimeError> {
        let child = self
            .child
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if child.is_none() {
            return Err(RuntimeError::MissingChild);
        }
        let status =
            terminate_owned_child_with(child, terminate_child).map_err(RuntimeError::Shutdown);
        if let RuntimeAddress::Unix(path) = &self.endpoint {
            let _ = std::fs::remove_file(path);
        }
        status
    }
}

#[async_trait]
impl CompletionProvider for OwnedRuntime {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    fn is_alive(&self) -> bool {
        matches!(self.try_wait(), Ok(None))
    }

    async fn exited(&self) {
        let notice = self
            .exit_notice
            .as_ref()
            .and_then(|notice| notice.try_clone().ok())
            .and_then(|notice| AsyncFd::with_interest(notice, Interest::READABLE).ok());
        if let Some(notice) = notice {
            if notice.readable().await.is_ok() {
                return;
            }
        }
        // Without a usable pidfd, observe the owned handle at a coarse interval.
        while self.is_alive() {
            tokio::time::sleep(EXIT_POLL_FALLBACK_INTERVAL).await;
        }
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.client.complete(request, cancellation).await
    }

    async fn propose(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
    ) -> Result<Option<WritingProposal>, ProviderError> {
        self.client
            .propose(request, cancellation, allow_replacement)
            .await
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        trigger: RequestTrigger,
    ) -> Result<ProviderOutcome, ProviderError> {
        self.client
            .propose_outcome(request, cancellation, allow_replacement, trigger)
            .await
    }

    async fn propose_outcome_with_partials(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        trigger: RequestTrigger,
        partials: Option<tokio::sync::mpsc::UnboundedSender<String>>,
    ) -> Result<ProviderOutcome, ProviderError> {
        self.client
            .propose_outcome_with_partials(
                request,
                cancellation,
                allow_replacement,
                trigger,
                partials,
            )
            .await
    }
}

impl StableRuntimeIdentity {
    #[must_use]
    pub fn sha256(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("runtime identity is serializable");
        encode_lower_hex(Sha256::digest(canonical))
    }
}

/// Content-free result of [`OwnedRuntime::warm_up`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WarmUpReport {
    elapsed: Duration,
    failure: Option<&'static str>,
}

impl WarmUpReport {
    pub(crate) const fn new(elapsed: Duration, failure: Option<&'static str>) -> Self {
        Self { elapsed, failure }
    }

    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// The failure class, or `None` when the completion finished.
    #[must_use]
    pub const fn failure(&self) -> Option<&'static str> {
        self.failure
    }
}

impl fmt::Display for WarmUpReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.failure {
            None => formatter.write_str("completed")?,
            Some(class) => write!(formatter, "failed class={class}")?,
        }
        write!(formatter, " elapsed_ms={}", self.elapsed.as_millis())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RuntimeLifecycleObservation {
    process_id: u32,
    runtime_identity_sha256: String,
    challenge_completed: bool,
    reaped: bool,
    exit_code: Option<i32>,
}

impl RuntimeLifecycleObservation {
    #[must_use]
    pub const fn process_id(&self) -> u32 {
        self.process_id
    }

    #[must_use]
    pub fn runtime_identity_sha256(&self) -> &str {
        &self.runtime_identity_sha256
    }

    #[must_use]
    pub const fn challenge_completed(&self) -> bool {
        self.challenge_completed
    }

    #[must_use]
    pub const fn reaped(&self) -> bool {
        self.reaped
    }

    #[must_use]
    pub const fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }
}

impl fmt::Debug for OwnedRuntime {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedRuntime")
            .field("process_id", &self.process_id())
            .field("endpoint", &self.endpoint)
            .field("token", &"[redacted]")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl Drop for OwnedRuntime {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("invalid owned-runtime configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("runtime artifact provenance failed")]
    Provenance(#[from] ProvenanceError),
    #[error("runtime client initialization failed")]
    Client(#[from] ClientError),
    #[error("failed to prepare the private runtime socket")]
    Endpoint(#[source] io::Error),
    #[error("failed to spawn the verified runtime binary")]
    Spawn(#[source] io::Error),
    #[error("runtime health challenge failed")]
    Health(#[source] ClientError),
    #[error("runtime accepted a deliberately incorrect authorization challenge")]
    AuthenticationNotEnforced,
    #[error("runtime negative authorization challenge failed unexpectedly")]
    NegativeChallenge(#[source] ClientError),
    #[error("runtime exited during startup with status {0:?}")]
    EarlyExit(Option<i32>),
    #[error("runtime did not become ready before the bounded startup deadline")]
    StartupTimeout,
    #[error("runtime child status check failed")]
    Wait(#[source] io::Error),
    #[error("owned runtime child is missing")]
    MissingChild,
    #[error("owned runtime shutdown failed")]
    Shutdown(#[source] io::Error),
}

#[derive(Clone)]
struct SecretToken(String);

impl SecretToken {
    fn new() -> Self {
        Self(format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ))
    }

    fn expose(&self) -> &str {
        &self.0
    }

    fn fixture() -> Self {
        Self(FIXTURE_TOKEN_CANARY.to_owned())
    }
}

impl fmt::Debug for SecretToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

fn prepare_runtime_socket(directory: &Path) -> Result<PathBuf, RuntimeError> {
    if !directory.is_absolute() {
        return Err(RuntimeError::InvalidConfig("runtime_dir"));
    }
    let metadata = match std::fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            std::fs::create_dir(directory).map_err(RuntimeError::Endpoint)?;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))
                .map_err(RuntimeError::Endpoint)?;
            std::fs::symlink_metadata(directory).map_err(RuntimeError::Endpoint)?
        }
        Err(error) => return Err(RuntimeError::Endpoint(error)),
    };
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(RuntimeError::InvalidConfig("runtime_dir"));
    }
    // A fixed name would let a test, or a second broker, unlink the socket the
    // running service is still using. Each process owns only its own name.
    retire_dead_runtime_sockets(directory)?;
    let path = directory.join(format!("llama-{}.sock", std::process::id()));
    if path.as_os_str().len() >= 108 {
        return Err(RuntimeError::InvalidConfig("runtime_socket"));
    }
    if let Ok(existing) = std::fs::symlink_metadata(&path) {
        if existing.file_type().is_socket() && existing.uid() == rustix::process::getuid().as_raw()
        {
            std::fs::remove_file(&path).map_err(RuntimeError::Endpoint)?;
        } else {
            return Err(RuntimeError::InvalidConfig("runtime_socket"));
        }
    }
    Ok(path)
}

fn retire_dead_runtime_sockets(directory: &Path) -> Result<(), RuntimeError> {
    let mine = std::process::id();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(RuntimeError::Endpoint(error)),
    };
    for entry in entries {
        let entry = entry.map_err(RuntimeError::Endpoint)?;
        let Some(name) = entry.file_name().into_string().ok() else {
            continue;
        };
        let Some(pid) = name
            .strip_prefix("llama-")
            .and_then(|rest| rest.strip_suffix(".sock"))
            .and_then(|pid| pid.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == mine || Path::new(&format!("/proc/{pid}")).exists() {
            continue;
        }
        let metadata = match std::fs::symlink_metadata(entry.path()) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(RuntimeError::Endpoint(error)),
        };
        if metadata.file_type().is_socket() && metadata.uid() == rustix::process::getuid().as_raw()
        {
            std::fs::remove_file(entry.path()).map_err(RuntimeError::Endpoint)?;
        }
    }
    Ok(())
}

fn tighten_runtime_socket(path: &Path) -> Result<(), RuntimeError> {
    let metadata = std::fs::symlink_metadata(path).map_err(RuntimeError::Endpoint)?;
    if !metadata.file_type().is_socket() || metadata.uid() != rustix::process::getuid().as_raw() {
        return Err(RuntimeError::InvalidConfig("runtime_socket"));
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(RuntimeError::Endpoint)
}

fn terminate_child(child: &mut Child) -> io::Result<ExitStatus> {
    if let Some(status) = child.try_wait()? {
        return Ok(status);
    }
    let process_group = Pid::from_child(child);
    let _ = kill_process_group(process_group, Signal::TERM);
    if let Some(status) = wait_bounded(child, GRACEFUL_SHUTDOWN_WAIT)? {
        return Ok(status);
    }
    let _ = kill_process_group(process_group, Signal::KILL);
    if let Some(status) = wait_bounded(child, FORCE_SHUTDOWN_WAIT)? {
        return Ok(status);
    }
    child.kill()?;
    child.wait()
}

fn terminate_owned_child_with(
    child: &mut Option<Child>,
    terminate: impl FnOnce(&mut Child) -> io::Result<ExitStatus>,
) -> io::Result<ExitStatus> {
    let result = terminate(child.as_mut().expect("child presence checked by caller"));
    if result.is_ok() {
        *child = None;
    }
    result
}

fn wait_bounded(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(test)]
mod tests;
