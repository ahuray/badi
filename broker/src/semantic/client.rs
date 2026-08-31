use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use crate::protocol::{
    MAX_AFTER_CHARS, MAX_BEFORE_CHARS, MAX_SUGGESTION_CHARS, MAX_SUGGESTION_WORDS, ProviderKind,
};
use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use reqwest::{Client, Response, StatusCode, Url};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use unicode_script::{Script, UnicodeScript};
use unicode_segmentation::UnicodeSegmentation;

pub const PROMPT_CONTRACT_ID: &str = "badi.semantic.inline-en.native-prefix.dev1";
pub const MAX_OUTPUT_TOKENS: u16 = 8;
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_000);
pub const MAX_CONNECT_TIMEOUT: Duration = Duration::from_secs(1);
pub const MAX_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_200);
pub const MAX_RESPONSE_BYTES: usize = 16 * 1_024;

const MAX_MODEL_ALIAS_BYTES: usize = 256;
const MAX_TOKEN_BYTES: usize = 4_096;
const MAX_RAW_OUTPUT_BYTES: usize = 1_024;
const AUTHORIZATION_CHALLENGE_TEXT: &str = "badi-owned-runtime-challenge";
const PROMPT_FORMAT_CONTRACT: &str = "{\"after\":\"must_be_empty\",\"language\":\"en_or_en_subtag\",\"prompt\":\"raw_before_caret\",\"transport\":\"llama_cpp_native_completion\"}";
const SAMPLING_CONTRACT: &str = "{\"cache_prompt\":false,\"n_predict\":8,\"seed\":42,\"stop\":[\".\",\"\\n\"],\"stream\":true,\"temperature\":0.0}";

#[must_use]
pub fn prompt_contract_sha256() -> String {
    let mut hasher = Sha256::new();
    for part in [
        PROMPT_CONTRACT_ID,
        PROMPT_FORMAT_CONTRACT,
        SAMPLING_CONTRACT,
    ] {
        hasher.update(part.as_bytes());
        hasher.update(b"\0");
    }
    encode_lower_hex(hasher.finalize())
}

#[derive(Clone)]
pub struct SemanticClientConfig {
    endpoint: SocketAddr,
    model_alias: String,
    authorization: HeaderValue,
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
        if raw_token.is_empty() || raw_token.len() > MAX_TOKEN_BYTES {
            return Err(ClientError::InvalidConfig("token"));
        }
        let mut authorization = HeaderValue::from_str(&format!("Bearer {raw_token}"))
            .map_err(|_| ClientError::InvalidConfig("token"))?;
        authorization.set_sensitive(true);
        let config = Self {
            endpoint,
            model_alias: model_alias.into(),
            authorization,
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
    Truncated,
}

#[derive(Clone, Debug)]
pub struct ObservedCompletion {
    disposition: CompletionDisposition,
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
            output: None,
            ttft: None,
            elapsed: Duration::ZERO,
            request_body_bytes: 0,
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
    #[error("failed to construct the loopback endpoint")]
    InvalidEndpoint,
    #[error("failed to construct the bounded HTTP client")]
    Client(#[source] reqwest::Error),
    #[error("semantic HTTP transport failed")]
    Transport(#[source] reqwest::Error),
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
}

#[derive(Clone, Debug)]
pub struct SemanticClient {
    config: SemanticClientConfig,
    client: Client,
    health_url: Url,
    challenge_url: Url,
    completion_url: Url,
}

impl SemanticClient {
    pub fn new(config: SemanticClientConfig) -> Result<Self, ClientError> {
        config.validate()?;
        let base = Url::parse(&format!("http://{}/", config.endpoint))
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let health_url = base
            .join("health")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let challenge_url = base
            .join("tokenize")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let completion_url = base
            .join("completion")
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(config.connect_timeout)
            .timeout(config.request_timeout)
            .pool_max_idle_per_host(1)
            .build()
            .map_err(ClientError::Client)?;
        Ok(Self {
            config,
            client,
            health_url,
            challenge_url,
            completion_url,
        })
    }

    #[must_use]
    pub const fn config(&self) -> &SemanticClientConfig {
        &self.config
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

    pub async fn complete_observed(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<ObservedCompletion, ClientError> {
        match validate_english_request(&request)? {
            InputEligibility::Abstain => return Ok(ObservedCompletion::language_abstention()),
            InputEligibility::Eligible => {}
        }

        // The language boundary deliberately precedes construction and JSON
        // serialization. A rejected request therefore cannot allocate an HTTP
        // payload or send a runtime request/body byte.
        let payload = serde_json::to_vec(&NativeStreamingRequest::new(&request))
            .map_err(|_| ClientError::InvalidRequest)?;
        let started = Instant::now();
        let operation = self.complete_inner(payload, started, cancellation.clone());
        tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout(self.config.request_timeout(), operation) => {
                result.map_err(|_| ClientError::Timeout)?
            }
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

    async fn probe_health_inner(&self) -> Result<HealthStatus, ClientError> {
        let response = self
            .client
            .get(self.health_url.clone())
            .header(AUTHORIZATION, self.config.authorization.clone())
            .send()
            .await
            .map_err(transport_error)?;
        match response.status() {
            StatusCode::SERVICE_UNAVAILABLE => Ok(HealthStatus::Loading),
            StatusCode::OK => {
                ensure_content_type(response.headers(), "application/json")?;
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
        cancellation: CancellationToken,
    ) -> Result<ObservedCompletion, ClientError> {
        let request_body_bytes = payload.len();
        let response = self
            .client
            .post(self.completion_url.clone())
            .header(AUTHORIZATION, self.config.authorization.clone())
            .header(CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .map_err(transport_error)?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(response.headers(), "text/event-stream")?;
        read_stream(response, request_body_bytes, started, cancellation).await
    }

    async fn probe_authorization_challenge_inner(&self) -> Result<(), ClientError> {
        let response = self
            .client
            .post(self.challenge_url.clone())
            .header(AUTHORIZATION, self.config.authorization.clone())
            .json(&TokenizeRequest {
                content: AUTHORIZATION_CHALLENGE_TEXT,
                add_special: false,
            })
            .send()
            .await
            .map_err(transport_error)?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(response.headers(), "application/json")?;
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

    async fn complete(
        &self,
        request: ProviderRequest,
        cancellation: CancellationToken,
    ) -> Result<Option<String>, ProviderError> {
        let observed = self
            .complete_observed(request, cancellation)
            .await
            .map_err(|error| match error {
                ClientError::Cancelled => ProviderError::Cancelled,
                _ => ProviderError::Unavailable,
            })?;
        match observed.disposition {
            CompletionDisposition::Suggested => Ok(observed.output),
            CompletionDisposition::ModelAbstained | CompletionDisposition::LanguageAbstained => {
                Ok(None)
            }
            CompletionDisposition::InvalidOutput | CompletionDisposition::Truncated => {
                Err(ProviderError::Unavailable)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InputEligibility {
    Eligible,
    Abstain,
}

fn validate_english_request(request: &ProviderRequest) -> Result<InputEligibility, ClientError> {
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
    if language
        .split('-')
        .next()
        .is_some_and(|primary| primary.eq_ignore_ascii_case("en"))
    {
        if request.before.is_empty() || !request.after.is_empty() {
            Ok(InputEligibility::Abstain)
        } else {
            Ok(InputEligibility::Eligible)
        }
    } else {
        Ok(InputEligibility::Abstain)
    }
}

fn valid_language_tag(value: &str) -> bool {
    (2..=35).contains(&value.chars().count())
        && value.split('-').all(|subtag| {
            !subtag.is_empty() && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

#[derive(Debug, Serialize)]
struct NativeStreamingRequest<'a> {
    prompt: &'a str,
    n_predict: u16,
    temperature: f32,
    stop: [&'static str; 2],
    stream: bool,
    seed: u32,
    cache_prompt: bool,
}

impl<'a> NativeStreamingRequest<'a> {
    fn new(request: &'a ProviderRequest) -> Self {
        Self {
            prompt: &request.before,
            n_predict: MAX_OUTPUT_TOKENS,
            temperature: 0.0,
            stop: [".", "\n"],
            stream: true,
            seed: 42,
            cache_prompt: false,
        }
    }
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

#[derive(Debug, Deserialize)]
struct TokenizeResponse {
    tokens: Vec<u32>,
}

#[derive(Debug, Deserialize)]
struct NativeStreamChunk {
    index: u32,
    #[serde(default)]
    content: String,
    stop: bool,
    #[serde(default)]
    truncated: Option<bool>,
    #[serde(default)]
    stop_type: Option<String>,
    #[serde(default)]
    stopped_limit: Option<bool>,
    #[serde(default)]
    stopped_word: Option<bool>,
    #[serde(default)]
    stopped_eos: Option<bool>,
    #[serde(default)]
    stopping_word: Option<String>,
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
    cancellation: CancellationToken,
) -> Result<ObservedCompletion, ClientError> {
    let mut pending = Vec::new();
    let mut response_body_bytes = 0_usize;
    let mut accumulator = StreamAccumulator::default();
    let mut saw_done = false;

    loop {
        let chunk = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            result = response.chunk() => result.map_err(transport_error)?,
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
    let elapsed = started.elapsed();
    let (disposition, output) = match accumulator.finish {
        _ if accumulator.invalid => (CompletionDisposition::InvalidOutput, None),
        Some(StreamFinish::Length) => (CompletionDisposition::Truncated, None),
        Some(StreamFinish::Stop) if accumulator.output.is_empty() => {
            (CompletionDisposition::ModelAbstained, None)
        }
        Some(StreamFinish::Stop) if valid_english_output(&accumulator.output) => {
            (CompletionDisposition::Suggested, Some(accumulator.output))
        }
        Some(StreamFinish::Stop) | None => (CompletionDisposition::InvalidOutput, None),
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
    Ok(ObservedCompletion {
        disposition,
        output,
        ttft: accumulator.ttft,
        elapsed,
        request_body_bytes,
        response_body_bytes,
    })
}

fn next_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, index + 2));
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, index + 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

fn event_data(event: &[u8]) -> Result<Option<String>, ClientError> {
    let event = std::str::from_utf8(event).map_err(|_| ClientError::MalformedStream)?;
    let mut lines = Vec::new();
    for line in event.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            lines.push(data.strip_prefix(' ').unwrap_or(data));
        }
    }
    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(lines.join("\n")))
    }
}

fn valid_english_output(value: &str) -> bool {
    if value.is_empty() || value.ends_with(char::is_whitespace) {
        return false;
    }
    let mut saw_latin = false;
    let mut mark_has_latin_base = false;
    for character in value.chars() {
        match character.script() {
            Script::Latin => {
                saw_latin = true;
                mark_has_latin_base = true;
            }
            Script::Inherited
                if mark_has_latin_base && ('\u{0300}'..='\u{036f}').contains(&character) => {}
            Script::Common if allowed_common_scalar(character) => {
                mark_has_latin_base = false;
            }
            _ => return false,
        }
    }
    saw_latin
}

const fn allowed_common_scalar(character: char) -> bool {
    matches!(
        character,
        ' ' | '0'
            ..='9'
                | '.'
                | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '\''
                | '\u{2019}'
                | '-'
                | '\u{2013}'
                | '\u{2014}'
                | '('
                | ')'
                | '['
                | ']'
                | '/'
                | '%'
                | '\u{2026}'
    )
}

fn ensure_content_type(headers: &HeaderMap, expected: &str) -> Result<(), ClientError> {
    let matches = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected));
    if matches {
        Ok(())
    } else {
        Err(ClientError::UnexpectedContentType)
    }
}

async fn read_bounded_body(mut response: Response, limit: usize) -> Result<Vec<u8>, ClientError> {
    let limit_u64 = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_u64)
    {
        return Err(ClientError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ClientError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn transport_error(error: reqwest::Error) -> ClientError {
    if error.is_timeout() {
        ClientError::Timeout
    } else {
        ClientError::Transport(error)
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
