use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::NoSuggestionReason;
use crate::protocol::{
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_SUGGESTION_CHARS, MAX_SUGGESTION_WORDS, ProviderKind,
};
use crate::provider::{
    CompletionProvider, ProviderError, ProviderOutcome, ProviderRequest, RequestTrigger,
    WritingProposal,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;

use super::wire::{
    self, Method, NativeStreamChunk, Response, StatusCode, TokenizeResponse, ensure_content_type,
    event_data, next_event_boundary, read_bounded_body,
};

pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_000);
pub const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_200);
pub const MAX_RESPONSE_BYTES: usize = 16 * 1_024;

const MAX_MODEL_ALIAS_BYTES: usize = 256;
const MAX_RUNTIME_PATH_BYTES: usize = 32;
const MAX_TOKEN_BYTES: usize = 4_096;
const MAX_RAW_OUTPUT_BYTES: usize = 1_024;
// After the writing budget, the outer deadline still lets the reader return
// words salvaged at the budget, and the HTTP timeout never ends it first.
const WRITING_RESULT_GRACE: Duration = Duration::from_millis(20);
const WRITING_HTTP_GRACE: Duration = Duration::from_millis(100);
const AUTHORIZATION_CHALLENGE_TEXT: &str = "badi-owned-runtime-challenge";
const WARM_UP_PROMPT: &str = "Thank you for your";
const WARM_UP_TOKENS: u16 = 2;
// A non-streaming reply also echoes generation settings; its body is discarded.
const MAX_WARM_UP_RESPONSE_BYTES: usize = 64 * 1_024;
/// How to reach and bound the owned writing runtime.
#[derive(Clone)]
pub struct SemanticClientConfig {
    endpoint: SocketAddr,
    model_alias: String,
    /// The `Authorization` header value: `Bearer` and the runtime's token.
    authorization: String,
    connect_timeout: Duration,
    request_timeout: Duration,
}

impl SemanticClientConfig {
    pub fn new(
        endpoint: SocketAddr,
        model_alias: impl Into<String>,
        token: impl AsRef<str>,
    ) -> Result<Self, ClientError> {
        let raw_token = token.as_ref();
        // A header value may hold any byte but controls other than tab, so
        // the token cannot end the header early.
        if raw_token.is_empty()
            || raw_token.len() > MAX_TOKEN_BYTES
            || raw_token
                .bytes()
                .any(|byte| byte != b'\t' && (byte < b' ' || byte == 0x7f))
        {
            return Err(ClientError::InvalidConfig("token"));
        }
        let config = Self {
            endpoint,
            model_alias: model_alias.into(),
            authorization: format!("Bearer {raw_token}"),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        };
        config.validate()?;
        Ok(config)
    }

    pub fn with_timeouts(
        mut self,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, ClientError> {
        self.connect_timeout = connect_timeout;
        self.request_timeout = request_timeout;
        self.validate()?;
        Ok(self)
    }

    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    fn validate(&self) -> Result<(), ClientError> {
        if !self.endpoint.ip().is_loopback() || self.endpoint.port() == 0 {
            return Err(ClientError::InvalidConfig("endpoint"));
        }
        if self.model_alias.is_empty()
            || self.model_alias.len() > MAX_MODEL_ALIAS_BYTES
            || self.model_alias.chars().any(char::is_control)
        {
            return Err(ClientError::InvalidConfig("model_alias"));
        }
        if self.connect_timeout.is_zero()
            || self.connect_timeout > MAX_CONNECT_TIMEOUT
            || self.request_timeout.is_zero()
            || self.request_timeout > MAX_REQUEST_TIMEOUT
            || self.connect_timeout > self.request_timeout
        {
            return Err(ClientError::InvalidConfig("timeouts"));
        }
        Ok(())
    }
}

impl fmt::Debug for SemanticClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SemanticClientConfig")
            .field("endpoint", &self.endpoint)
            .field("model_alias", &self.model_alias)
            .field("authorization", &"[redacted]")
            .field("connect_timeout", &self.connect_timeout)
            .field("request_timeout", &self.request_timeout)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionDisposition {
    Suggested,
    ModelAbstained,
    LanguageAbstained,
    InvalidOutput,
}

#[derive(Clone, Debug)]
pub struct ObservedCompletion {
    disposition: CompletionDisposition,
    /// Diagnostic class of an abstention; `None` for output or errors.
    no_suggestion: Option<NoSuggestionReason>,
    output: Option<String>,
    ttft: Option<Duration>,
    elapsed: Duration,
    request_body_bytes: usize,
    response_body_bytes: usize,
}

impl ObservedCompletion {
    #[must_use]
    pub const fn disposition(&self) -> CompletionDisposition {
        self.disposition
    }

    #[must_use]
    pub const fn no_suggestion_reason(&self) -> Option<NoSuggestionReason> {
        self.no_suggestion
    }

    #[must_use]
    pub fn output(&self) -> Option<&str> {
        self.output.as_deref()
    }

    #[must_use]
    pub const fn ttft(&self) -> Option<Duration> {
        self.ttft
    }

    #[must_use]
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    #[must_use]
    pub const fn request_body_bytes(&self) -> usize {
        self.request_body_bytes
    }

    #[must_use]
    pub const fn response_body_bytes(&self) -> usize {
        self.response_body_bytes
    }

    fn language_abstention() -> Self {
        Self {
            disposition: CompletionDisposition::LanguageAbstained,
            no_suggestion: Some(NoSuggestionReason::RequestAbstained),
            output: None,
            ttft: None,
            elapsed: Duration::ZERO,
            request_body_bytes: 0,
            response_body_bytes: 0,
        }
    }

    fn writing_budget_abstention(started: Instant, request_body_bytes: usize) -> Self {
        // Match the stream deadline's no-output convention. This records a
        // spent writing budget, not a model refusal or a runtime health result.
        Self {
            disposition: CompletionDisposition::ModelAbstained,
            no_suggestion: Some(NoSuggestionReason::BudgetPrefill),
            output: None,
            ttft: None,
            elapsed: started.elapsed(),
            request_body_bytes,
            response_body_bytes: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HealthStatus {
    Ready,
    Loading,
}

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("semantic request cancelled")]
    Cancelled,
    #[error("semantic request timed out")]
    Timeout,
    #[error("invalid semantic configuration: {0}")]
    InvalidConfig(&'static str),
    #[error("semantic request exceeded the context contract")]
    InvalidRequest,
    /// The connection failed, closed early or broke HTTP/1.1 framing.
    #[error("semantic HTTP transport failed")]
    Transport(#[source] std::io::Error),
    #[error("semantic endpoint returned HTTP status {0}")]
    UnexpectedStatus(StatusCode),
    #[error("semantic endpoint returned an unexpected content type")]
    UnexpectedContentType,
    #[error("semantic response exceeded the byte limit")]
    ResponseTooLarge,
    #[error("semantic stream was malformed")]
    MalformedStream,
}

impl ClientError {
    #[must_use]
    pub const fn retryable_during_startup(&self) -> bool {
        matches!(self, Self::Transport(_) | Self::Timeout)
    }

    /// Content-free class for diagnostics.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::InvalidConfig(_) | Self::InvalidRequest => "configuration",
            Self::Transport(_) => "transport",
            Self::UnexpectedStatus(_) => "http_status",
            Self::UnexpectedContentType | Self::ResponseTooLarge | Self::MalformedStream => {
                "malformed_response"
            }
        }
    }
}

/// Sees the exact body of each request a [`SemanticClient`] posts to its
/// runtime, just before it is sent. Completion bodies carry the user's text:
/// the broker never installs an observer. It exists for explicit local
/// experiments that must record the requests production makes.
pub trait RequestObserver: fmt::Debug + Send + Sync {
    fn observe(&self, body: &[u8]);
}

/// The loopback runtime's client. Each request uses its own connection:
/// no proxy, redirect, retry or pooling.
#[derive(Clone, Debug)]
pub struct SemanticClient {
    config: SemanticClientConfig,
    request_observer: Option<Arc<dyn RequestObserver>>,
}

impl SemanticClient {
    pub fn new(config: SemanticClientConfig) -> Result<Self, ClientError> {
        config.validate()?;
        Ok(Self {
            config,
            request_observer: None,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &SemanticClientConfig {
        &self.config
    }

    /// This client with `observer` shown every request body it posts.
    #[must_use]
    pub fn with_request_observer(mut self, observer: Arc<dyn RequestObserver>) -> Self {
        self.request_observer = Some(observer);
        self
    }

    /// Posts one authenticated JSON `body` to `path` on the owned runtime and
    /// returns its response unread. `timeout` replaces the client's request
    /// timeout. `path` must be one lowercase segment such as `/completion`,
    /// so the credential can reach only this runtime's own endpoints.
    pub async fn post_runtime(
        &self,
        path: &'static str,
        body: Vec<u8>,
        timeout: Option<Duration>,
    ) -> Result<Response, ClientError> {
        let path = runtime_path(path)?;
        if let Some(observer) = &self.request_observer {
            observer.observe(&body);
        }
        self.exchange(Method::Post, path, Some(&body), timeout)
            .await
    }

    /// One request to the runtime, bounded as a whole by `timeout` or else
    /// by the client's request timeout.
    async fn exchange(
        &self,
        method: Method,
        path: &str,
        body: Option<&[u8]>,
        timeout: Option<Duration>,
    ) -> Result<Response, ClientError> {
        let deadline = tokio::time::Instant::now() + timeout.unwrap_or(self.config.request_timeout);
        let request = wire::Request {
            method,
            path,
            authorization: &self.config.authorization,
            body,
        };
        wire::exchange(
            self.config.endpoint,
            request,
            self.config.connect_timeout,
            deadline,
        )
        .await
    }

    pub async fn probe_health(
        &self,
        cancellation: CancellationToken,
    ) -> Result<HealthStatus, ClientError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = self.probe_health_inner() => result,
        }
    }

    /// Completes with the automatic writing budget.
    pub async fn complete_observed(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<ObservedCompletion, ClientError> {
        self.complete_observed_within(request, cancellation, RequestTrigger::Automatic)
            .await
    }

    /// Completes with the writing budget of `trigger`.
    pub async fn complete_observed_within(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        trigger: RequestTrigger,
    ) -> Result<ObservedCompletion, ClientError> {
        self.complete_observed_started(
            request,
            cancellation,
            Instant::now(),
            trigger.writing_budget(),
        )
        .await
    }

    async fn complete_observed_started(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<ObservedCompletion, ClientError> {
        match validate_request(&request)? {
            InputEligibility::Abstain => return Ok(ObservedCompletion::language_abstention()),
            InputEligibility::Eligible => {}
        }

        // The language boundary deliberately precedes construction and JSON
        // serialization. A rejected request therefore cannot allocate an HTTP
        // payload or send a runtime request/body byte.
        let plan =
            crate::writing::completion_plan(&request, crate::writing::TRAILING_SPACE_HEALING);
        // Healing can consume the whole window (one unfinished English word),
        // and an unhealed language can send only whitespace. Neither carries
        // context, and the runtime answers an empty prompt with a malformed
        // final chunk, so this is a request-side abstention, sent nowhere.
        if plan.prompt.trim().is_empty() {
            return Ok(ObservedCompletion::language_abstention());
        }
        let payload =
            serde_json::to_vec(&plan.payload()).map_err(|_| ClientError::InvalidRequest)?;
        let operation =
            self.complete_inner(payload, started, budget, cancellation.clone(), plan.echo);
        let deadline = started + budget + WRITING_RESULT_GRACE;
        let observed = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout_at(deadline.into(), operation) => {
                result.map_err(|_| ClientError::Timeout)?
            }
        }?;
        if observed.output.as_deref().is_some_and(|output| {
            !request
                .language
                .as_deref()
                .and_then(crate::writing::WritingLanguage::from_tag)
                .is_some_and(|language| language.accepts_output(output))
                || crate::writing::introduces_unsupported_fact(&request.before, output)
        }) {
            return Ok(ObservedCompletion {
                disposition: CompletionDisposition::ModelAbstained,
                no_suggestion: Some(NoSuggestionReason::OutputRejected),
                output: None,
                ..observed
            });
        }
        Ok(observed)
    }

    async fn complete_started(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<Result<String, NoSuggestionReason>, ProviderError> {
        let observed = self
            .complete_observed_started(request, cancellation, started, budget)
            .await
            .map_err(|error| match error {
                ClientError::Cancelled => ProviderError::Cancelled,
                _ => ProviderError::Unavailable,
            })?;
        let abstention = observed
            .no_suggestion
            .unwrap_or(NoSuggestionReason::ModelAbstained);
        match observed.disposition {
            CompletionDisposition::Suggested => Ok(observed.output.ok_or(abstention)),
            CompletionDisposition::ModelAbstained | CompletionDisposition::LanguageAbstained => {
                Ok(Err(abstention))
            }
            CompletionDisposition::InvalidOutput => Err(ProviderError::Unavailable),
        }
    }

    pub async fn probe_authorization_challenge(
        &self,
        cancellation: CancellationToken,
    ) -> Result<(), ClientError> {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = self.probe_authorization_challenge_inner() => result,
        }
    }

    /// Run one fixed, tiny completion so that a real request is not the
    /// runtime's first inference. It carries no user text; the reply is
    /// checked for status, type and size only and then discarded.
    pub async fn warm_up(
        &self,
        timeout: Duration,
        cancellation: CancellationToken,
    ) -> Result<(), ClientError> {
        if timeout.is_zero() {
            return Err(ClientError::InvalidConfig("warm_up_timeout"));
        }
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout(timeout, self.warm_up_inner(timeout)) => {
                result.map_err(|_| ClientError::Timeout)?
            }
        }
    }

    async fn warm_up_inner(&self, timeout: Duration) -> Result<(), ClientError> {
        let body = serde_json::to_vec(&serde_json::json!({
            "prompt": WARM_UP_PROMPT,
            "n_predict": WARM_UP_TOKENS,
            "temperature": 0.0,
            "seed": 42,
            "stream": false,
            "cache_prompt": false,
        }))
        .map_err(|_| ClientError::InvalidRequest)?;
        let response = self
            .post_runtime("/completion", body, Some(timeout))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(&response, "application/json")?;
        read_bounded_body(response, MAX_WARM_UP_RESPONSE_BYTES).await?;
        Ok(())
    }

    async fn probe_health_inner(&self) -> Result<HealthStatus, ClientError> {
        let response = self.exchange(Method::Get, "/health", None, None).await?;
        match response.status() {
            StatusCode::SERVICE_UNAVAILABLE => Ok(HealthStatus::Loading),
            StatusCode::OK => {
                ensure_content_type(&response, "application/json")?;
                let body = read_bounded_body(response, MAX_RESPONSE_BYTES).await?;
                let health: HealthResponse =
                    serde_json::from_slice(&body).map_err(|_| ClientError::MalformedStream)?;
                if health.status == "ok" {
                    Ok(HealthStatus::Ready)
                } else {
                    Err(ClientError::MalformedStream)
                }
            }
            status => Err(ClientError::UnexpectedStatus(status)),
        }
    }

    async fn complete_inner(
        &self,
        payload: Vec<u8>,
        started: Instant,
        budget: Duration,
        cancellation: CancellationToken,
        echoed_prefix: Option<&str>,
    ) -> Result<ObservedCompletion, ClientError> {
        if cancellation.is_cancelled() {
            return Err(ClientError::Cancelled);
        }
        let writing_deadline = started + budget;
        if Instant::now() >= writing_deadline {
            // A spelling attempt and its continuation share one budget. Do
            // not submit another inference job after that budget is spent.
            return Ok(ObservedCompletion::writing_budget_abstention(started, 0));
        }
        let request_body_bytes = payload.len();
        // An explicit budget can outlast the client's request timeout. The
        // writing deadline still ends first.
        let timeout = budget + WRITING_HTTP_GRACE;
        let send = self.post_runtime("/completion", payload, Some(timeout));
        let response = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            () = tokio::time::sleep_until(writing_deadline.into()) => {
                // llama.cpp can withhold streaming headers during prefill.
                // That wait consumes the same budget as the response stream.
                return Ok(ObservedCompletion::writing_budget_abstention(
                    started, request_body_bytes,
                ));
            },
            result = send => result?,
        };
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(&response, "text/event-stream")?;
        read_stream(
            response,
            request_body_bytes,
            started,
            budget,
            cancellation,
            echoed_prefix,
        )
        .await
    }

    async fn probe_authorization_challenge_inner(&self) -> Result<(), ClientError> {
        let body = serde_json::to_vec(&TokenizeRequest {
            content: AUTHORIZATION_CHALLENGE_TEXT,
            add_special: false,
        })
        .map_err(|_| ClientError::InvalidRequest)?;
        let response = self.post_runtime("/tokenize", body, None).await?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(&response, "application/json")?;
        let body = read_bounded_body(response, MAX_RESPONSE_BYTES).await?;
        let challenge: TokenizeResponse =
            serde_json::from_slice(&body).map_err(|_| ClientError::MalformedStream)?;
        if challenge.tokens.is_empty() {
            return Err(ClientError::MalformedStream);
        }
        Ok(())
    }
}

#[async_trait]
impl CompletionProvider for SemanticClient {
    fn kind(&self) -> ProviderKind {
        ProviderKind::LocalModel
    }

    async fn propose(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
    ) -> Result<Option<WritingProposal>, ProviderError> {
        self.propose_outcome(
            request,
            cancellation,
            allow_replacement,
            RequestTrigger::Automatic,
        )
        .await
        .map(ProviderOutcome::into_proposal)
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        trigger: RequestTrigger,
    ) -> Result<ProviderOutcome, ProviderError> {
        let started = Instant::now();
        let budget = trigger.writing_budget();
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        if allow_replacement
            && matches!(validate_request(&request), Ok(InputEligibility::Eligible))
            && request
                .language
                .as_deref()
                .and_then(crate::writing::WritingLanguage::from_tag)
                == Some(crate::writing::WritingLanguage::English)
        {
            if let Some(target) = crate::writing::correction_target(&request.before) {
                let word = target.word;
                if let Some(corrected) = crate::writing::unambiguous_correction(word) {
                    if cancellation.is_cancelled() {
                        return Err(ProviderError::Cancelled);
                    }
                    return Ok(ProviderOutcome::Proposal(WritingProposal {
                        text: format!("{corrected}{}", target.delimiter),
                        replace_before: Some(target.suffix.to_owned()),
                    }));
                }
                let payload = serde_json::to_vec(&crate::writing::correction_payload(word))
                    .map_err(|_| ProviderError::Unavailable)?;
                let observed = tokio::select! {
                    biased;
                    () = cancellation.cancelled() => return Err(ProviderError::Cancelled),
                    result = tokio::time::timeout_at(
                        (started + budget + WRITING_RESULT_GRACE).into(),
                        self.complete_inner(payload, started, budget, cancellation.clone(), None),
                    ) => result.unwrap_or(Err(ClientError::Timeout)),
                }
                .map_err(|error| match error {
                    ClientError::Cancelled => ProviderError::Cancelled,
                    _ => ProviderError::Unavailable,
                })?;
                if let Some(corrected) = observed.output.as_deref().map(str::trim) {
                    if crate::writing::valid_correction(word, corrected) {
                        return Ok(ProviderOutcome::Proposal(WritingProposal {
                            text: format!("{corrected}{}", target.delimiter),
                            replace_before: Some(target.suffix.to_owned()),
                        }));
                    }
                }
            }
        }
        let before = request.before.clone();
        let after = request.after.clone();
        let language = request.language.clone();
        let text = match self
            .complete_started(request, cancellation, started, budget)
            .await?
        {
            Ok(text) => text,
            Err(reason) => return Ok(ProviderOutcome::NoSuggestion(reason)),
        };
        let valid =
            crate::writing::validate_proposal(&before, &after, &text, language.as_deref()).is_ok();
        Ok(if valid {
            ProviderOutcome::Proposal(WritingProposal {
                text,
                replace_before: None,
            })
        } else {
            ProviderOutcome::NoSuggestion(NoSuggestionReason::OutputRejected)
        })
    }

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        self.complete_started(
            request,
            cancellation,
            Instant::now(),
            RequestTrigger::Automatic.writing_budget(),
        )
        .await
        .map(Result::ok)
    }
}

/// `path` if it is one lowercase segment such as `/completion`, so the
/// credential can reach only this runtime's own endpoints.
fn runtime_path(path: &str) -> Result<&str, ClientError> {
    let segment = path.strip_prefix('/').unwrap_or_default();
    if !segment.is_empty()
        && segment.len() <= MAX_RUNTIME_PATH_BYTES
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte == b'-')
    {
        Ok(path)
    } else {
        Err(ClientError::InvalidConfig("runtime_path"))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputEligibility {
    Eligible,
    Abstain,
}

fn validate_request(request: &ProviderRequest) -> Result<InputEligibility, ClientError> {
    if request.before.chars().count() > MAX_BEFORE_CHARS
        || request.after.chars().count() > MAX_AFTER_CHARS
    {
        return Err(ClientError::InvalidRequest);
    }
    let Some(language) = request.language.as_deref() else {
        return Ok(InputEligibility::Abstain);
    };
    if !valid_language_tag(language) {
        return Err(ClientError::InvalidRequest);
    }
    // A joiner not between two Arabic-script letters is an ordinary Persian
    // typing state (ZWNJ just typed), not a runtime failure: send nothing.
    if !crate::segment::valid_orthographic_joiners(&request.before)
        || crate::writing::WritingLanguage::from_tag(language).is_none()
        || request.before.is_empty()
        || !request.after.is_empty()
    {
        Ok(InputEligibility::Abstain)
    } else {
        Ok(InputEligibility::Eligible)
    }
}

fn valid_language_tag(value: &str) -> bool {
    (2..=35).contains(&value.chars().count())
        && value.split('-').all(|subtag| {
            !subtag.is_empty() && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

#[derive(Debug, Deserialize)]
struct HealthResponse {
    status: String,
}

#[derive(Debug, Serialize)]
struct TokenizeRequest {
    content: &'static str,
    add_special: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum StreamFinish {
    Stop,
    Length,
}

#[derive(Debug, Default)]
struct StreamAccumulator {
    output: String,
    ttft: Option<Duration>,
    finish: Option<StreamFinish>,
    invalid: bool,
}

impl StreamAccumulator {
    fn accept_event(&mut self, data: &str, started: Instant) -> Result<bool, ClientError> {
        let chunk: NativeStreamChunk =
            serde_json::from_str(data).map_err(|_| ClientError::MalformedStream)?;
        if chunk.index != 0 || self.finish.is_some() {
            return Err(ClientError::MalformedStream);
        }
        if !chunk.stop
            && (chunk.truncated.is_some()
                || chunk.stop_type.is_some()
                || chunk.stopped_limit.is_some()
                || chunk.stopped_word.is_some()
                || chunk.stopped_eos.is_some()
                || chunk.stopping_word.is_some())
        {
            return Err(ClientError::MalformedStream);
        }
        if !chunk.content.is_empty() && self.ttft.is_none() {
            self.ttft = Some(started.elapsed());
        }
        if self.output.len().saturating_add(chunk.content.len()) > MAX_RAW_OUTPUT_BYTES {
            self.invalid = true;
        } else {
            self.output.push_str(&chunk.content);
        }
        if !chunk.stop {
            return Ok(false);
        }
        let stopped_at_limit = chunk.truncated.unwrap_or(false)
            || chunk.stopped_limit.unwrap_or(false)
            || chunk
                .stop_type
                .as_deref()
                .is_some_and(|kind| kind == "limit");
        let stopped_at_word = chunk.stopped_word.unwrap_or(false)
            || chunk
                .stop_type
                .as_deref()
                .is_some_and(|kind| kind == "word");
        let stopped_at_eos = chunk.stopped_eos.unwrap_or(false)
            || chunk.stop_type.as_deref().is_some_and(|kind| kind == "eos");
        if chunk
            .stop_type
            .as_deref()
            .is_some_and(|kind| !matches!(kind, "word" | "eos" | "limit"))
            || usize::from(stopped_at_limit)
                + usize::from(stopped_at_word)
                + usize::from(stopped_at_eos)
                != 1
        {
            self.invalid = true;
        }
        if stopped_at_word {
            match chunk.stopping_word.as_deref() {
                Some(".") => {
                    if self.output.len().saturating_add(1) > MAX_RAW_OUTPUT_BYTES {
                        self.invalid = true;
                    } else {
                        self.output.push('.');
                    }
                }
                Some("\n") => {}
                _ => self.invalid = true,
            }
        } else if chunk
            .stopping_word
            .as_deref()
            .is_some_and(|word| !word.is_empty())
        {
            self.invalid = true;
        }
        self.finish = Some(if stopped_at_limit {
            StreamFinish::Length
        } else {
            StreamFinish::Stop
        });
        Ok(true)
    }
}

async fn read_stream(
    mut response: Response,
    request_body_bytes: usize,
    started: Instant,
    budget: Duration,
    cancellation: CancellationToken,
    echoed_prefix: Option<&str>,
) -> Result<ObservedCompletion, ClientError> {
    let mut pending = Vec::new();
    let mut response_body_bytes = 0_usize;
    let mut accumulator = StreamAccumulator::default();
    let mut saw_done = false;
    let writing_deadline = tokio::time::sleep_until((started + budget).into());
    tokio::pin!(writing_deadline);

    loop {
        let chunk = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            () = &mut writing_deadline => {
                let output = (!accumulator.invalid)
                    .then(|| crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix)
                        .and_then(crate::writing::available_complete_words))
                    .flatten();
                return Ok(ObservedCompletion {
                    disposition: if output.is_some() { CompletionDisposition::Suggested }
                        else { CompletionDisposition::ModelAbstained },
                    no_suggestion: output.is_none().then_some(NoSuggestionReason::BudgetStream),
                    output, ttft: accumulator.ttft, elapsed: started.elapsed(),
                    request_body_bytes, response_body_bytes,
                });
            },
            result = response.chunk() => result?,
        };
        let Some(chunk) = chunk else {
            break;
        };
        response_body_bytes = response_body_bytes.saturating_add(chunk.len());
        if response_body_bytes > MAX_RESPONSE_BYTES {
            return Err(ClientError::ResponseTooLarge);
        }
        pending.extend_from_slice(&chunk);
        while let Some((event_end, consumed)) = next_event_boundary(&pending) {
            let event = pending[..event_end].to_vec();
            pending.drain(..consumed);
            let data = event_data(&event)?;
            let Some(data) = data else {
                continue;
            };
            saw_done = accumulator.accept_event(&data, started)?;
            if crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix)
                .is_none_or(str::is_empty)
            {
                // Echo tokens are not a visible continuation token.
                accumulator.ttft = None;
            }
            if !accumulator.invalid {
                if let Some(output) =
                    crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix)
                        .and_then(|raw| crate::writing::complete_word_prefix(raw, 4, false))
                {
                    return Ok(ObservedCompletion {
                        disposition: CompletionDisposition::Suggested,
                        no_suggestion: None,
                        output: Some(output),
                        ttft: accumulator.ttft,
                        elapsed: started.elapsed(),
                        request_body_bytes,
                        response_body_bytes,
                    });
                }
            }
            if saw_done {
                break;
            }
        }
        if saw_done {
            break;
        }
    }

    if !saw_done || accumulator.finish.is_none() || !pending.iter().all(u8::is_ascii_whitespace) {
        return Err(ClientError::MalformedStream);
    }
    Ok(finish_observed_stream(
        &accumulator,
        echoed_prefix,
        started,
        request_body_bytes,
        response_body_bytes,
    ))
}

fn finish_observed_stream(
    accumulator: &StreamAccumulator,
    echoed_prefix: Option<&str>,
    started: Instant,
    request_body_bytes: usize,
    response_body_bytes: usize,
) -> ObservedCompletion {
    let elapsed = started.elapsed();
    let (disposition, output) = if accumulator.invalid {
        (CompletionDisposition::InvalidOutput, None)
    } else {
        // At a token limit the last token may contain only part of a word.
        // Keep only the prefix before its final separator.
        let continuation =
            crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix).unwrap_or("");
        let raw = if accumulator.finish == Some(StreamFinish::Length) {
            continuation
                .rsplit_once(char::is_whitespace)
                .map_or("", |(prefix, _)| prefix)
        } else {
            continuation
        };
        match crate::writing::complete_word_prefix(raw, 4, true) {
            Some(output) => (CompletionDisposition::Suggested, Some(output)),
            None => (CompletionDisposition::ModelAbstained, None),
        }
    };
    let output = output.filter(|value| {
        value.chars().count() <= MAX_SUGGESTION_CHARS
            && value.unicode_words().count() <= MAX_SUGGESTION_WORDS
    });
    let disposition = if matches!(disposition, CompletionDisposition::Suggested) && output.is_none()
    {
        CompletionDisposition::InvalidOutput
    } else {
        disposition
    };
    ObservedCompletion {
        disposition,
        no_suggestion: (disposition == CompletionDisposition::ModelAbstained)
            .then_some(NoSuggestionReason::ModelAbstained),
        output,
        ttft: accumulator.ttft,
        elapsed,
        request_body_bytes,
        response_body_bytes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_and_expired_writing_request_remains_cancelled() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("listener");
        let client = SemanticClient::new(
            SemanticClientConfig::new(
                listener.local_addr().expect("endpoint"),
                "budget-priority-fixture",
                "public-fixture-token",
            )
            .expect("config"),
        )
        .expect("client");
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        let expired = Instant::now()
            .checked_sub(Duration::from_secs(2))
            .expect("two seconds before test request");
        let budget = RequestTrigger::Explicit.writing_budget();
        assert!(matches!(
            client
                .complete_inner(Vec::new(), expired, budget, cancellation, None)
                .await,
            Err(ClientError::Cancelled)
        ));
        let abstention = client
            .complete_inner(Vec::new(), expired, budget, CancellationToken::new(), None)
            .await
            .expect("expired budget");
        assert_eq!(
            abstention.disposition(),
            CompletionDisposition::ModelAbstained
        );
        assert_eq!(
            abstention.no_suggestion_reason(),
            Some(NoSuggestionReason::BudgetPrefill)
        );
        assert_eq!(abstention.request_body_bytes(), 0);
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    }
}
