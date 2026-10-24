//! Explicit experiment transport over the production client's authenticated
//! runtime endpoint. It never changes the production streaming deadline or
//! output contract.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;

use badi_broker::semantic::client::{ClientError, RequestObserver, SemanticClient};
use badi_broker::semantic::wire::{
    NativeStreamChunk, StatusCode, TokenizeResponse, ensure_content_type, event_data,
    next_event_boundary, read_bounded_body,
};

const MAX_LAB_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_LAB_RAW_BYTES: usize = 4096;

#[derive(Debug)]
pub(crate) struct TokenLogprob {
    pub bytes: Vec<u8>,
    pub logprob: f64,
}

#[derive(Debug, Default)]
pub(crate) struct LabStream {
    pub raw: String,
    pub ttft_ms: Option<f64>,
    pub first_word_ms: Option<f64>,
    pub first_four_words_ms: Option<f64>,
    pub latency_ms: f64,
    pub stopped: bool,
    pub terminal_received: bool,
    pub deadline: bool,
    pub tokens_predicted: Option<u64>,
    pub tokens_evaluated: Option<u64>,
    pub slot_tokens_cached: Option<u64>,
    pub reused_prompt_tokens: Option<u64>,
    pub newly_evaluated_prompt_tokens: Option<u64>,
    pub token_logprobs: Option<Vec<TokenLogprob>>,
}

impl LabStream {
    fn for_payload(payload: &Value) -> Self {
        let mut result = Self::default();
        if payload["n_probs"] == 1 {
            result.token_logprobs = Some(Vec::new());
        }
        result
    }

    fn capture_token_logprob(&mut self, data: &str, content: &str) {
        if self.token_logprobs.is_none() {
            return;
        }
        if let Some(token) = streamed_token_logprob(data) {
            if let Some(tokens) = self.token_logprobs.as_mut() {
                if tokens.len() < 64 {
                    tokens.push(token);
                    return;
                }
            }
        } else if content.is_empty() {
            return;
        }
        self.token_logprobs = None;
    }
}

fn streamed_token_logprob(data: &str) -> Option<TokenLogprob> {
    let value: Value = serde_json::from_str(data).ok()?;
    let tokens = value["completion_probabilities"].as_array()?;
    let [token] = tokens.as_slice() else {
        return None;
    };
    u32::try_from(token["id"].as_u64()?).ok()?;
    let bytes = token["bytes"].as_array()?;
    if bytes.is_empty() || bytes.len() > 128 {
        return None;
    }
    let bytes: Vec<u8> = bytes
        .iter()
        .map(|value| u8::try_from(value.as_u64()?).ok())
        .collect::<Option<_>>()?;
    let logprob = token["logprob"].as_f64()?;
    if !logprob.is_finite() || logprob > 0.0 {
        return None;
    }
    Some(TokenLogprob { bytes, logprob })
}

#[derive(Clone, Copy)]
pub(crate) struct LabObservation<'a> {
    pub before: &'a str,
    pub language: &'a str,
    pub echo: &'a str,
    /// Starts before tokenizer preflight, unlike the inference-only TTFT.
    pub started: Instant,
}

impl LabObservation<'_> {
    fn record(self, result: &mut LabStream, natural_stop: bool) {
        let Some(raw) = result.raw.strip_prefix(self.echo) else {
            return;
        };
        let eligible = |words| {
            badi_broker::writing::complete_word_prefix(raw, words, natural_stop).is_some_and(
                |candidate| {
                    candidate.unicode_words().count() >= words
                        && badi_broker::writing::WritingLanguage::from_tag(self.language)
                            .is_some_and(|language| language.accepts_output(&candidate))
                        && badi_broker::writing::validate_proposal(
                            self.before,
                            "",
                            &candidate,
                            Some(self.language),
                        )
                        .is_ok()
                },
            )
        };
        let elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        if result.first_word_ms.is_none() && eligible(1) {
            result.first_word_ms = Some(elapsed_ms);
        }
        if result.first_four_words_ms.is_none() && eligible(4) {
            result.first_four_words_ms = Some(elapsed_ms);
        }
    }
}

/// Records the body of each request the production client sends, so a
/// record can show exactly what production asked the runtime.
#[derive(Debug, Default)]
pub(crate) struct RequestTrace(Mutex<Vec<Option<Value>>>);

impl RequestObserver for RequestTrace {
    fn observe(&self, body: &[u8]) {
        if let Ok(mut requests) = self.0.lock() {
            requests.push(serde_json::from_slice(body).ok());
        }
    }
}

impl RequestTrace {
    /// The recorded JSON bodies, or `None` if any body was not JSON.
    pub(crate) fn requests(&self) -> Option<Vec<Value>> {
        self.0.lock().ok()?.iter().cloned().collect()
    }
}

/// Apply only the template embedded in the verified, owned model. This
/// endpoint formats text locally; it neither infers nor loads repository code.
pub(crate) async fn apply_template(
    client: &SemanticClient,
    messages: Value,
    budget_ms: u64,
    cancellation: CancellationToken,
) -> Result<String, ClientError> {
    let operation = async {
        let body = serde_json::to_vec(&json!({"messages":messages}))
            .map_err(|_| ClientError::InvalidRequest)?;
        let response = client
            .post_runtime(
                "/apply-template",
                body,
                Some(Duration::from_millis(budget_ms.min(2000))),
            )
            .await?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(&response, "application/json")?;
        let body = read_bounded_body(response, MAX_LAB_RESPONSE_BYTES).await?;
        let value: Value =
            serde_json::from_slice(&body).map_err(|_| ClientError::MalformedStream)?;
        let prompt = value["prompt"]
            .as_str()
            .filter(|text| !text.is_empty() && text.len() <= 48 * 1024)
            .ok_or(ClientError::MalformedStream)?;
        Ok(prompt.to_owned())
    };
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(ClientError::Cancelled),
        result = operation => result,
    }
}

pub(crate) async fn token_count(
    client: &SemanticClient,
    prompt: &str,
    budget_ms: u64,
    cancellation: CancellationToken,
) -> Result<usize, ClientError> {
    let operation = async {
        let body =
            serde_json::to_vec(&json!({"content":prompt,"add_special":true,"parse_special":true}))
                .map_err(|_| ClientError::InvalidRequest)?;
        let response = client
            .post_runtime(
                "/tokenize",
                body,
                Some(Duration::from_millis(budget_ms.min(2000))),
            )
            .await?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(&response, "application/json")?;
        let body = read_bounded_body(response, MAX_LAB_RESPONSE_BYTES).await?;
        let result: TokenizeResponse =
            serde_json::from_slice(&body).map_err(|_| ClientError::MalformedStream)?;
        if result.tokens.is_empty() {
            return Err(ClientError::MalformedStream);
        }
        Ok(result.tokens.len())
    };
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(ClientError::Cancelled),
        result = operation => result,
    }
}

pub(crate) async fn stream(
    client: &SemanticClient,
    payload: Value,
    budget_ms: u64,
    cancellation: CancellationToken,
    observation: Option<LabObservation<'_>>,
) -> Result<LabStream, ClientError> {
    let started = Instant::now();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(budget_ms);
    let mut result = LabStream::for_payload(&payload);
    let body = serde_json::to_vec(&payload).map_err(|_| ClientError::InvalidRequest)?;
    let send = client.post_runtime(
        "/completion",
        body,
        Some(Duration::from_millis(budget_ms + 100)),
    );
    let mut response = tokio::select! {
        biased;
        () = cancellation.cancelled() => return Err(ClientError::Cancelled),
        () = tokio::time::sleep_until(deadline) => {
            result.deadline = true;
            result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
            return Ok(result);
        },
        response = send => response?,
    };
    if response.status() != StatusCode::OK {
        return Err(ClientError::UnexpectedStatus(response.status()));
    }
    ensure_content_type(&response, "text/event-stream")?;
    let mut pending = Vec::new();
    let mut received = 0_usize;
    loop {
        let bytes = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            () = tokio::time::sleep_until(deadline) => {
                result.deadline = true;
                break;
            },
            bytes = response.chunk() => bytes?,
        };
        let Some(bytes) = bytes else {
            return Err(ClientError::MalformedStream);
        };
        received = received.saturating_add(bytes.len());
        if received > MAX_LAB_RESPONSE_BYTES {
            return Err(ClientError::ResponseTooLarge);
        }
        pending.extend_from_slice(&bytes);
        while let Some((end, consumed)) = next_event_boundary(&pending) {
            let data = event_data(&pending[..end])?;
            pending.drain(..consumed);
            let Some(data) = data else {
                continue;
            };
            // The native endpoint uses a terminal stop:true JSON event.
            let chunk: NativeStreamChunk =
                serde_json::from_str(&data).map_err(|_| ClientError::MalformedStream)?;
            if chunk.index != 0 || chunk.truncated == Some(true) {
                return Err(ClientError::MalformedStream);
            }
            if !chunk.content.is_empty() && result.ttft_ms.is_none() {
                result.ttft_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            if !chunk.stop {
                result.capture_token_logprob(&data, &chunk.content);
            }
            result.raw.push_str(&chunk.content);
            if result.raw.len() > MAX_LAB_RAW_BYTES {
                return Err(ClientError::ResponseTooLarge);
            }
            if let Some(observation) = observation {
                observation.record(&mut result, false);
            }
            if chunk.stop {
                let metadata: Value =
                    serde_json::from_str(&data).map_err(|_| ClientError::MalformedStream)?;
                let length_stop = chunk.stopped_limit == Some(true)
                    || chunk.stop_type.as_deref() == Some("limit");
                let natural_stop = chunk.stopped_eos == Some(true)
                    || chunk.stopped_word == Some(true)
                    || matches!(chunk.stop_type.as_deref(), Some("eos" | "word"));
                if !length_stop && !natural_stop {
                    return Err(ClientError::MalformedStream);
                }
                result.stopped = natural_stop && !length_stop;
                result.terminal_received = true;
                result.tokens_predicted = metadata["tokens_predicted"].as_u64();
                result.tokens_evaluated = metadata["tokens_evaluated"].as_u64();
                // llama.cpp reports the occupied final slot, including
                // generated tokens. It does not prove prompt-token reuse.
                result.slot_tokens_cached = metadata["tokens_cached"].as_u64();
                // Only final timings describe this completed request. Keep
                // absent or invalid optional counters unknown, including
                // when an older runtime omits them; never infer from slot
                // occupancy or the total prompt-token count.
                result.reused_prompt_tokens = metadata["timings"]["cache_n"].as_u64();
                result.newly_evaluated_prompt_tokens = metadata["timings"]["prompt_n"].as_u64();
                if let Some(observation) = observation {
                    let stopped = result.stopped;
                    observation.record(&mut result, stopped);
                }
                result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                return Ok(result);
            }
        }
    }
    result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok(result)
}

#[cfg(test)]
mod tests;
