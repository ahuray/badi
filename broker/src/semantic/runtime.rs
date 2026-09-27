use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use reqwest::StatusCode;
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

use super::client::{ClientError, HealthStatus, SemanticClient, SemanticClientConfig};
use super::provenance::{ProvenanceError, VerifiedDirectoryManifest, VerifiedFile};

pub const LLAMA_CPP_LAUNCH_CONTRACT_ID: &str = "badi.llama-cpp-owned-eval.v1";
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
    writing: Option<WritingProfile>,
    launcher: Launcher,
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
            writing: None,
            launcher: Launcher::Direct,
        };
        launch.validate()?;
        Ok(launch)
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
            writing: None,
            launcher: Launcher::Direct,
        };
        launch.validate()?;
        Ok(launch)
    }

    pub async fn spawn(self) -> Result<OwnedRuntime, RuntimeError> {
        self.spawn_with_post_spawn_hook(|_| {}).await
    }

    /// Launches the installed writing provider: [`WritingProfile::PRODUCTION`].
    #[must_use]
    pub const fn for_writing(mut self) -> Self {
        self.writing = Some(WritingProfile::PRODUCTION);
        self
    }

    /// Launches for writing with `profile` instead of production's settings.
    pub fn with_writing_profile(mut self, profile: WritingProfile) -> Result<Self, RuntimeError> {
        self.writing = Some(profile);
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

        let endpoint = reserve_loopback_endpoint()?;
        let token = if self.fixture_behavior.is_some() {
            SecretToken::fixture()
        } else {
            SecretToken::new()
        };
        let negative_token = SecretToken::new();
        let config = SemanticClientConfig::new(endpoint, &self.model_alias, token.expose())?;
        let config = if self.writing.is_some() {
            config.for_writing()
        } else {
            config
        };
        let client = SemanticClient::new(config)?;
        let negative_config =
            SemanticClientConfig::new(endpoint, &self.model_alias, negative_token.expose())?;
        let negative_client = SemanticClient::new(negative_config)?;
        let identity = self.identity();
        let mut command = self.runtime_command()?;
        if self.fixture_behavior.is_some() {
            command.arg(FIXTURE_ARGUMENT);
        }
        command
            .env_clear()
            .env("LANG", "C.UTF-8")
            .env("LC_ALL", "C.UTF-8")
            .env("LLAMA_ARG_HOST", Ipv4Addr::LOCALHOST.to_string())
            .env("LLAMA_ARG_PORT", endpoint.port().to_string())
            .env("LLAMA_ARG_MODEL", self.model.path())
            .env("LLAMA_ARG_ALIAS", &self.model_alias)
            .env("LLAMA_API_KEY", token.expose())
            .env("LLAMA_ARG_CTX_SIZE", self.context_size().to_string())
            .env("LLAMA_ARG_N_PARALLEL", "1")
            .env("LLAMA_ARG_THREADS", self.threads.to_string())
            .env("LLAMA_ARG_THREADS_BATCH", self.threads.to_string())
            .env("LLAMA_ARG_N_GPU_LAYERS", GPU_LAYERS.to_string())
            .env("LLAMA_ARG_UI", "0")
            .env("LLAMA_ARG_OFFLINE", "1")
            .env("LLAMA_ARG_CACHE_PROMPT", "0")
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
            endpoint,
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
        Ok(runtime)
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
        let writing = self.writing;
        StableRuntimeIdentity {
            launch_contract_id: writing.map_or(LLAMA_CPP_LAUNCH_CONTRACT_ID, |profile| {
                profile.launch_contract_id
            }),
            binary_sha256: self.binary.sha256().to_owned(),
            runtime_bundle_manifest_sha256: self
                .runtime_bundle
                .as_ref()
                .map(|bundle| bundle.sha256().to_owned()),
            model_sha256: self.model.sha256().to_owned(),
            model_size: self.model.identity().size,
            model_alias: self.model_alias.clone(),
            model_origin: writing.and_then(|profile| profile.model_origin),
            threads: self.threads,
            context_size: self.context_size(),
            gpu_layers: GPU_LAYERS,
            batch_size: writing.map(|profile| profile.batch_size),
            ubatch_size: writing.map(|profile| profile.batch_size),
        }
    }

    fn context_size(&self) -> u16 {
        self.writing
            .map_or(CONTEXT_SIZE, |profile| profile.context_size)
    }

    fn configure_mode(&self, command: &mut Command) {
        if let Some(behavior) = self.fixture_behavior {
            command.env("BADI_FIXTURE_BEHAVIOR", behavior.environment_value());
        }
        if let Some(profile) = self.writing {
            // Reuse only the active slot's bounded KV state. Disable the
            // separate RAM archive; no slot-save path or disk cache is enabled.
            command
                .env("LLAMA_ARG_CACHE_PROMPT", "1")
                .env("LLAMA_ARG_CACHE_RAM", "0")
                // The runtime observes cancellation between prefill batches,
                // so the batch bounds the cost of a cancelled request.
                .env("LLAMA_ARG_BATCH", profile.batch_size.to_string())
                .env("LLAMA_ARG_UBATCH", profile.batch_size.to_string());
            if let Some(checkpoints) = profile.context_checkpoints {
                command.env("LLAMA_ARG_CTX_CHECKPOINTS", checkpoints.to_string());
            }
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
        if let Some(profile) = &self.writing {
            profile.validate()?;
        }
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
}

pub struct OwnedRuntime {
    child: Mutex<Option<Child>>,
    /// Pidfd of the owned child; it becomes readable once the child exits.
    exit_notice: Option<OwnedFd>,
    client: SemanticClient,
    endpoint: SocketAddr,
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
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
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
        terminate_owned_child_with(child, terminate_child).map_err(RuntimeError::Shutdown)
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
    #[error("failed to reserve a private loopback endpoint")]
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

fn reserve_loopback_endpoint() -> Result<SocketAddr, RuntimeError> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(RuntimeError::Endpoint)?;
    let endpoint = listener.local_addr().map_err(RuntimeError::Endpoint)?;
    drop(listener);
    Ok(endpoint)
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

fn encode_lower_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::io::Read as _;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use rustix::process::{Pid, PidfdFlags, Signal};
    use sha2::{Digest, Sha256};

    use super::{
        CompletionProvider as _, FixtureBehavior, LlamaCppLaunch, OwnedRuntime, ProvenanceError,
        RuntimeError, SecretToken, SemanticClient, SemanticClientConfig, StableRuntimeIdentity,
        WritingProfile, terminate_child, terminate_owned_child_with,
    };
    use crate::semantic::provenance::{
        DirectoryManifestExpectation, FileExpectation, VerifiedFile, directory_manifest_sha256,
        verify_directory_manifest, verify_file,
    };

    #[test]
    fn writing_bounds_batches_without_changing_evaluation_defaults() -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let binary_path = temporary.path().join("fixture");
        let model_path = temporary.path().join("model");
        fs::write(&binary_path, b"fixture")?;
        fs::write(&model_path, b"model")?;
        let launch = LlamaCppLaunch::for_fixture(
            verify_observed_file(&binary_path)?,
            verify_observed_file(&model_path)?,
            FixtureBehavior::Ready,
        )?;
        let baseline_identity = launch.identity();
        for (launch, writing) in [(launch.clone(), false), (launch.for_writing(), true)] {
            let mut command = Command::new(&binary_path);
            command.env_clear().env("LLAMA_ARG_CACHE_PROMPT", "0");
            launch.configure_mode(&mut command);
            let identity = launch.identity();
            let serialized_identity = serde_json::to_value(&identity)?;
            for name in ["batch_size", "ubatch_size"] {
                assert_eq!(
                    serialized_identity.get(name),
                    writing.then_some(&serde_json::json!(16))
                );
            }
            assert_eq!(identity.sha256() == baseline_identity.sha256(), !writing);
            if writing {
                let mut previous_writing_identity = identity.clone();
                previous_writing_identity.batch_size = None;
                previous_writing_identity.ubatch_size = None;
                assert_ne!(identity.sha256(), previous_writing_identity.sha256());
            }
            let environment: std::collections::BTreeMap<_, _> = command
                .get_envs()
                .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
                .collect();
            assert_eq!(
                environment["LLAMA_ARG_CACHE_PROMPT"],
                if writing { "1" } else { "0" }
            );
            for name in ["LLAMA_ARG_BATCH", "LLAMA_ARG_UBATCH"] {
                assert_eq!(environment.get(name).copied(), writing.then_some("16"));
            }
            assert_eq!(
                environment.get("LLAMA_ARG_CACHE_RAM").copied(),
                writing.then_some("0")
            );
        }
        Ok(())
    }

    const TEST_PROFILE: WritingProfile = WritingProfile {
        launch_contract_id: "badi.test.writing-profile.v1",
        context_size: 1_024,
        batch_size: 64,
        context_checkpoints: Some(0),
        model_origin: Some("test_artifact"),
    };

    fn fixture_launch(directory: &Path) -> Result<LlamaCppLaunch, Box<dyn Error>> {
        let binary = directory.join("runtime");
        let model = directory.join("model");
        fs::write(&binary, b"fixture runtime")?;
        fs::write(&model, b"fixture model")?;
        Ok(LlamaCppLaunch::for_fixture(
            verify_observed_file(&binary)?,
            verify_observed_file(&model)?,
            FixtureBehavior::Ready,
        )?)
    }

    fn launch_environment(launch: &LlamaCppLaunch) -> BTreeMap<String, String> {
        let mut command = Command::new("/bin/true");
        launch.configure_mode(&mut command);
        command
            .get_envs()
            .filter_map(|(key, value)| {
                Some((key.to_str()?.to_owned(), value?.to_str()?.to_owned()))
            })
            .collect()
    }

    #[test]
    fn a_writing_profile_binds_its_launch_and_identity() -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let launch = fixture_launch(temporary.path())?;
        let production = launch.clone().for_writing();
        assert_eq!(
            launch
                .clone()
                .with_writing_profile(WritingProfile::PRODUCTION)?
                .identity(),
            production.identity()
        );
        let profiled = launch.clone().with_writing_profile(TEST_PROFILE)?;
        let identity = profiled.identity();
        assert_eq!(identity.launch_contract_id, "badi.test.writing-profile.v1");
        assert_eq!(identity.context_size, 1_024);
        assert_eq!(
            (identity.batch_size, identity.ubatch_size),
            (Some(64), Some(64))
        );
        assert_eq!(identity.model_origin, Some("test_artifact"));
        // Only the profile's own fields differ from the production identity.
        let mut as_production = identity.clone();
        as_production.launch_contract_id = WritingProfile::PRODUCTION.launch_contract_id;
        as_production.context_size = super::CONTEXT_SIZE;
        as_production.batch_size = Some(16);
        as_production.ubatch_size = Some(16);
        as_production.model_origin = None;
        assert_eq!(as_production, production.identity());
        // An absent origin is omitted from the digest input, not serialized.
        let without_origin = launch
            .with_writing_profile(WritingProfile {
                model_origin: None,
                ..TEST_PROFILE
            })?
            .identity();
        assert!(
            serde_json::to_value(&without_origin)?
                .get("model_origin")
                .is_none()
        );
        assert_ne!(without_origin.sha256(), identity.sha256());

        for (selected, batch, checkpoints) in
            [(production, "16", None), (profiled, "64", Some("0"))]
        {
            let environment = launch_environment(&selected);
            assert_eq!(environment["LLAMA_ARG_BATCH"], batch);
            assert_eq!(environment["LLAMA_ARG_UBATCH"], batch);
            assert_eq!(environment["LLAMA_ARG_CACHE_PROMPT"], "1");
            assert_eq!(environment["LLAMA_ARG_CACHE_RAM"], "0");
            assert_eq!(
                environment
                    .get("LLAMA_ARG_CTX_CHECKPOINTS")
                    .map(String::as_str),
                checkpoints
            );
        }
        Ok(())
    }

    #[test]
    fn writing_profiles_outside_their_bounds_are_rejected() -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let launch = fixture_launch(temporary.path())?;
        let long_contract: &'static str = "c".repeat(129).leak();
        let long_origin: &'static str = "o".repeat(65).leak();
        let contract = |launch_contract_id| WritingProfile {
            launch_contract_id,
            ..TEST_PROFILE
        };
        let context = |context_size| WritingProfile {
            context_size,
            ..TEST_PROFILE
        };
        let batch = |batch_size| WritingProfile {
            batch_size,
            ..TEST_PROFILE
        };
        let checkpoints = |context_checkpoints| WritingProfile {
            context_checkpoints,
            ..TEST_PROFILE
        };
        let origin = |model_origin| WritingProfile {
            model_origin,
            ..TEST_PROFILE
        };
        for (profile, field) in [
            (contract(""), "launch_contract_id"),
            (contract("two words"), "launch_contract_id"),
            (contract("badi.v1\n"), "launch_contract_id"),
            (contract(long_contract), "launch_contract_id"),
            (context(0), "context_size"),
            (context(8_193), "context_size"),
            (batch(0), "batch_size"),
            (batch(1_025), "batch_size"),
            (checkpoints(Some(65)), "context_checkpoints"),
            (origin(Some("")), "model_origin"),
            (origin(Some("Lab")), "model_origin"),
            (origin(Some("lab-artifact")), "model_origin"),
            (origin(Some(long_origin)), "model_origin"),
        ] {
            assert!(
                matches!(
                    launch.clone().with_writing_profile(profile),
                    Err(RuntimeError::InvalidConfig(name)) if name == field
                ),
                "{profile:?}"
            );
        }
        let boundary = WritingProfile {
            launch_contract_id: &long_contract[1..],
            context_size: 8_192,
            batch_size: 8_192,
            context_checkpoints: Some(64),
            model_origin: Some(&long_origin[1..]),
        };
        assert!(launch.with_writing_profile(boundary).is_ok());
        Ok(())
    }

    /// Installed runtimes, lifecycle receipts and Lab provenance compare the
    /// identity digest, so the evaluation and production launches keep their
    /// exact serialized identity.
    #[test]
    fn evaluation_and_production_identity_serialization_is_pinned() -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let launch = fixture_launch(temporary.path())?;
        let common = concat!(
            ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
            ",\"runtime_bundle_manifest_sha256\":null",
            ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
            ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
            ",\"context_size\":512,\"gpu_layers\":0",
        );
        assert_eq!(
            serde_json::to_string(&launch.identity())?,
            format!("{{\"launch_contract_id\":\"badi.llama-cpp-owned-eval.v1\"{common}}}")
        );
        assert_eq!(
            serde_json::to_string(&launch.for_writing().identity())?,
            format!(
                "{{\"launch_contract_id\":\"badi.writing.completion-and-spelling.en-de-fa.v2\"{common},\"batch_size\":16,\"ubatch_size\":16}}"
            )
        );
        Ok(())
    }

    #[test]
    fn contained_launch_execs_through_the_parent_death_helper() -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let bundle_path = root.join("runtime");
        fs::create_dir(&bundle_path)?;
        let binary_path = bundle_path.join("llama-server");
        fs::write(&binary_path, b"fixture")?;
        let model_path = root.join("model.gguf");
        fs::write(&model_path, b"model")?;
        let bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
            &bundle_path,
            directory_manifest_sha256(&bundle_path)?,
        )?)?;
        let binary = verify_observed_file(&binary_path)?;
        let model = verify_observed_file(&model_path)?;

        let direct = LlamaCppLaunch::new(binary.clone(), bundle, model.clone(), "fixture", 1)?;
        assert_eq!(direct.runtime_command()?.get_program(), binary_path);
        let command = direct.contained().runtime_command()?;
        assert_eq!(command.get_program(), std::env::current_exe()?);
        let arguments: Vec<_> = command.get_args().collect();
        assert_eq!(
            arguments,
            [
                std::ffi::OsStr::new(crate::semantic::process::EXEC_HELPER_FLAG),
                std::ffi::OsStr::new(&std::process::id().to_string()),
                binary_path.as_os_str(),
            ]
        );
        // The helper passes no arguments, so a fixture cannot be contained.
        let fixture = LlamaCppLaunch::for_fixture(binary, model, FixtureBehavior::Ready)?;
        assert!(matches!(
            fixture.contained().validate(),
            Err(RuntimeError::InvalidConfig("runtime_bundle"))
        ));
        Ok(())
    }

    #[test]
    fn shutdown_allows_a_busy_child_to_finish_its_term_cleanup() -> Result<(), Box<dyn Error>> {
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "trap 'sleep 0.35; exit 0' TERM; printf r; while :; do sleep 10 & wait $!; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let mut ready = [0_u8; 1];
        child
            .stdout
            .as_mut()
            .expect("readiness pipe")
            .read_exact(&mut ready)?;
        assert_eq!(ready, *b"r");
        let status = terminate_child(&mut child)?;
        assert!(status.success(), "TERM cleanup was interrupted: {status}");
        Ok(())
    }

    #[tokio::test]
    async fn post_spawn_bundle_mutation_is_rejected_and_child_is_reaped()
    -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let root = fs::canonicalize(temporary.path())?;
        let bundle_path = root.join("runtime");
        let model_directory = root.join("model");
        fs::create_dir(&bundle_path)?;
        fs::create_dir(&model_directory)?;
        let binary_path = bundle_path.join("llama-server");
        fs::write(
            &binary_path,
            b"#!/bin/sh\ntrap '' TERM\nwhile :; do /bin/sleep 1; done\n",
        )?;
        fs::set_permissions(&binary_path, fs::Permissions::from_mode(0o700))?;
        let library_path = bundle_path.join("libfixture.so");
        fs::write(&library_path, b"reviewed")?;
        let model_path = model_directory.join("fixture.gguf");
        fs::write(&model_path, b"model")?;

        let binary = verify_observed_file(&binary_path)?;
        let model = verify_observed_file(&model_path)?;
        let bundle_digest = directory_manifest_sha256(&bundle_path)?;
        let bundle = verify_directory_manifest(&DirectoryManifestExpectation::new(
            &bundle_path,
            bundle_digest,
        )?)?;
        let launch = LlamaCppLaunch::new(binary, bundle, model, "fixture", 1)?;
        let process_id = Arc::new(AtomicU32::new(0));
        let observed_process_id = Arc::clone(&process_id);
        let result = launch
            .spawn_with_post_spawn_hook(move |spawned_process_id| {
                observed_process_id.store(spawned_process_id, Ordering::SeqCst);
                let replacement = library_path.with_extension("tmp");
                fs::write(&replacement, b"tampered").expect("test mutation must succeed");
                fs::rename(&replacement, &library_path).expect("test mutation must succeed");
            })
            .await;
        assert!(matches!(
            result,
            Err(RuntimeError::Provenance(ProvenanceError::IdentityChanged))
        ));
        let process_id = process_id.load(Ordering::SeqCst);
        assert_ne!(process_id, 0);
        assert!(!Path::new(&format!("/proc/{process_id}")).exists());
        Ok(())
    }

    #[test]
    fn failed_termination_keeps_child_owned_for_retry() -> Result<(), Box<dyn Error>> {
        let child = Command::new("/bin/sh")
            .arg("-c")
            .arg("while :; do /bin/sleep 1; done")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let process_id = child.id();
        let mut child = Some(child);
        let failure = terminate_owned_child_with(&mut child, |_| {
            Err(io::Error::other("injected termination failure"))
        });
        assert!(failure.is_err());
        assert_eq!(
            child.as_ref().map(std::process::Child::id),
            Some(process_id)
        );

        terminate_owned_child_with(&mut child, terminate_child)?;
        assert!(child.is_none());
        assert!(!Path::new(&format!("/proc/{process_id}")).exists());
        Ok(())
    }

    fn owned_runtime_at(endpoint: std::net::SocketAddr) -> Result<OwnedRuntime, Box<dyn Error>> {
        let child = Command::new("/bin/sleep")
            .arg("30")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        let exit_notice =
            rustix::process::pidfd_open(Pid::from_child(&child), PidfdFlags::empty()).ok();
        Ok(OwnedRuntime {
            child: Mutex::new(Some(child)),
            exit_notice,
            client: SemanticClient::new(
                SemanticClientConfig::new(endpoint, "fixture", "public-fixture-token")?
                    .for_writing(),
            )?,
            endpoint,
            token_credential: SecretToken::fixture(),
            identity: StableRuntimeIdentity {
                launch_contract_id: crate::writing::WRITING_CONTRACT,
                binary_sha256: "0".repeat(64),
                runtime_bundle_manifest_sha256: None,
                model_sha256: "0".repeat(64),
                model_size: 1,
                model_alias: "fixture".to_owned(),
                model_origin: None,
                threads: 1,
                context_size: super::CONTEXT_SIZE,
                gpu_layers: super::GPU_LAYERS,
                batch_size: Some(16),
                ubatch_size: Some(16),
            },
        })
    }

    #[tokio::test]
    async fn exit_is_observed_by_event_and_by_the_polling_fallback() -> Result<(), Box<dyn Error>> {
        let endpoint = std::net::SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 9));
        for event_driven in [true, false] {
            let mut runtime = owned_runtime_at(endpoint)?;
            assert!(runtime.exit_notice.is_some());
            if !event_driven {
                runtime.exit_notice = None;
            }
            assert!(
                tokio::time::timeout(Duration::from_millis(100), runtime.exited())
                    .await
                    .is_err(),
                "a live runtime must not report an exit"
            );
            let pid = Pid::from_raw(i32::try_from(runtime.process_id().unwrap())?).unwrap();
            rustix::process::kill_process(pid, Signal::KILL)?;
            tokio::time::timeout(Duration::from_secs(3), runtime.exited()).await?;
            assert!(!runtime.is_alive());
        }
        Ok(())
    }

    #[tokio::test]
    async fn warm_up_reports_completion_and_failure_without_failing_the_runtime()
    -> Result<(), Box<dyn Error>> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
        let runtime = owned_runtime_at(listener.local_addr()?)?;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("warm-up connection");
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = socket.read(&mut buffer).await.expect("warm-up request");
                assert_ne!(read, 0);
                request.extend_from_slice(&buffer[..read]);
                let text = String::from_utf8_lossy(&request);
                let Some((headers, body)) = text.split_once("\r\n\r\n") else {
                    continue;
                };
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .expect("content length")
                    .parse()
                    .expect("length");
                if body.len() >= length {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .expect("warm-up reply");
            let (_stalled, _) = listener.accept().await.expect("stalled connection");
            tokio::time::sleep(Duration::from_secs(2)).await;
        });
        let completed = runtime.warm_up().await;
        assert_eq!(completed.failure(), None);
        let rendered = completed.to_string();
        assert!(rendered.starts_with("completed elapsed_ms="), "{rendered}");

        let failed = runtime.warm_up_within(Duration::from_millis(100)).await;
        assert_eq!(failed.failure(), Some("timeout"));
        assert!(failed.elapsed() < Duration::from_millis(500));
        let rendered = failed.to_string();
        assert!(
            rendered.starts_with("failed class=timeout elapsed_ms="),
            "{rendered}"
        );
        assert!(crate::provider::CompletionProvider::is_alive(&runtime));
        server.abort();
        let process_id = runtime.process_id().expect("owned child");
        drop(runtime);
        assert!(!Path::new(&format!("/proc/{process_id}")).exists());
        Ok(())
    }

    fn verify_observed_file(path: &Path) -> Result<VerifiedFile, Box<dyn Error>> {
        let bytes = fs::read(path)?;
        Ok(verify_file(&FileExpectation::new(
            path,
            super::encode_lower_hex(Sha256::digest(&bytes)),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )?)?)
    }
}
