use std::fmt;
use std::io;
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
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
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::protocol::ProviderKind;
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest, WritingProposal};

use super::client::{ClientError, HealthStatus, SemanticClient, SemanticClientConfig};
use super::provenance::{ProvenanceError, VerifiedDirectoryManifest, VerifiedFile};

pub const LLAMA_CPP_LAUNCH_CONTRACT_ID: &str = "badi.llama-cpp-owned-eval.v1";
pub const CONTEXT_SIZE: u16 = 512;
pub const GPU_LAYERS: u16 = 0;
const WRITING_BATCH_SIZE: u16 = 16;
const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);
const HEALTH_POLL_INTERVAL: Duration = Duration::from_millis(20);
// A cancelled cold prefill may still be unwinding inside the runtime. Allow
// that work and its HTTP readers to stop before the process-group kill fallback.
const GRACEFUL_SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
const FORCE_SHUTDOWN_WAIT: Duration = Duration::from_secs(1);
const FIXTURE_ARGUMENT: &str = "__fixture-backend";
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

#[derive(Clone, Debug)]
pub struct LlamaCppLaunch {
    binary: VerifiedFile,
    runtime_bundle: Option<VerifiedDirectoryManifest>,
    model: VerifiedFile,
    model_alias: String,
    startup_timeout: Duration,
    threads: usize,
    fixture_behavior: Option<FixtureBehavior>,
    writing: bool,
    #[cfg(feature = "writing-lab")]
    writing_lab: bool,
    #[cfg(feature = "writing-lab")]
    explicit_lab_artifact: bool,
    #[cfg(feature = "writing-lab")]
    lab_prefill_batch: crate::writing_lab::PrefillBatch,
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
            writing: false,
            #[cfg(feature = "writing-lab")]
            writing_lab: false,
            #[cfg(feature = "writing-lab")]
            explicit_lab_artifact: false,
            #[cfg(feature = "writing-lab")]
            lab_prefill_batch: crate::writing_lab::PrefillBatch::Tokens16,
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
            writing: false,
            #[cfg(feature = "writing-lab")]
            writing_lab: false,
            #[cfg(feature = "writing-lab")]
            explicit_lab_artifact: false,
            #[cfg(feature = "writing-lab")]
            lab_prefill_batch: crate::writing_lab::PrefillBatch::Tokens16,
        };
        launch.validate()?;
        Ok(launch)
    }

    pub async fn spawn(self) -> Result<OwnedRuntime, RuntimeError> {
        self.spawn_with_post_spawn_hook(|_| {}).await
    }

    #[must_use]
    pub const fn for_writing(mut self) -> Self {
        self.writing = true;
        self
    }

    #[cfg(feature = "writing-lab")]
    #[must_use]
    pub const fn for_writing_lab(mut self) -> Self {
        self.writing = true;
        self.writing_lab = true;
        self
    }

    #[cfg(feature = "writing-lab")]
    pub(crate) const fn for_writing_lab_artifact(mut self) -> Self {
        self = self.for_writing_lab();
        self.explicit_lab_artifact = true;
        self
    }

    #[cfg(feature = "writing-lab")]
    pub(crate) const fn with_lab_prefill_batch(
        mut self,
        batch: crate::writing_lab::PrefillBatch,
    ) -> Self {
        self.lab_prefill_batch = batch;
        self
    }

    fn writing_batch_size(&self) -> u16 {
        #[cfg(feature = "writing-lab")]
        if self.writing_lab {
            return self.lab_prefill_batch.tokens();
        }
        WRITING_BATCH_SIZE
    }

    fn model_origin(&self) -> Option<&'static str> {
        #[cfg(feature = "writing-lab")]
        if self.explicit_lab_artifact {
            return Some("explicit_lab_artifact");
        }
        None
    }

    fn context_size(&self) -> u16 {
        #[cfg(feature = "writing-lab")]
        if self.writing_lab {
            return crate::writing_lab::CONTEXT_TOKENS;
        }
        CONTEXT_SIZE
    }

    fn launch_contract(&self) -> &'static str {
        #[cfg(feature = "writing-lab")]
        if self.writing_lab {
            return crate::writing_lab::LAUNCH_CONTRACT;
        }
        if self.writing {
            crate::writing::WRITING_CONTRACT
        } else {
            LLAMA_CPP_LAUNCH_CONTRACT_ID
        }
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
        let config = if self.writing {
            config.for_writing()
        } else {
            config
        };
        let client = SemanticClient::new(config)?;
        let negative_config =
            SemanticClientConfig::new(endpoint, &self.model_alias, negative_token.expose())?;
        let negative_client = SemanticClient::new(negative_config)?;
        let identity = self.runtime_identity();
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
        self.reverify_artifacts()?;
        let child = command.spawn().map_err(RuntimeError::Spawn)?;
        let mut runtime = OwnedRuntime {
            child: Mutex::new(Some(child)),
            client,
            endpoint,
            token_credential: token,
            identity,
        };
        post_spawn_hook(runtime.process_id().ok_or(RuntimeError::MissingChild)?);
        if let Err(error) = self.reverify_artifacts() {
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
        if let Err(error) = self.reverify_artifacts() {
            let _ = runtime.terminate();
            return Err(error.into());
        }
        Ok(runtime)
    }

    fn runtime_command(&self) -> Result<Command, RuntimeError> {
        #[cfg(feature = "writing-lab")]
        if self.writing_lab {
            #[cfg(target_os = "linux")]
            return crate::writing_lab::process::launch_command(self.binary.path())
                .map_err(RuntimeError::Spawn);
            #[cfg(not(target_os = "linux"))]
            return Err(RuntimeError::InvalidConfig("lab_parent_death_unavailable"));
        }
        Ok(Command::new(self.binary.path()))
    }

    fn runtime_identity(&self) -> StableRuntimeIdentity {
        StableRuntimeIdentity {
            launch_contract_id: self.launch_contract(),
            binary_sha256: self.binary.sha256().to_owned(),
            runtime_bundle_manifest_sha256: self
                .runtime_bundle
                .as_ref()
                .map(|bundle| bundle.sha256().to_owned()),
            model_sha256: self.model.sha256().to_owned(),
            model_size: self.model.identity().size,
            model_alias: self.model_alias.clone(),
            model_origin: self.model_origin(),
            threads: self.threads,
            context_size: self.context_size(),
            gpu_layers: GPU_LAYERS,
            batch_size: self.writing.then_some(self.writing_batch_size()),
            ubatch_size: self.writing.then_some(self.writing_batch_size()),
        }
    }

    fn configure_mode(&self, command: &mut Command) {
        #[cfg(feature = "writing-lab")]
        if self.writing_lab {
            // Bounded model comparisons own one active slot. Recurrent model
            // checkpoint copies must not inherit the runtime's default of 32.
            command.env("LLAMA_ARG_CTX_CHECKPOINTS", "0");
        }
        if let Some(behavior) = self.fixture_behavior {
            command.env("BADI_FIXTURE_BEHAVIOR", behavior.environment_value());
        }
        if self.writing {
            // Reuse only the active slot's bounded KV state. Disable the
            // separate RAM archive; no slot-save path or disk cache is enabled.
            command
                .env("LLAMA_ARG_CACHE_PROMPT", "1")
                .env("LLAMA_ARG_CACHE_RAM", "0")
                // The runtime observes cancellation between prefill batches.
                // Production stays at 16. Only an explicit Lab launch can test
                // a larger chunk, whose cancellation cost is measured separately.
                .env("LLAMA_ARG_BATCH", self.writing_batch_size().to_string())
                .env("LLAMA_ARG_UBATCH", self.writing_batch_size().to_string());
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
        match (&self.runtime_bundle, self.fixture_behavior) {
            (Some(bundle), None) if self.binary.path().parent() == Some(bundle.path()) => {}
            (None, Some(_)) => {}
            _ => return Err(RuntimeError::InvalidConfig("runtime_bundle")),
        }
        Ok(())
    }

    fn reverify_artifacts(&self) -> Result<(), ProvenanceError> {
        if let Some(bundle) = &self.runtime_bundle {
            bundle.reverify()?;
        }
        self.binary.reverify()?;
        self.model.reverify()
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
}

impl StableRuntimeIdentity {
    #[must_use]
    pub fn sha256(&self) -> String {
        let canonical = serde_json::to_vec(self).expect("runtime identity is serializable");
        encode_lower_hex(Sha256::digest(canonical))
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
    use std::error::Error;
    use std::fs;
    use std::io;
    use std::io::Read as _;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use sha2::{Digest, Sha256};

    use super::{
        FixtureBehavior, LlamaCppLaunch, ProvenanceError, RuntimeError, terminate_child,
        terminate_owned_child_with,
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
        let baseline_identity = launch.runtime_identity();
        for (launch, writing) in [(launch.clone(), false), (launch.for_writing(), true)] {
            let mut command = Command::new(&binary_path);
            command.env_clear().env("LLAMA_ARG_CACHE_PROMPT", "0");
            launch.configure_mode(&mut command);
            let identity = launch.runtime_identity();
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

    #[cfg(feature = "writing-lab")]
    #[test]
    fn lab_prefill_batches_bind_launch_and_identity_without_changing_other_defaults()
    -> Result<(), Box<dyn Error>> {
        use crate::writing_lab::PrefillBatch;

        let temporary = tempfile::tempdir()?;
        let binary = temporary.path().join("fixture");
        let model = temporary.path().join("weights");
        fs::write(&binary, b"fixture")?;
        fs::write(&model, b"disposable model")?;
        let launch = LlamaCppLaunch::for_fixture(
            verify_observed_file(&binary)?,
            verify_observed_file(&model)?,
            FixtureBehavior::Ready,
        )?;
        for unchanged in [launch.clone(), launch.clone().for_writing()] {
            let selected = unchanged
                .clone()
                .with_lab_prefill_batch(PrefillBatch::Tokens64);
            assert_eq!(
                serde_json::to_vec(&selected.runtime_identity())?,
                serde_json::to_vec(&unchanged.runtime_identity())?
            );
            assert_eq!(selected.writing_batch_size(), super::WRITING_BATCH_SIZE);
        }
        let baseline = launch.for_writing_lab();
        let explicit_default = baseline
            .clone()
            .with_lab_prefill_batch(PrefillBatch::Tokens16);
        assert_eq!(
            serde_json::to_vec(&baseline.runtime_identity())?,
            serde_json::to_vec(&explicit_default.runtime_identity())?
        );
        let candidate = baseline
            .clone()
            .with_lab_prefill_batch(PrefillBatch::Tokens64);
        let mut candidate_identity = candidate.runtime_identity();
        assert_eq!(candidate_identity.batch_size, Some(64));
        assert_eq!(candidate_identity.ubatch_size, Some(64));
        assert_ne!(
            candidate_identity.sha256(),
            baseline.runtime_identity().sha256()
        );
        candidate_identity.batch_size = Some(16);
        candidate_identity.ubatch_size = Some(16);
        assert_eq!(candidate_identity, baseline.runtime_identity());
        for (selected, expected) in [(baseline, "16"), (candidate, "64")] {
            let mut command = Command::new(&binary);
            selected.configure_mode(&mut command);
            let environment: std::collections::BTreeMap<_, _> = command
                .get_envs()
                .map(|(key, value)| (key.to_str().unwrap(), value.unwrap().to_str().unwrap()))
                .collect();
            assert_eq!(environment["LLAMA_ARG_BATCH"], expected);
            assert_eq!(environment["LLAMA_ARG_UBATCH"], expected);
            assert_eq!(environment["LLAMA_ARG_CACHE_PROMPT"], "1");
            assert_eq!(environment["LLAMA_ARG_CACHE_RAM"], "0");
        }
        Ok(())
    }

    #[cfg(feature = "writing-lab")]
    #[test]
    fn explicit_lab_artifact_identity_is_bound_without_changing_default_serialization()
    -> Result<(), Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let binary = temporary.path().join("fixture");
        let model = temporary.path().join("weights");
        fs::write(&binary, b"fixture")?;
        fs::write(&model, b"disposable model")?;
        let launch = LlamaCppLaunch::for_fixture(
            verify_observed_file(&binary)?,
            verify_observed_file(&model)?,
            FixtureBehavior::Ready,
        )?;
        for default in [
            launch.clone(),
            launch.clone().for_writing(),
            launch.clone().for_writing_lab(),
        ] {
            assert!(
                serde_json::to_value(default.runtime_identity())?
                    .get("model_origin")
                    .is_none()
            );
        }
        let ordinary = launch.clone().for_writing_lab().runtime_identity();
        let explicit = launch.for_writing_lab_artifact().runtime_identity();
        assert_eq!(explicit.model_origin, Some("explicit_lab_artifact"));
        assert_ne!(explicit.sha256(), ordinary.sha256());
        let mut without_marker = explicit;
        without_marker.model_origin = None;
        assert_eq!(ordinary, without_marker);
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
                fs::write(&library_path, b"tampered").expect("test mutation must succeed");
            })
            .await;
        assert!(matches!(
            result,
            Err(RuntimeError::Provenance(
                ProvenanceError::DirectoryManifestDigestMismatch
            ))
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

    fn verify_observed_file(path: &Path) -> Result<VerifiedFile, Box<dyn Error>> {
        let bytes = fs::read(path)?;
        Ok(verify_file(&FileExpectation::new(
            path,
            super::encode_lower_hex(Sha256::digest(&bytes)),
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        )?)?)
    }
}
