use std::fmt;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::metrics::NoSuggestionReason;
use crate::protocol::{
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_SUGGESTION_CHARS, MAX_SUGGESTION_WORDS, ProviderKind,
    valid_language_tag,
};
use crate::provider::{
    CompletionProvider, ProviderError, ProviderOutcome, ProviderReply, ProviderRequest,
    ProviderStats, RequestTrigger, WritingProposal,
};
use crate::spelling::Located;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;

use super::wire::{
    self, Method, NativeStreamChunk, NativeTimings, Response, StatusCode, TokenizeResponse,
    ensure_content_type, event_data, next_event_boundary, read_bounded_body,
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
/// Token probabilities kept per stream; generation asks for far fewer tokens.
const MAX_PROBABILITY_TOKENS: usize = 64;
// After the writing budget, the outer deadline still lets the reader return
// words salvaged at the budget, and the HTTP timeout never ends it first.
const WRITING_RESULT_GRACE: Duration = Duration::from_millis(20);
const WRITING_HTTP_GRACE: Duration = Duration::from_millis(100);
const AUTHORIZATION_CHALLENGE_TEXT: &str = "badi-owned-runtime-challenge";
const WARM_UP_PROMPT: &str = "Thank you for your";
const WARM_UP_TOKENS: u16 = 2;
// A non-streaming reply also echoes generation settings; its body is discarded.
const MAX_WARM_UP_RESPONSE_BYTES: usize = 64 * 1_024;
/// First-token candidates read for an alternative, and how many of them may
/// be continued before giving up.
const ALTERNATIVE_CANDIDATES: u8 = 8;
const ALTERNATIVE_ATTEMPTS: usize = 3;
// A non-streaming reply with candidate probabilities, and the settings echo.
const MAX_CANDIDATES_RESPONSE_BYTES: usize = 64 * 1_024;
/// Linux `sockaddr_un.sun_path` holds at most 108 bytes.
const MAX_UNIX_SOCKET_BYTES: usize = 108;

/// Where the owned writing runtime accepts its HTTP connection.
///
/// Production uses a Unix socket in the private runtime directory. Loopback
/// remains for tests and the Lab's stand-in servers.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RuntimeAddress {
    Loopback(SocketAddr),
    Unix(PathBuf),
}

/// `llama-<pid>.sock`, or the revision runtime's `llama-<pid>-revision.sock`,
/// in the private runtime directory. The pid keeps a test or a second broker
/// from replacing the socket the running service is using.
pub(crate) fn is_owned_runtime_socket_name(name: &str) -> bool {
    let Some(pid) = name
        .strip_prefix("llama-")
        .and_then(|rest| rest.strip_suffix(".sock"))
        .map(|rest| rest.strip_suffix("-revision").unwrap_or(rest))
    else {
        return false;
    };
    !pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit())
}

fn unix_socket_path_is_usable(path: &Path) -> bool {
    path.is_absolute()
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(is_owned_runtime_socket_name)
        && path.as_os_str().len() < MAX_UNIX_SOCKET_BYTES
}

/// How to reach and bound the owned writing runtime.
#[derive(Clone)]
pub struct SemanticClientConfig {
    address: RuntimeAddress,
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
            address: RuntimeAddress::Loopback(endpoint),
            model_alias: model_alias.into(),
            authorization: format!("Bearer {raw_token}"),
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
        };
        config.validate()?;
        Ok(config)
    }

    /// A client for the production runtime socket. The path must be absolute
    /// and end in `llama-<pid>.sock`, and it must fit in a Unix socket address.
    pub fn unix(
        path: impl Into<PathBuf>,
        model_alias: impl Into<String>,
        token: impl AsRef<str>,
    ) -> Result<Self, ClientError> {
        let mut config = Self::new(
            SocketAddr::from((std::net::Ipv4Addr::LOCALHOST, 1)),
            model_alias,
            token,
        )?;
        config.address = RuntimeAddress::Unix(path.into());
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
    pub const fn address(&self) -> &RuntimeAddress {
        &self.address
    }

    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    fn validate(&self) -> Result<(), ClientError> {
        match &self.address {
            RuntimeAddress::Loopback(endpoint) => {
                if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
                    return Err(ClientError::InvalidConfig("endpoint"));
                }
            }
            RuntimeAddress::Unix(path) => {
                if !unix_socket_path_is_usable(path) {
                    return Err(ClientError::InvalidConfig("endpoint"));
                }
            }
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
            .field("address", &self.address)
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
    /// The streamed text as the runtime sent it, before it is shaped into
    /// complete continuation words; `None` when the stream was invalid. Only
    /// a dictionary choice reads it: its answer follows an opened quote, with
    /// no space before the word.
    raw: Option<String>,
    /// The lowest pre-sampling log-probability among the shown text's
    /// tokens, when the request asked for probabilities and all were read.
    min_logprob: Option<f64>,
    ttft: Option<Duration>,
    elapsed: Duration,
    request_body_bytes: usize,
    response_body_bytes: usize,
    /// Newly evaluated prompt tokens (`prompt_n`); `None` when unknown.
    prompt_tokens: Option<u32>,
    /// Prompt tokens the runtime reused from its prefix cache.
    cache_tokens: Option<u32>,
    /// Tokens generated (the runtime's count, else streamed events seen).
    generated_tokens: u32,
    /// Bytes of the first generated token, a healed echo included, when the
    /// request asked for probabilities.
    first_token_bytes: Option<usize>,
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

    /// The first generated token as streamed, a healed echo included.
    fn first_token(&self) -> Option<&str> {
        self.raw.as_deref()?.get(..self.first_token_bytes?)
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

    #[must_use]
    pub const fn prompt_tokens(&self) -> Option<u32> {
        self.prompt_tokens
    }

    #[must_use]
    pub const fn cache_tokens(&self) -> Option<u32> {
        self.cache_tokens
    }

    #[must_use]
    pub const fn generated_tokens(&self) -> u32 {
        self.generated_tokens
    }

    fn stats(&self) -> ProviderStats {
        ProviderStats {
            ttft: self.ttft,
            prompt_tokens: self.prompt_tokens,
            cache_tokens: self.cache_tokens,
            generated_tokens: self.generated_tokens,
            // Clamped to the i32 range, so the conversion cannot overflow.
            #[allow(clippy::cast_possible_truncation)]
            min_logprob_hundredths: self
                .min_logprob
                .map(|minimum| (minimum * 100.0).round().clamp(-1.0e6, 0.0) as i32),
        }
    }

    fn language_abstention() -> Self {
        Self {
            disposition: CompletionDisposition::LanguageAbstained,
            no_suggestion: Some(NoSuggestionReason::RequestAbstained),
            output: None,
            raw: None,
            min_logprob: None,
            ttft: None,
            elapsed: Duration::ZERO,
            request_body_bytes: 0,
            response_body_bytes: 0,
            prompt_tokens: None,
            cache_tokens: None,
            generated_tokens: 0,
            first_token_bytes: None,
        }
    }

    fn writing_budget_abstention(started: Instant, request_body_bytes: usize) -> Self {
        // Match the stream deadline's no-output convention. This records a
        // spent writing budget, not a model refusal or a runtime health result.
        Self {
            disposition: CompletionDisposition::ModelAbstained,
            no_suggestion: Some(NoSuggestionReason::BudgetPrefill),
            output: None,
            raw: None,
            min_logprob: None,
            ttft: None,
            elapsed: started.elapsed(),
            request_body_bytes,
            response_body_bytes: 0,
            prompt_tokens: None,
            cache_tokens: None,
            generated_tokens: 0,
            first_token_bytes: None,
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
    /// The languages whose trailing space is healed for the loaded model.
    healing: Vec<crate::writing::WritingLanguage>,
    /// The person's language choices, shared by this client's clones.
    preferences: Arc<std::sync::RwLock<crate::writing::LanguagePreferences>>,
    /// The person's suggestion length, shared by this client's clones.
    length: Arc<std::sync::RwLock<crate::writing::SuggestionLength>>,
}

impl SemanticClient {
    pub fn new(config: SemanticClientConfig) -> Result<Self, ClientError> {
        config.validate()?;
        Ok(Self {
            config,
            request_observer: None,
            healing: crate::writing::TRAILING_SPACE_HEALING.to_vec(),
            preferences: Arc::default(),
            length: Arc::default(),
        })
    }

    /// This client healing the languages reviewed for the model whose
    /// weights hash to `model_sha256`.
    #[must_use]
    pub fn for_model(mut self, model_sha256: &str) -> Self {
        self.healing = crate::writing::trailing_space_healing(model_sha256);
        self
    }

    /// Applies the person's language choices to every clone of this client.
    pub fn set_language_preferences(&self, preferences: &crate::writing::LanguagePreferences) {
        match self.preferences.write() {
            Ok(mut guard) => preferences.clone_into(&mut guard),
            Err(poisoned) => preferences.clone_into(&mut poisoned.into_inner()),
        }
    }

    pub fn set_suggestion_length(&self, length: crate::writing::SuggestionLength) {
        match self.length.write() {
            Ok(mut guard) => *guard = length,
            Err(poisoned) => *poisoned.into_inner() = length,
        }
    }

    fn suggestion_length(&self) -> crate::writing::SuggestionLength {
        self.length
            .read()
            .map_or_else(|poisoned| *poisoned.into_inner(), |guard| *guard)
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
        match &self.config.address {
            RuntimeAddress::Loopback(endpoint) => {
                wire::exchange(*endpoint, request, self.config.connect_timeout, deadline).await
            }
            RuntimeAddress::Unix(path) => {
                wire::exchange_unix(path, request, self.config.connect_timeout, deadline).await
            }
        }
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
        let preferences = self.preferences.read().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |guard| guard.clone(),
        );
        // A language the person turned off is never sent to the model.
        if request
            .language
            .as_deref()
            .and_then(crate::writing::WritingLanguage::from_tag)
            .is_some_and(|language| !preferences.enables(language))
        {
            return Ok(ObservedCompletion::language_abstention());
        }
        let healed = preferences.healed(&self.healing);
        let length = self.suggestion_length();
        let plan = crate::writing::completion_plan(&request, &healed);
        // Healing can consume the whole window (one unfinished English word),
        // and an unhealed language can send only whitespace. Neither carries
        // context, and the runtime answers an empty prompt with a malformed
        // final chunk, so this is a request-side abstention, sent nowhere.
        // An unhealed prompt that ends in a space abstains too: no reviewed
        // model continued one usefully (see `CompletionPlan::sends_nothing`).
        if plan.sends_nothing() {
            return Ok(ObservedCompletion::language_abstention());
        }
        let run = |plan| {
            self.complete_plan(
                plan,
                &cancellation,
                started,
                budget,
                StreamShape {
                    echoed_prefix: None,
                    confidence_floor: Some(crate::writing::CONFIDENCE_FLOOR),
                    length,
                    first_token_must_extend: false,
                },
            )
        };
        let mut observed = run(plan).await?;
        // A healed lexicon word whose answer began with a shorter token is
        // taken as finished, within the same budget.
        if let Some(finished) = plan.finished_word(observed.first_token()) {
            observed = run(finished).await?;
        }
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

    /// Sends one completion plan and reads it as `shape` says, with the
    /// plan's echo, within the writing budget that began at `started`.
    async fn complete_plan(
        &self,
        plan: crate::writing::CompletionPlan<'_>,
        cancellation: &CancellationToken,
        started: Instant,
        budget: Duration,
        shape: StreamShape<'_>,
    ) -> Result<ObservedCompletion, ClientError> {
        let mut payload = plan.payload();
        // The confidence gate needs each token's probability before sampling.
        payload["n_probs"] = serde_json::json!(1);
        payload["post_sampling_probs"] = serde_json::json!(false);
        let payload = serde_json::to_vec(&payload).map_err(|_| ClientError::InvalidRequest)?;
        let operation = self.complete_inner(
            payload,
            started,
            budget,
            cancellation.clone(),
            StreamShape {
                echoed_prefix: plan.echo,
                first_token_must_extend: plan.finished_word_prompt.is_some(),
                ..shape
            },
        );
        let deadline = started + budget + WRITING_RESULT_GRACE;
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout_at(deadline.into(), operation) => {
                result.map_err(|_| ClientError::Timeout)?
            }
        }
    }

    /// An alternative continuation: the model's most likely first tokens
    /// after the same prompt a suggestion gets, the best whose word is not in
    /// `excluded`, continued greedily within the length setting. `None` when
    /// none fits. Explicit, so no confidence floor applies.
    async fn alternative_text(
        &self,
        request: &ProviderRequest,
        excluded: &[String],
        cancellation: &CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<Option<String>, ClientError> {
        if !matches!(validate_request(request)?, InputEligibility::Eligible) {
            return Ok(None);
        }
        let preferences = self.preferences.read().map_or_else(
            |poisoned| poisoned.into_inner().clone(),
            |guard| guard.clone(),
        );
        if request
            .language
            .as_deref()
            .and_then(crate::writing::WritingLanguage::from_tag)
            .is_none_or(|language| !preferences.enables(language))
        {
            return Ok(None);
        }
        let healed = preferences.healed(&self.healing);
        let plan = crate::writing::completion_plan(request, &healed);
        if plan.sends_nothing() {
            return Ok(None);
        }
        let echo = plan.echo.unwrap_or_default();
        let length = self.suggestion_length();
        let candidates = self
            .first_token_candidates(&plan, cancellation, started, budget)
            .await?;
        let first_word = |text: &str| {
            text.split_whitespace()
                .next()
                .map(|word| {
                    word.trim_end_matches(|c: char| !c.is_alphanumeric())
                        .to_lowercase()
                })
                .filter(|word| !word.is_empty())
        };
        // A token that is itself a word already shown is not continued.
        let admissible = candidates.into_iter().filter(|token| {
            token.len() > echo.len()
                && token.starts_with(echo)
                && !token.contains(['<', '>', '`', '\n', '\r'])
                && (plan.digits || !token.chars().any(char::is_numeric))
                && first_word(&token[echo.len()..]).is_none_or(|word| !excluded.contains(&word))
        });
        for token in admissible.take(ALTERNATIVE_ATTEMPTS) {
            let prompt = format!("{}{token}", plan.prompt);
            let follow = crate::writing::CompletionPlan {
                prompt: &prompt,
                echo: None,
                max_tokens: plan.max_tokens.saturating_sub(1).max(1),
                digits: plan.digits,
                finished_word_prompt: None,
            };
            let shape = StreamShape {
                length,
                ..StreamShape::default()
            };
            let rest = self
                .complete_plan(follow, cancellation, started, budget, shape)
                .await?
                .output
                .unwrap_or_default();
            let composed = format!("{token}{rest}");
            let Some(text) = crate::writing::complete_word_prefix(
                &composed[echo.len()..],
                length.max_words(),
                true,
            ) else {
                continue;
            };
            if first_word(&text).is_some_and(|word| !excluded.contains(&word)) {
                return Ok(Some(text));
            }
        }
        Ok(None)
    }

    /// The model's most likely first tokens after `plan`'s prompt, best
    /// first, read before sampling from one generated token.
    async fn first_token_candidates(
        &self,
        plan: &crate::writing::CompletionPlan<'_>,
        cancellation: &CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<Vec<String>, ClientError> {
        let mut payload = plan.payload();
        payload["n_predict"] = serde_json::json!(1);
        payload["n_probs"] = serde_json::json!(ALTERNATIVE_CANDIDATES);
        payload["post_sampling_probs"] = serde_json::json!(false);
        payload["stream"] = serde_json::json!(false);
        let body = serde_json::to_vec(&payload).map_err(|_| ClientError::InvalidRequest)?;
        let remaining = (started + budget).saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(Vec::new());
        }
        let exchange = async {
            let response = self
                .post_runtime("/completion", body, Some(remaining + WRITING_HTTP_GRACE))
                .await?;
            if response.status() != StatusCode::OK {
                return Err(ClientError::UnexpectedStatus(response.status()));
            }
            ensure_content_type(&response, "application/json")?;
            read_bounded_body(response, MAX_CANDIDATES_RESPONSE_BYTES).await
        };
        let body = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            result = tokio::time::timeout(remaining, exchange) => match result {
                Ok(body) => body?,
                Err(_) => return Ok(Vec::new()),
            },
        };
        let reply: serde_json::Value =
            serde_json::from_slice(&body).map_err(|_| ClientError::MalformedStream)?;
        Ok(first_token_texts(&reply))
    }

    /// The rest of the word at the caret from the request's vocabulary, in a
    /// language the person turned on. It needs no model, so it answers at
    /// once, and the model runs only when it abstains.
    fn vocabulary_completion(
        &self,
        request: &ProviderRequest,
        started: Instant,
    ) -> Option<WritingProposal> {
        let vocabulary = request.vocabulary.as_ref()?;
        let language = request
            .language
            .as_deref()
            .and_then(crate::writing::WritingLanguage::from_tag)?;
        let enabled = self
            .preferences
            .read()
            .is_ok_and(|preferences| preferences.enables(language));
        if !enabled || !matches!(validate_request(request), Ok(InputEligibility::Eligible)) {
            return None;
        }
        let outcome =
            crate::vocabulary::complete(&request.before, &request.after, language, vocabulary);
        trace_vocabulary(language.tag(), vocabulary, &outcome, started);
        outcome.ok().map(|completion| WritingProposal {
            text: completion.suffix,
            replace_before: None,
        })
    }

    async fn complete_started(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<(Result<String, NoSuggestionReason>, ProviderStats), ProviderError> {
        let observed = self
            .complete_observed_started(request, cancellation, started, budget)
            .await
            .map_err(|error| match error {
                ClientError::Cancelled => ProviderError::Cancelled,
                _ => ProviderError::Unavailable,
            })?;
        let stats = observed.stats();
        let abstention = observed
            .no_suggestion
            .unwrap_or(NoSuggestionReason::ModelAbstained);
        match observed.disposition {
            CompletionDisposition::Suggested => Ok((observed.output.ok_or(abstention), stats)),
            CompletionDisposition::ModelAbstained | CompletionDisposition::LanguageAbstained => {
                Ok((Err(abstention), stats))
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

    /// A spelling correction of the field's last word, when one is certain
    /// enough. English: a single lexicon candidate, else the model's
    /// validated word. Other enabled languages: the dictionary's single
    /// candidate, else the model's choice among its candidates.
    async fn correction(
        &self,
        request: &ProviderRequest,
        explicit: bool,
        cancellation: &CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<Option<WritingProposal>, ProviderError> {
        let spelling = crate::spelling::global();
        let Some(located) = spelling.locate(&request.before, request.language.as_deref(), explicit)
        else {
            return Ok(None);
        };
        let (target, corrected) = match located {
            Located::English(target) => {
                let unambiguous = crate::writing::unambiguous_correction(target.word);
                let asked = unambiguous.is_none();
                let answer = if asked {
                    self.spelling_completion(
                        &crate::writing::correction_payload(target.word),
                        false,
                        cancellation,
                        started,
                        budget,
                    )
                    .await?
                } else {
                    None
                };
                let corrected = unambiguous.or_else(|| {
                    answer.clone().filter(|corrected| {
                        crate::writing::valid_correction(target.word, corrected)
                    })
                });
                trace_spelling(
                    "en",
                    target.word,
                    None,
                    asked,
                    answer.as_deref(),
                    corrected.as_deref(),
                    started,
                );
                (target, corrected)
            }
            Located::Dictionary { language, target } => {
                let word = target.word.to_owned();
                let candidates =
                    tokio::task::spawn_blocking(move || spelling.candidates(language, &word))
                        .await
                        .map_err(|_| ProviderError::Unavailable)?;
                let asked = candidates.len() > 1;
                let answer = if asked {
                    self.spelling_completion(
                        &crate::spelling::choice_payload(&request.before, target.word, &candidates),
                        true,
                        cancellation,
                        started,
                        budget,
                    )
                    .await?
                } else {
                    None
                };
                let corrected = if let [only] = candidates.as_slice() {
                    Some(only.clone())
                } else {
                    answer
                        .as_deref()
                        .and_then(|output| crate::spelling::chosen(output, &candidates).cloned())
                };
                trace_spelling(
                    language.tag(),
                    target.word,
                    Some(&candidates),
                    asked,
                    answer.as_deref(),
                    corrected.as_deref(),
                    started,
                );
                (target, corrected)
            }
        };
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        Ok(corrected.map(|corrected| WritingProposal {
            text: format!("{corrected}{}", target.delimiter),
            replace_before: Some(target.suffix.to_owned()),
        }))
    }

    /// The model's trimmed answer to a spelling prompt, within the writing
    /// budget the continuation shares: its complete words, or with `raw` the
    /// streamed text itself.
    async fn spelling_completion(
        &self,
        payload: &serde_json::Value,
        raw: bool,
        cancellation: &CancellationToken,
        started: Instant,
        budget: Duration,
    ) -> Result<Option<String>, ProviderError> {
        let payload = serde_json::to_vec(payload).map_err(|_| ProviderError::Unavailable)?;
        let observed = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ProviderError::Cancelled),
            result = tokio::time::timeout_at(
                (started + budget + WRITING_RESULT_GRACE).into(),
                self.complete_inner(payload, started, budget, cancellation.clone(), StreamShape::default()),
            ) => result.unwrap_or(Err(ClientError::Timeout)),
        }
        .map_err(|error| match error {
            ClientError::Cancelled => ProviderError::Cancelled,
            _ => ProviderError::Unavailable,
        })?;
        let answer = if raw { observed.raw } else { observed.output };
        Ok(answer
            .as_deref()
            .map(str::trim)
            .filter(|answer| !answer.is_empty())
            .map(str::to_owned))
    }

    async fn complete_inner(
        &self,
        payload: Vec<u8>,
        started: Instant,
        budget: Duration,
        cancellation: CancellationToken,
        shape: StreamShape<'_>,
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
            shape,
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

    fn set_language_preferences(&self, preferences: &crate::writing::LanguagePreferences) {
        Self::set_language_preferences(self, preferences);
    }

    fn set_suggestion_length(&self, length: crate::settings::SuggestionLength) {
        Self::set_suggestion_length(self, length);
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
        .map(|reply| reply.outcome.into_proposal())
    }

    async fn propose_outcome(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        allow_replacement: bool,
        trigger: RequestTrigger,
    ) -> Result<ProviderReply, ProviderError> {
        let started = Instant::now();
        let budget = trigger.writing_budget();
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        // Typed letters that begin a word in the window or the writer's earlier
        // writing are no typo, so the vocabulary answers before spelling.
        if let Some(proposal) = self.vocabulary_completion(&request, started) {
            return Ok(ProviderOutcome::Proposal(proposal).into());
        }
        // A correction replaces the last word of the field; mid-text it stays
        // editor-owned. It needs a well-formed request, not a language the
        // model continues in: spelling has its own languages.
        if allow_replacement
            && request.after.is_empty()
            && !request.before.is_empty()
            && validate_request(&request).is_ok()
        {
            // A spelling-only field's own check counts as asked for.
            let explicit = !matches!(trigger, RequestTrigger::Automatic);
            if let Some(proposal) = self
                .correction(&request, explicit, &cancellation, started, budget)
                .await?
            {
                return Ok(ProviderOutcome::Proposal(proposal).into());
            }
        }
        let before = request.before.clone();
        let after = request.after.clone();
        let language = request.language.clone();
        let (text, stats) = match self
            .complete_started(request, cancellation, started, budget)
            .await?
        {
            (Ok(text), stats) => (text, stats),
            (Err(reason), stats) => {
                return Ok(ProviderReply {
                    outcome: ProviderOutcome::NoSuggestion(reason),
                    stats: Some(stats),
                });
            }
        };
        let outcome = match crate::writing::line_rest(&after) {
            Some(line) => crate::writing::mid_line_insertion(&before, line, &text),
            None => Some(text),
        }
        .and_then(|text| {
            crate::writing::validate_proposal(&before, &after, &text, language.as_deref())
                .ok()
                .map(|()| {
                    ProviderOutcome::Proposal(WritingProposal {
                        text,
                        replace_before: None,
                    })
                })
        })
        .unwrap_or(ProviderOutcome::NoSuggestion(
            NoSuggestionReason::OutputRejected,
        ));
        Ok(ProviderReply {
            outcome,
            stats: Some(stats),
        })
    }

    async fn alternative(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
        trigger: RequestTrigger,
        excluded: Vec<String>,
    ) -> Result<ProviderReply, ProviderError> {
        let started = Instant::now();
        let budget = trigger.writing_budget();
        if cancellation.is_cancelled() {
            return Err(ProviderError::Cancelled);
        }
        let text = self
            .alternative_text(&request, &excluded, &cancellation, started, budget)
            .await
            .map_err(|error| match error {
                ClientError::Cancelled => ProviderError::Cancelled,
                _ => ProviderError::Unavailable,
            })?;
        let outcome = text
            .filter(|text| {
                crate::writing::line_rest(&request.after).is_none()
                    && request
                        .language
                        .as_deref()
                        .and_then(crate::writing::WritingLanguage::from_tag)
                        .is_some_and(|language| language.accepts_output(text))
                    && !crate::writing::introduces_unsupported_fact(&request.before, text)
                    && crate::writing::validate_proposal(
                        &request.before,
                        &request.after,
                        text,
                        request.language.as_deref(),
                    )
                    .is_ok()
            })
            .map_or(
                ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained),
                |text| {
                    ProviderOutcome::Proposal(WritingProposal {
                        text,
                        replace_before: None,
                    })
                },
            );
        Ok(outcome.into())
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
        .map(|(text, _stats)| text.ok())
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
        || crate::writing::caret_inside_word(&request.before, &request.after)
    {
        Ok(InputEligibility::Abstain)
    } else {
        Ok(InputEligibility::Eligible)
    }
}

/// One `vocabulary` trace line per request with vocabulary: how many window
/// and learned words it carried, and the outcome with its sources. The word
/// itself is text, kept only in a `--text` run.
fn trace_vocabulary(
    language: &str,
    vocabulary: &crate::vocabulary::Vocabulary,
    outcome: &Result<crate::vocabulary::Completion, crate::vocabulary::Abstention>,
    started: Instant,
) {
    crate::debug_trace::emit("vocabulary", |entry| {
        entry.put("language", language);
        entry.put("window_words", vocabulary.window.len());
        entry.put("learned_words", vocabulary.learned.len());
        match outcome {
            Ok(completion) => {
                entry.put("outcome", "completed");
                entry.put(
                    "sources",
                    completion
                        .sources
                        .iter()
                        .map(|source| source.as_str())
                        .collect::<Vec<_>>(),
                );
                entry.text("word", &completion.word);
            }
            Err(abstention) => entry.put("outcome", abstention.as_str()),
        }
        entry.put(
            "ms",
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
    });
}

/// One `spelling` trace line per located misspelling: its language, how
/// many dictionary candidates it had, and whether a correction resulted. The
/// word, candidates and model answer are text, kept only in a `--text` run.
fn trace_spelling(
    language: &str,
    word: &str,
    candidates: Option<&[String]>,
    asked: bool,
    answer: Option<&str>,
    corrected: Option<&str>,
    started: Instant,
) {
    crate::debug_trace::emit("spelling", |entry| {
        entry.put("language", language);
        if let Some(candidates) = candidates {
            entry.put("candidates", candidates.len());
            entry.text("candidate_words", &candidates.join(", "));
        }
        entry.put("asked_model", asked);
        entry.put("corrected", corrected.is_some());
        entry.put(
            "ms",
            u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        entry.text("word", word);
        if let Some(answer) = answer {
            entry.text("answer", answer);
        }
        if let Some(corrected) = corrected {
            entry.text("correction", corrected);
        }
    });
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
    /// Streamed events that carried content (one per generated token).
    tokens: u32,
    /// The latest per-token timings the runtime reported.
    timings: Option<NativeTimings>,
    /// Byte length and pre-sampling log-probability of each content token,
    /// in order, when the request asked for probabilities.
    token_logprobs: Vec<(usize, f64)>,
    /// Bytes of streamed content, before a stop period the provider appends.
    content_bytes: usize,
    /// A content token arrived without a readable probability.
    probabilities_unknown: bool,
}

impl StreamAccumulator {
    /// Newly evaluated prompt tokens, cache-reused prompt tokens and
    /// generated tokens, as far as the stream reported them.
    fn counts(&self) -> (Option<u32>, Option<u32>, u32) {
        let generated = self
            .timings
            .and_then(|timings| timings.predicted_n)
            .unwrap_or(self.tokens);
        (
            self.timings.and_then(|timings| timings.prompt_n),
            self.timings.and_then(|timings| timings.cache_n),
            generated,
        )
    }
}

impl StreamAccumulator {
    /// Every content token arrived with a probability, and together they
    /// cover the streamed content exactly.
    fn probabilities_known(&self) -> bool {
        let covered: usize = self.token_logprobs.iter().map(|(bytes, _)| bytes).sum();
        !self.probabilities_unknown && covered == self.content_bytes
    }

    fn first_token_bytes(&self) -> Option<usize> {
        self.token_logprobs.first().map(|&(bytes, _)| bytes)
    }

    /// Whether the first streamed token, once known, fails to begin with the
    /// whole `echo`; unknown probabilities leave nothing to check it by.
    fn first_token_breaks(&self, echo: Option<&str>) -> bool {
        let first = self
            .first_token_bytes()
            .and_then(|bytes| self.output.get(..bytes));
        (self.probabilities_unknown || first.is_some())
            && !first.is_some_and(|token| echo.is_some_and(|echo| token.starts_with(echo)))
    }

    /// Records each streamed token's probability. A stop token whose text
    /// the runtime withholds (no bytes) is not part of the output.
    fn accept_probabilities(&mut self, value: Option<&serde_json::Value>, content: &str) {
        let Some(entries) = value.and_then(serde_json::Value::as_array) else {
            if !content.is_empty() {
                self.probabilities_unknown = true;
            }
            return;
        };
        for entry in entries {
            let bytes = entry
                .get("bytes")
                .and_then(serde_json::Value::as_array)
                .map(Vec::len)
                .or_else(|| {
                    entry
                        .get("token")
                        .and_then(serde_json::Value::as_str)
                        .map(str::len)
                });
            let logprob = entry.get("logprob").and_then(serde_json::Value::as_f64);
            match (bytes, logprob) {
                (Some(0), _) => {}
                (Some(bytes), Some(logprob))
                    if logprob.is_finite()
                        && logprob <= 0.0
                        && self.token_logprobs.len() < MAX_PROBABILITY_TOKENS =>
                {
                    self.token_logprobs.push((bytes, logprob));
                }
                _ => self.probabilities_unknown = true,
            }
        }
    }

    /// The lowest log-probability among the tokens that make up `shown_bytes`
    /// of output after an `echo_bytes` healed echo; a token that straddles
    /// the echo counts. `None` when any probability is missing or the tokens
    /// do not cover the streamed content exactly.
    fn shown_min_logprob(&self, echo_bytes: usize, shown_bytes: usize) -> Option<f64> {
        if !self.probabilities_known() {
            return None;
        }
        let end = echo_bytes.saturating_add(shown_bytes);
        let mut offset = 0;
        let mut minimum: Option<f64> = None;
        for &(bytes, logprob) in &self.token_logprobs {
            let begin = offset;
            offset += bytes;
            if offset <= echo_bytes {
                continue;
            }
            if begin >= end {
                break;
            }
            minimum = Some(minimum.map_or(logprob, |minimum| minimum.min(logprob)));
        }
        minimum
    }

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
        if !chunk.content.is_empty() {
            self.tokens = self.tokens.saturating_add(1);
        }
        if chunk.timings.is_some() {
            self.timings = chunk.timings;
        }
        self.accept_probabilities(chunk.completion_probabilities.as_ref(), &chunk.content);
        self.content_bytes = self.content_bytes.saturating_add(chunk.content.len());
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

fn observed_completion(
    accumulator: &StreamAccumulator,
    disposition: CompletionDisposition,
    no_suggestion: Option<NoSuggestionReason>,
    output: Option<String>,
    started: Instant,
    request_body_bytes: usize,
    response_body_bytes: usize,
) -> ObservedCompletion {
    let (prompt_tokens, cache_tokens, generated_tokens) = accumulator.counts();
    ObservedCompletion {
        disposition,
        no_suggestion,
        output,
        raw: (!accumulator.invalid).then(|| accumulator.output.clone()),
        min_logprob: None,
        ttft: accumulator.ttft,
        elapsed: started.elapsed(),
        request_body_bytes,
        response_body_bytes,
        prompt_tokens,
        cache_tokens,
        generated_tokens,
        first_token_bytes: accumulator.first_token_bytes(),
    }
}

/// The candidate texts of a non-streaming reply's first token, best first:
/// its `top_logprobs`, each read from its bytes when they are valid UTF-8.
fn first_token_texts(reply: &serde_json::Value) -> Vec<String> {
    let Some(candidates) = reply
        .get("completion_probabilities")
        .and_then(|value| value.get(0))
        .and_then(|first| first.get("top_logprobs"))
        .and_then(serde_json::Value::as_array)
    else {
        return Vec::new();
    };
    let mut scored: Vec<(f64, String)> = candidates
        .iter()
        .filter_map(|candidate| {
            let logprob = candidate
                .get("logprob")?
                .as_f64()
                .filter(|value| value.is_finite())?;
            let text = candidate
                .get("bytes")
                .and_then(serde_json::Value::as_array)
                .and_then(|bytes| {
                    bytes
                        .iter()
                        .map(|byte| byte.as_u64().and_then(|byte| u8::try_from(byte).ok()))
                        .collect::<Option<Vec<u8>>>()
                })
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .or_else(|| candidate.get("token")?.as_str().map(str::to_owned))?;
            Some((logprob, text))
        })
        .collect();
    scored.sort_by(|left, right| right.0.total_cmp(&left.0));
    scored.into_iter().map(|(_, text)| text).collect()
}

/// How a streamed completion becomes a suggestion: the healed echo to strip,
/// the confidence floor when gated, and how many words it may show.
#[derive(Clone, Copy, Default)]
struct StreamShape<'a> {
    echoed_prefix: Option<&'a str>,
    confidence_floor: Option<f64>,
    length: crate::writing::SuggestionLength,
    /// The echo is a typed lexicon word: stop reading as soon as the first
    /// token shows it does not write that whole word, so the unhealed
    /// request keeps the rest of the budget.
    first_token_must_extend: bool,
}

/// What the stream had produced when the writing budget ran out: its
/// complete words, if any, through the same confidence gate.
fn deadline_observation(
    accumulator: &StreamAccumulator,
    shape: StreamShape<'_>,
    started: Instant,
    request_body_bytes: usize,
    response_body_bytes: usize,
) -> ObservedCompletion {
    let output = (!accumulator.invalid)
        .then(|| {
            crate::writing::strip_healed_prefix(&accumulator.output, shape.echoed_prefix).and_then(
                |raw| crate::writing::available_complete_words(raw, shape.length.max_words()),
            )
        })
        .flatten();
    let disposition = if output.is_some() {
        CompletionDisposition::Suggested
    } else {
        CompletionDisposition::ModelAbstained
    };
    let no_suggestion = output.is_none().then_some(NoSuggestionReason::BudgetStream);
    gate_confidence(
        observed_completion(
            accumulator,
            disposition,
            no_suggestion,
            output,
            started,
            request_body_bytes,
            response_body_bytes,
        ),
        accumulator,
        shape,
    )
}

#[allow(clippy::too_many_lines)]
async fn read_stream(
    mut response: Response,
    request_body_bytes: usize,
    started: Instant,
    budget: Duration,
    cancellation: CancellationToken,
    shape: StreamShape<'_>,
) -> Result<ObservedCompletion, ClientError> {
    let echoed_prefix = shape.echoed_prefix;
    let words = shape.length.max_words();
    let gate = |observed: ObservedCompletion, accumulator: &StreamAccumulator| {
        gate_confidence(observed, accumulator, shape)
    };
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
                return Ok(deadline_observation(
                    &accumulator, shape, started, request_body_bytes, response_body_bytes,
                ));
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
            if shape.first_token_must_extend && accumulator.first_token_breaks(echoed_prefix) {
                return Ok(observed_completion(
                    &accumulator,
                    CompletionDisposition::ModelAbstained,
                    Some(NoSuggestionReason::ModelAbstained),
                    None,
                    started,
                    request_body_bytes,
                    response_body_bytes,
                ));
            }
            if crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix)
                .is_none_or(str::is_empty)
            {
                // Echo tokens are not a visible continuation token.
                accumulator.ttft = None;
            }
            if !accumulator.invalid {
                if let Some(output) =
                    crate::writing::strip_healed_prefix(&accumulator.output, echoed_prefix)
                        .and_then(|raw| crate::writing::complete_word_prefix(raw, words, false))
                {
                    return Ok(gate(
                        observed_completion(
                            &accumulator,
                            CompletionDisposition::Suggested,
                            None,
                            Some(output),
                            started,
                            request_body_bytes,
                            response_body_bytes,
                        ),
                        &accumulator,
                    ));
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
    Ok(gate(
        finish_observed_stream(
            &accumulator,
            echoed_prefix,
            words,
            started,
            request_body_bytes,
            response_body_bytes,
        ),
        &accumulator,
    ))
}

/// Applies the confidence floor to a finished observation: a suggestion is
/// kept only when every token of its shown text was at least as likely as
/// the floor; unknown likelihood fails closed. Without a floor nothing
/// changes.
fn gate_confidence(
    mut observed: ObservedCompletion,
    accumulator: &StreamAccumulator,
    shape: StreamShape<'_>,
) -> ObservedCompletion {
    let Some(floor) = shape.confidence_floor else {
        return observed;
    };
    let Some(output) = observed.output.as_deref() else {
        return observed;
    };
    let echo_bytes = shape.echoed_prefix.map_or(0, str::len);
    // The token that writes a typed lexicon word (` whether` after `whet`)
    // was chosen among those the stem allows; its probability is how likely
    // that word was before anything was typed, so it is not counted. A
    // suggestion that only finishes the word is judged by that choice and
    // the lexicon alone.
    let counted_from = accumulator
        .first_token_bytes()
        .filter(|&bytes| shape.first_token_must_extend && bytes > echo_bytes)
        .unwrap_or(echo_bytes);
    let shown_end = echo_bytes + output.len();
    let minimum =
        accumulator.shown_min_logprob(counted_from, shown_end.saturating_sub(counted_from));
    let word_only =
        counted_from > echo_bytes && counted_from >= shown_end && accumulator.probabilities_known();
    observed.min_logprob = minimum;
    if !word_only && minimum.is_none_or(|minimum| minimum < floor) {
        observed.disposition = CompletionDisposition::ModelAbstained;
        observed.no_suggestion = Some(NoSuggestionReason::LowConfidence);
        observed.output = None;
    }
    observed
}

fn finish_observed_stream(
    accumulator: &StreamAccumulator,
    echoed_prefix: Option<&str>,
    words: usize,
    started: Instant,
    request_body_bytes: usize,
    response_body_bytes: usize,
) -> ObservedCompletion {
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
        match crate::writing::complete_word_prefix(raw, words, true) {
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
    observed_completion(
        accumulator,
        disposition,
        (disposition == CompletionDisposition::ModelAbstained)
            .then_some(NoSuggestionReason::ModelAbstained),
        output,
        started,
        request_body_bytes,
        response_body_bytes,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_counts_come_from_per_token_timings() {
        let started = Instant::now();
        let mut accumulator = StreamAccumulator::default();
        assert_eq!(accumulator.counts(), (None, None, 0));
        accumulator
            .accept_event(
                r#"{"index":0,"content":" for","stop":false,"timings":{"prompt_n":41,"cache_n":33,"predicted_n":1}}"#,
                started,
            )
            .expect("token event");
        assert_eq!(accumulator.counts(), (Some(41), Some(33), 1));
        assert!(accumulator.ttft.is_some());
        accumulator
            .accept_event(
                r#"{"index":0,"content":" the","stop":false,"timings":{"prompt_n":41,"cache_n":33,"predicted_n":2}}"#,
                started,
            )
            .expect("token event");
        // Without timings the generated count falls back to token events.
        accumulator.timings = None;
        assert_eq!(accumulator.counts(), (None, None, 2));

        // Malformed optional statistics stay unknown instead of rejecting
        // an otherwise valid token event.
        let mut malformed = StreamAccumulator::default();
        malformed
            .accept_event(
                r#"{"index":0,"content":" for","stop":false,"timings":[]}"#,
                started,
            )
            .expect("token event with malformed timings");
        malformed
            .accept_event(
                r#"{"index":0,"content":" the","stop":false,"timings":{"prompt_n":-1,"cache_n":"33","predicted_n":1.5}}"#,
                started,
            )
            .expect("token event with malformed timing fields");
        assert_eq!(malformed.counts(), (None, None, 2));
    }

    #[test]
    fn text_after_the_caret_is_eligible_unless_the_caret_splits_a_word() {
        let request = |before: &str, after: &str| ProviderRequest {
            before: before.to_owned(),
            after: after.to_owned(),
            language: Some("en".to_owned()),
            vocabulary: None,
        };
        for (before, after, expected) in [
            ("Please find the", "", InputEligibility::Eligible),
            (
                "Please find the",
                " report today.",
                InputEligibility::Eligible,
            ),
            (
                "Please find the ",
                "report today.",
                InputEligibility::Eligible,
            ),
            ("Please find the upd", ".", InputEligibility::Eligible),
            (
                "Please find the rep",
                "ort today.",
                InputEligibility::Abstain,
            ),
        ] {
            assert_eq!(
                validate_request(&request(before, after)).expect("valid request"),
                expected,
                "{before:?} | {after:?}"
            );
        }
    }

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
                .complete_inner(
                    Vec::new(),
                    expired,
                    budget,
                    cancellation,
                    StreamShape::default()
                )
                .await,
            Err(ClientError::Cancelled)
        ));
        let abstention = client
            .complete_inner(
                Vec::new(),
                expired,
                budget,
                CancellationToken::new(),
                StreamShape::default(),
            )
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
