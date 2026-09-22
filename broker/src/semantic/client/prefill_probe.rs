//! Fixed-input diagnostic transport. It discards response text and token IDs as
//! each bounded response is read; only counts and terminal metadata survive.

use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    AUTHORIZATION, ClientError, Duration, Instant, SemanticClient, StatusCode, ensure_content_type,
    event_data, next_event_boundary, read_bounded_body, transport_error,
};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Default, Serialize)]
pub struct PrefillMetrics {
    pub content_bytes: usize,
    pub returned_token_count: Option<usize>,
    pub tokens_predicted: Option<u64>,
    pub predicted_n: Option<u64>,
    pub cache_n: Option<u64>,
    pub prompt_n: Option<u64>,
    pub resolved_n_predict: Option<i64>,
    pub stop_type: Option<&'static str>,
    pub truncated: Option<bool>,
    pub latency_ms: f64,
}

impl PrefillMetrics {
    fn observe(&mut self, chunk: &Value) -> Result<bool, ClientError> {
        if !chunk.is_object()
            || chunk
                .get("index")
                .is_some_and(|index| index.as_u64() != Some(0))
        {
            return Err(ClientError::MalformedStream);
        }
        let content = chunk["content"]
            .as_str()
            .ok_or(ClientError::MalformedStream)?;
        self.content_bytes = self
            .content_bytes
            .checked_add(content.len())
            .ok_or(ClientError::ResponseTooLarge)?;
        if let Some(tokens) = chunk.get("tokens") {
            let tokens = tokens.as_array().ok_or(ClientError::MalformedStream)?;
            if tokens.iter().any(|token| token.as_u64().is_none()) {
                return Err(ClientError::MalformedStream);
            }
            self.returned_token_count = Some(
                self.returned_token_count
                    .unwrap_or(0)
                    .checked_add(tokens.len())
                    .ok_or(ClientError::ResponseTooLarge)?,
            );
        }
        let stop = chunk["stop"]
            .as_bool()
            .ok_or(ClientError::MalformedStream)?;
        if stop {
            self.tokens_predicted = chunk["tokens_predicted"].as_u64();
            self.predicted_n = chunk["timings"]["predicted_n"].as_u64();
            self.cache_n = chunk["timings"]["cache_n"].as_u64();
            self.prompt_n = chunk["timings"]["prompt_n"].as_u64();
            self.resolved_n_predict = chunk["generation_settings"]["n_predict"].as_i64();
            self.truncated = chunk["truncated"].as_bool();
            // Never retain arbitrary response strings in this diagnostic.
            self.stop_type = match chunk["stop_type"].as_str() {
                Some("eos") => Some("eos"),
                Some("limit") => Some("limit"),
                Some("word") => Some("word"),
                Some("none") => Some("none"),
                Some(_) => Some("unrecognized"),
                None => None,
            };
        }
        Ok(stop)
    }
}

impl SemanticClient {
    pub(crate) async fn prefill_probe(
        &self,
        prompt: &'static str,
        stream: bool,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> Result<PrefillMetrics, ClientError> {
        self.diagnostic_completion(prompt, 0, stream, budget, cancellation)
            .await
    }

    pub(crate) async fn prime_context(
        &self,
        prompt: &str,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> Result<PrefillMetrics, ClientError> {
        self.diagnostic_completion(prompt, 1, true, budget, cancellation)
            .await
    }

    async fn diagnostic_completion(
        &self,
        prompt: &str,
        n_predict: u8,
        stream: bool,
        budget: Duration,
        cancellation: CancellationToken,
    ) -> Result<PrefillMetrics, ClientError> {
        let started = Instant::now();
        let operation = async {
            let mut response = self
                .client
                .post(self.completion_url.clone())
                .header(AUTHORIZATION, self.config.authorization.clone())
                .timeout(budget)
                .json(&json!({"prompt":prompt,"stream":stream,"id_slot":0,
                    "n_predict":n_predict,"return_tokens":true,"cache_prompt":true,
                    "temperature":0,"seed":42}))
                .send()
                .await
                .map_err(transport_error)?;
            if response.status() != StatusCode::OK {
                return Err(ClientError::UnexpectedStatus(response.status()));
            }
            let mut metrics = PrefillMetrics::default();
            if !stream {
                ensure_content_type(response.headers(), "application/json")?;
                let bytes = read_bounded_body(response, MAX_RESPONSE_BYTES).await?;
                let result =
                    serde_json::from_slice(&bytes).map_err(|_| ClientError::MalformedStream)?;
                if !metrics.observe(&result)? {
                    return Err(ClientError::MalformedStream);
                }
                return Ok(metrics);
            }
            ensure_content_type(response.headers(), "text/event-stream")?;
            let mut pending = Vec::new();
            let mut received = 0_usize;
            while let Some(bytes) = response.chunk().await.map_err(transport_error)? {
                received = received.saturating_add(bytes.len());
                if received > MAX_RESPONSE_BYTES {
                    return Err(ClientError::ResponseTooLarge);
                }
                pending.extend_from_slice(&bytes);
                while let Some((end, consumed)) = next_event_boundary(&pending) {
                    let data = event_data(&pending[..end])?;
                    pending.drain(..consumed);
                    if let Some(data) = data {
                        let chunk = serde_json::from_str(&data)
                            .map_err(|_| ClientError::MalformedStream)?;
                        if metrics.observe(&chunk)? {
                            return Ok(metrics);
                        }
                    }
                }
            }
            Err(ClientError::MalformedStream)
        };
        let mut result = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            result = tokio::time::timeout(budget, operation) =>
                result.map_err(|_| ClientError::Timeout)??,
        };
        result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::semantic::client::SemanticClientConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const PROMPT: &str = "Disposable prefill fixture.";

    async fn fixture(
        body: String,
        stream: bool,
        header_delay: Duration,
        body_delay: Duration,
    ) -> (SemanticClient, tokio::task::JoinHandle<Value>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let client = SemanticClient::new(
            SemanticClientConfig::new(listener.local_addr().unwrap(), "fixture", "fixture-key")
                .unwrap(),
        )
        .unwrap();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let end = loop {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0 && bytes.len() < 8192);
                bytes.extend_from_slice(&buffer[..n]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            assert!(
                headers
                    .to_ascii_lowercase()
                    .contains("authorization: bearer fixture-key")
            );
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            while bytes.len() < end + length {
                let n = socket.read(&mut buffer).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
            }
            let request = serde_json::from_slice(&bytes[end..end + length]).unwrap();
            tokio::time::sleep(header_delay).await;
            let content_type = if stream {
                "text/event-stream"
            } else {
                "application/json"
            };
            let headers = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(headers.as_bytes()).await;
            tokio::time::sleep(body_delay).await;
            for bytes in body.as_bytes().chunks(7) {
                if socket.write_all(bytes).await.is_err() {
                    break;
                }
            }
            request
        });
        (client, task)
    }

    fn terminal(content: &str, tokens: &Value) -> Value {
        json!({"content":content,"tokens":tokens,"stop":true,"index":0,
            "tokens_predicted":1,"timings":{"predicted_n":1,"cache_n":12,"prompt_n":1},
            "generation_settings":{"n_predict":0},"stop_type":"limit","truncated":false})
    }

    #[tokio::test]
    async fn prefill_counts_nonstream_and_partial_stream_generation_without_retaining_text() {
        for stream in [false, true] {
            let final_chunk = terminal(
                if stream { "" } else { "é" },
                &if stream { json!([]) } else { json!([71]) },
            );
            let body = if stream {
                format!(
                    "data: {}\n\ndata: {final_chunk}\n\n",
                    json!({"content":"é","tokens":[71],"stop":false,"index":0,"timings":{"cache_n":999}})
                )
            } else {
                final_chunk.to_string()
            };
            let (client, task) = fixture(body, stream, Duration::ZERO, Duration::ZERO).await;
            let observed = client
                .prefill_probe(
                    PROMPT,
                    stream,
                    Duration::from_secs(1),
                    CancellationToken::new(),
                )
                .await
                .unwrap();
            assert_eq!(observed.content_bytes, 2);
            assert_eq!(observed.returned_token_count, Some(1));
            assert_eq!(observed.tokens_predicted, Some(1));
            assert_eq!(observed.predicted_n, Some(1));
            assert_eq!(observed.cache_n, Some(12));
            assert_eq!(observed.prompt_n, Some(1));
            assert_eq!(observed.resolved_n_predict, Some(0));
            assert_eq!(observed.stop_type, Some("limit"));
            let serialized = serde_json::to_string(&observed).unwrap();
            assert!(!serialized.contains('é') && !serialized.contains("fixture-key"));
            assert_eq!(
                task.await.unwrap(),
                json!({"prompt":PROMPT,"stream":stream,"id_slot":0,"n_predict":0,"return_tokens":true,"cache_prompt":true,"temperature":0,"seed":42})
            );
        }
    }

    #[test]
    fn prefill_empty_output_does_not_invent_zero_generation_or_cache_counts() {
        let mut result = PrefillMetrics::default();
        assert!(result.observe(&json!({"content":"","stop":true})).unwrap());
        assert_eq!(result.content_bytes, 0);
        assert_eq!(result.returned_token_count, None);
        assert_eq!(result.tokens_predicted, None);
        assert_eq!(result.predicted_n, None);
        assert_eq!(result.cache_n, None);
        assert_eq!(result.prompt_n, None);
        assert_eq!(result.resolved_n_predict, None);
        assert!(result.observe(&json!({"content":"","tokens":[],"stop":true,"tokens_predicted":0,"timings":{"predicted_n":0,"cache_n":0,"prompt_n":0}})).unwrap());
        assert_eq!(result.returned_token_count, Some(0));
        assert_eq!(result.tokens_predicted, Some(0));
        assert_eq!(result.predicted_n, Some(0));
        assert_eq!(result.cache_n, Some(0));
        assert_eq!(result.prompt_n, Some(0));
    }

    #[tokio::test]
    async fn paced_primer_explicitly_requests_one_token_and_discards_its_text() {
        let mut final_chunk = terminal("primer-output-canary", &json!([71]));
        final_chunk["generation_settings"]["n_predict"] = json!(1);
        let (client, task) = fixture(
            format!("data: {final_chunk}\n\n"),
            true,
            Duration::ZERO,
            Duration::ZERO,
        )
        .await;
        let metrics = client
            .prime_context(
                "Stable context.\n\nStyle.\n\n",
                Duration::from_secs(1),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(
            task.await.unwrap(),
            json!({"prompt":"Stable context.\n\nStyle.\n\n",
            "stream":true,"id_slot":0,"n_predict":1,"return_tokens":true,
            "cache_prompt":true,"temperature":0,"seed":42})
        );
        assert_eq!(metrics.resolved_n_predict, Some(1));
        assert_eq!(metrics.returned_token_count, Some(1));
        assert!(
            !serde_json::to_string(&metrics)
                .unwrap()
                .contains("primer-output-canary")
        );
    }

    #[tokio::test]
    async fn prefill_deadline_includes_headers_and_body_and_cancellation_wins() {
        for stream in [false, true] {
            for headers in [false, true] {
                let delay = Duration::from_millis(100);
                let final_chunk = terminal("", &json!([]));
                let body = if stream {
                    format!("data: {final_chunk}\n\n")
                } else {
                    final_chunk.to_string()
                };
                let (client, task) = fixture(
                    body,
                    stream,
                    if headers { delay } else { Duration::ZERO },
                    if headers { Duration::ZERO } else { delay },
                )
                .await;
                let result = client
                    .prefill_probe(
                        PROMPT,
                        stream,
                        Duration::from_millis(20),
                        CancellationToken::new(),
                    )
                    .await;
                assert!(matches!(result, Err(ClientError::Timeout)));
                task.abort();
                let _ = task.await;
            }
            let (client, task) = fixture(
                String::new(),
                stream,
                Duration::from_secs(1),
                Duration::ZERO,
            )
            .await;
            let cancellation = CancellationToken::new();
            let cancel = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                cancel.cancel();
            });
            assert!(matches!(
                client
                    .prefill_probe(PROMPT, stream, Duration::from_secs(1), cancellation)
                    .await,
                Err(ClientError::Cancelled)
            ));
            task.abort();
            let _ = task.await;
        }
    }

    #[tokio::test]
    async fn prefill_rejects_truncated_streams_and_bounded_body_overflow() {
        for (stream, body, oversized) in [
            (
                true,
                "data: {\"content\":\"x\",\"tokens\":[1],\"stop\":false}\n\n".to_owned(),
                false,
            ),
            (true, "x".repeat(MAX_RESPONSE_BYTES + 1), true),
            (false, "x".repeat(MAX_RESPONSE_BYTES + 1), true),
        ] {
            let (client, task) = fixture(body, stream, Duration::ZERO, Duration::ZERO).await;
            let result = client
                .prefill_probe(
                    PROMPT,
                    stream,
                    Duration::from_secs(2),
                    CancellationToken::new(),
                )
                .await;
            assert!(
                matches!(result, Err(ClientError::ResponseTooLarge)) && oversized
                    || matches!(result, Err(ClientError::MalformedStream)) && !oversized
            );
            task.abort();
            let _ = task.await;
        }
    }
}
