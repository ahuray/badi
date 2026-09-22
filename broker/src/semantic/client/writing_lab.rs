//! Explicit experiment transport. It shares authentication and endpoint ownership,
//! but never changes the production streaming deadline or output contract.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};

use super::{
    AUTHORIZATION, CONTENT_TYPE, ClientError, Duration, Instant, NativeStreamChunk, SemanticClient,
    StatusCode, TokenizeResponse, ensure_content_type, event_data, next_event_boundary,
    read_bounded_body, transport_error,
};
use tokio_util::sync::CancellationToken;
use unicode_segmentation::UnicodeSegmentation;

const MAX_LAB_RESPONSE_BYTES: usize = 128 * 1024;
const MAX_LAB_RAW_BYTES: usize = 4096;

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
            crate::writing::complete_word_prefix(raw, words, natural_stop).is_some_and(
                |candidate| {
                    candidate.unicode_words().count() >= words
                        && crate::writing::WritingLanguage::from_tag(self.language)
                            .is_some_and(|language| language.accepts_output(&candidate))
                        && crate::writing::validate_proposal(
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

impl SemanticClient {
    /// Apply only the template embedded in the verified, owned model. This
    /// endpoint formats text locally; it neither infers nor loads repository code.
    pub(crate) async fn lab_apply_template(
        &self,
        messages: Value,
        budget_ms: u64,
        cancellation: CancellationToken,
    ) -> Result<String, ClientError> {
        let mut url = self.completion_url.clone();
        url.set_path("/apply-template");
        let operation = async {
            let response = self
                .client
                .post(url)
                .header(AUTHORIZATION, self.config.authorization.clone())
                .timeout(Duration::from_millis(budget_ms.min(2000)))
                .json(&json!({"messages":messages}))
                .send()
                .await
                .map_err(transport_error)?;
            if response.status() != StatusCode::OK {
                return Err(ClientError::UnexpectedStatus(response.status()));
            }
            ensure_content_type(response.headers(), "application/json")?;
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

    pub(crate) fn with_lab_production_boundary(mut self) -> Self {
        self.lab_boundary_healing = true;
        self
    }

    pub(crate) fn with_lab_trace(&self) -> (Self, Arc<Mutex<Vec<Value>>>) {
        let trace = Arc::new(Mutex::new(Vec::new()));
        let mut client = self.clone();
        client.lab_trace = Some(Arc::clone(&trace));
        (client, trace)
    }

    pub(crate) async fn lab_token_count(
        &self,
        prompt: &str,
        budget_ms: u64,
        cancellation: CancellationToken,
    ) -> Result<usize, ClientError> {
        let operation = async {
            let response = self
                .client
                .post(self.challenge_url.clone())
                .header(AUTHORIZATION, self.config.authorization.clone())
                .timeout(Duration::from_millis(budget_ms.min(2000)))
                .json(&json!({"content":prompt,"add_special":true,"parse_special":true}))
                .send()
                .await
                .map_err(transport_error)?;
            if response.status() != StatusCode::OK {
                return Err(ClientError::UnexpectedStatus(response.status()));
            }
            ensure_content_type(response.headers(), "application/json")?;
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

    pub(crate) async fn lab_stream(
        &self,
        payload: Value,
        budget_ms: u64,
        cancellation: CancellationToken,
        observation: Option<LabObservation<'_>>,
    ) -> Result<LabStream, ClientError> {
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + Duration::from_millis(budget_ms);
        let mut result = LabStream::default();
        let send = self
            .client
            .post(self.completion_url.clone())
            .header(AUTHORIZATION, self.config.authorization.clone())
            .header(CONTENT_TYPE, "application/json")
            .timeout(Duration::from_millis(budget_ms + 100))
            .json(&payload)
            .send();
        let mut response = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(ClientError::Cancelled),
            () = tokio::time::sleep_until(deadline) => {
                result.deadline = true;
                result.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
                return Ok(result);
            },
            response = send => response.map_err(transport_error)?,
        };
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(response.headers(), "text/event-stream")?;
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
                bytes = response.chunk() => bytes.map_err(transport_error)?,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{CompletionProvider, ProviderError, ProviderRequest};
    use crate::semantic::client::SemanticClientConfig;
    use std::fmt::Write as _;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn fixture(
        body: String,
        delay: Duration,
        content_type: &'static str,
    ) -> (SemanticClient, tokio::task::JoinHandle<Value>) {
        fixture_with_stall(body, delay, content_type, None).await
    }

    async fn fixture_with_stall(
        body: String,
        delay: Duration,
        content_type: &'static str,
        stall: Option<Duration>,
    ) -> (SemanticClient, tokio::task::JoinHandle<Value>) {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind");
        let endpoint = listener.local_addr().expect("endpoint");
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            let end = loop {
                let count = socket.read(&mut buffer).await.expect("request");
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
                if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let length: usize = std::str::from_utf8(&bytes[..end])
                .expect("header")
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("length"))
                })
                .expect("body length");
            while bytes.len() < end + length {
                let count = socket.read(&mut buffer).await.expect("body");
                assert_ne!(count, 0);
                bytes.extend_from_slice(&buffer[..count]);
            }
            let payload: Value =
                serde_json::from_slice(&bytes[end..end + length]).expect("payload");
            tokio::time::sleep(delay).await;
            let length = if stall.is_some() {
                String::new()
            } else {
                format!("Content-Length: {}\r\n", body.len())
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{length}Connection: close\r\n\r\n{body}"
            );
            let _ = socket.write_all(response.as_bytes()).await;
            if let Some(stall) = stall {
                tokio::time::sleep(stall).await;
            }
            payload
        });
        let client = SemanticClient::new(
            SemanticClientConfig::new(endpoint, "fixture", "local-test-secret")
                .expect("config")
                .for_writing(),
        )
        .expect("client");
        (client, task)
    }

    fn production_request(before: &str, language: &str) -> ProviderRequest {
        ProviderRequest {
            before: before.to_owned(),
            after: String::new(),
            language: Some(language.to_owned()),
        }
    }

    fn events(pieces: &[&str], terminal: Option<&str>) -> String {
        let mut body = String::new();
        for piece in pieces {
            write!(
                body,
                "data: {}\n\n",
                json!({"index":0,"content":piece,"stop":false})
            )
            .expect("fixture event");
        }
        if let Some(terminal) = terminal {
            write!(
                body,
                "data: {}\n\n",
                json!({"index":0,"content":"","stop":true,"stop_type":terminal})
            )
            .expect("fixture terminal event");
        }
        body
    }

    #[tokio::test]
    async fn terminal_receipt_is_distinct_from_natural_word_completion_and_deadline() {
        for (terminal, received, natural) in [
            (Some("limit"), true, false),
            (Some("eos"), true, true),
            (None, false, false),
        ] {
            let (client, server) = fixture_with_stall(
                events(&["complete words "], terminal),
                Duration::ZERO,
                "text/event-stream",
                terminal.is_none().then_some(Duration::from_millis(100)),
            )
            .await;
            let observed = client
                .lab_stream(
                    json!({"prompt":"fixture"}),
                    30,
                    CancellationToken::new(),
                    None,
                )
                .await
                .unwrap();
            assert_eq!(observed.terminal_received, received);
            assert_eq!(observed.stopped, natural);
            assert_eq!(observed.deadline, !received);
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn production_boundary_handles_chunked_exact_echo_for_all_writing_languages() {
        for (before, language, pieces, expected) in [
            (
                "Please review the ",
                "en",
                vec![" ", "report and make changes "],
                "report and make changes",
            ),
            (
                "Please review the  ",
                "en-US",
                vec![" ", " ", "report"],
                "report",
            ),
            (
                "Bitte lies das ",
                "de-DE",
                vec![" ", "Dokument sorgfältig"],
                "Dokument sorgfältig",
            ),
            ("لطفا این گزارش را ", "fa", vec![" ", "بخوانید"], "بخوانید"),
        ] {
            let (client, server) = fixture(
                events(&pieces, Some("eos")),
                Duration::ZERO,
                "text/event-stream",
            )
            .await;
            let result = client
                .with_lab_production_boundary()
                .propose(
                    production_request(before, language),
                    CancellationToken::new(),
                    true,
                )
                .await
                .expect("proposal")
                .expect("suggestion");
            assert_eq!(result.text, expected);
            assert_eq!(result.replace_before, None);
            let payload = server.await.expect("server");
            assert_eq!(payload["prompt"], before.trim_end_matches(' '));
            assert_eq!(payload["n_predict"], 8);
            let echo = &before[before.trim_end_matches(' ').len()..];
            assert_eq!(
                payload["grammar"],
                format!(
                    "root ::= {} [^<>\\n\\r`]*",
                    serde_json::to_string(echo).expect("literal")
                )
            );
        }
    }

    #[tokio::test]
    async fn production_boundary_never_exposes_missing_mismatched_or_incomplete_echoes() {
        for (pieces, terminal, expected) in [
            (vec!["report and make changes "], "eos", None),
            (vec!["  report and make changes "], "eos", None),
            (vec![" "], "eos", None),
            (vec![" ", "docu"], "limit", None),
            (vec![" ", "report and par"], "limit", Some("report and")),
        ] {
            let (client, server) = fixture(
                events(&pieces, Some(terminal)),
                Duration::ZERO,
                "text/event-stream",
            )
            .await;
            let result = client
                .with_lab_production_boundary()
                .propose(
                    production_request("Please review the ", "en"),
                    CancellationToken::new(),
                    true,
                )
                .await
                .expect("single attempt");
            assert_eq!(
                result.as_ref().map(|proposal| proposal.text.as_str()),
                expected
            );
            server.await.expect("only one request");
        }
    }

    #[tokio::test]
    async fn production_boundary_deadline_salvages_complete_words_and_cancellation_stays_terminal()
    {
        let (client, server) = fixture_with_stall(
            events(&[" ", "report and par"], None),
            Duration::ZERO,
            "text/event-stream",
            Some(Duration::from_millis(700)),
        )
        .await;
        let result = client
            .with_lab_production_boundary()
            .propose(
                production_request("Please review the ", "en"),
                CancellationToken::new(),
                true,
            )
            .await
            .expect("deadline")
            .expect("complete prefix");
        assert_eq!(result.text, "report and");
        server.await.expect("single stalled request");
        for header_delay in [Duration::ZERO, Duration::from_millis(150)] {
            let (client, server) = fixture_with_stall(
                events(&[" "], None),
                header_delay,
                "text/event-stream",
                Some(Duration::from_millis(300)),
            )
            .await;
            let cancellation = CancellationToken::new();
            let cancel = cancellation.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                cancel.cancel();
            });
            assert!(matches!(
                client
                    .with_lab_production_boundary()
                    .propose(
                        production_request("Please review the ", "en"),
                        cancellation,
                        true
                    )
                    .await,
                Err(ProviderError::Cancelled)
            ));
            server.await.expect("cancelled request");
        }
    }

    #[tokio::test]
    async fn production_boundary_does_not_replace_the_dictionary_correction_path() {
        let (client, server) = fixture(String::new(), Duration::ZERO, "text/event-stream").await;
        let (client, trace) = client.with_lab_production_boundary().with_lab_trace();
        let result = client
            .propose(
                production_request("This is my adress ", "en"),
                CancellationToken::new(),
                true,
            )
            .await
            .expect("dictionary")
            .expect("correction");
        assert_eq!(result.text, "address ");
        assert_eq!(result.replace_before.as_deref(), Some("adress "));
        assert!(trace.lock().expect("trace").is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn production_boundary_cannot_submit_after_spelling_spends_the_shared_budget() {
        let (client, server) = fixture(
            String::new(),
            Duration::from_millis(700),
            "text/event-stream",
        )
        .await;
        let (client, trace) = client.with_lab_production_boundary().with_lab_trace();
        let result = client
            .propose(
                production_request("This is teh ", "en"),
                CancellationToken::new(),
                true,
            )
            .await
            .expect("spent budget");
        assert!(result.is_none());
        assert_eq!(
            trace.lock().expect("trace").len(),
            1,
            "no continuation request after the spelling deadline"
        );
        let correction_payload = server.await.expect("spelling request");
        assert_ne!(
            correction_payload["grammar"],
            "root ::= \" \" [^<>\\n\\r`]*"
        );
    }

    #[tokio::test]
    async fn native_template_uses_local_formatter_and_rejects_missing_prompt() {
        let messages = json!([{"role":"user","content":"Continue this disposable draft."}]);
        let (client, task) = fixture(
            json!({"prompt":"<native-assistant>"}).to_string(),
            Duration::ZERO,
            "application/json",
        )
        .await;
        assert_eq!(
            client
                .lab_apply_template(messages.clone(), 550, CancellationToken::new())
                .await
                .unwrap(),
            "<native-assistant>"
        );
        assert_eq!(task.await.unwrap(), json!({"messages":messages}));
        for body in [json!({}), json!({"prompt":""}), json!({"prompt":false})] {
            let (client, task) =
                fixture(body.to_string(), Duration::ZERO, "application/json").await;
            assert!(matches!(
                client
                    .lab_apply_template(messages.clone(), 550, CancellationToken::new())
                    .await,
                Err(ClientError::MalformedStream)
            ));
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn native_template_cancellation_does_not_dispatch() {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let client = SemanticClient::new(
            SemanticClientConfig::new(
                listener.local_addr().unwrap(),
                "template-fixture",
                "public-fixture-key",
            )
            .unwrap(),
        )
        .unwrap();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            client.lab_apply_template(json!([]), 550, cancelled).await,
            Err(ClientError::Cancelled)
        ));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn lab_tokenizer_counts_exact_special_token_aware_prompt() {
        let (client, task) = fixture(
            "{\"tokens\":[1,2,3]}".to_owned(),
            Duration::ZERO,
            "application/json",
        )
        .await;
        assert_eq!(
            client
                .lab_token_count(
                    "<|im_start|>assistant\nhello",
                    550,
                    CancellationToken::new()
                )
                .await
                .expect("count"),
            3
        );
        let payload = task.await.expect("server");
        assert_eq!(
            payload,
            json!({"content":"<|im_start|>assistant\nhello","add_special":true,"parse_special":true})
        );
    }

    #[tokio::test]
    async fn lab_stream_retains_raw_and_distinguishes_a_token_limit() {
        let body = "data: {\"index\":0,\"content\":\" next wor\",\"stop\":false}\n\ndata: {\"index\":0,\"content\":\"\",\"stop\":true,\"stop_type\":\"limit\",\"tokens_predicted\":3,\"tokens_evaluated\":12,\"tokens_cached\":8}\n\n";
        let (client, task) = fixture(body.to_owned(), Duration::ZERO, "text/event-stream").await;
        let observed = client
            .lab_stream(
                json!({"prompt":"The"}),
                550,
                CancellationToken::new(),
                Some(LabObservation {
                    before: "The",
                    language: "en",
                    echo: "",
                    started: Instant::now()
                        .checked_sub(Duration::from_millis(50))
                        .expect("preflight clock"),
                }),
            )
            .await
            .expect("stream");
        assert_eq!(observed.raw, " next wor");
        assert!(!observed.stopped && !observed.deadline);
        assert_eq!(observed.tokens_predicted, Some(3));
        assert_eq!(observed.slot_tokens_cached, Some(8));
        assert_eq!(observed.reused_prompt_tokens, None);
        assert_eq!(observed.newly_evaluated_prompt_tokens, None);
        assert!(observed.ttft_ms.is_some());
        assert!(observed.first_word_ms.expect("complete first word") >= 50.0);
        assert!(observed.first_four_words_ms.is_none());
        task.await.expect("server");
    }

    #[tokio::test]
    async fn lab_terminal_timings_keep_reuse_processing_and_occupancy_distinct() {
        let partial = json!({"index":0,"content":" next ","stop":false,
            "timings":{"cache_n":999,"prompt_n":888}});
        let terminal = json!({"index":0,"content":"","stop":true,"stop_type":"eos",
            "tokens_predicted":6,"tokens_evaluated":121,"tokens_cached":127,
            "timings":{"cache_n":93,"prompt_n":28}});
        let (client, task) = fixture(
            format!("data: {partial}\n\ndata: {terminal}\n\n"),
            Duration::ZERO,
            "text/event-stream",
        )
        .await;
        let payload = json!({"prompt":"The","n_predict":8,"cache_prompt":true,"stream":true});
        let observed = client
            .lab_stream(payload.clone(), 550, CancellationToken::new(), None)
            .await
            .expect("stream");
        assert_eq!(observed.raw, " next ");
        assert!(observed.stopped && !observed.deadline);
        assert_eq!(observed.tokens_predicted, Some(6));
        assert_eq!(observed.tokens_evaluated, Some(121));
        assert_eq!(observed.slot_tokens_cached, Some(127));
        assert_eq!(observed.reused_prompt_tokens, Some(93));
        assert_eq!(observed.newly_evaluated_prompt_tokens, Some(28));
        assert_eq!(task.await.expect("unchanged request"), payload);
    }

    #[tokio::test]
    async fn lab_optional_terminal_counters_preserve_zero_and_unknown_independently() {
        for (timings, reused, evaluated) in [
            (json!(null), None, None),
            (json!({}), None, None),
            (json!([]), None, None),
            (json!("unknown"), None, None),
            (json!({"cache_n":null,"prompt_n":null}), None, None),
            (json!({"cache_n":-1,"prompt_n":1.5}), None, None),
            (json!({"cache_n":"93","prompt_n":true}), None, None),
            (json!({"cache_n":1e100,"prompt_n":{}}), None, None),
            (json!({"cache_n":0,"prompt_n":0}), Some(0), Some(0)),
            (json!({"cache_n":93}), Some(93), None),
            (json!({"prompt_n":28}), None, Some(28)),
        ] {
            let terminal = json!({"index":0,"content":" next","stop":true,"stop_type":"eos",
                "tokens_evaluated":121,"tokens_cached":127,"timings":timings});
            let (client, task) = fixture(
                format!("data: {terminal}\n\n"),
                Duration::ZERO,
                "text/event-stream",
            )
            .await;
            let observed = client
                .lab_stream(json!({"prompt":"The"}), 550, CancellationToken::new(), None)
                .await
                .expect("optional statistics do not invalidate completion");
            assert_eq!(observed.raw, " next");
            assert!(observed.stopped && !observed.deadline);
            assert_eq!(observed.reused_prompt_tokens, reused);
            assert_eq!(observed.newly_evaluated_prompt_tokens, evaluated);
            task.await.expect("server");
        }
    }

    #[tokio::test]
    async fn lab_deadline_does_not_promote_partial_timings_to_completed_measurements() {
        let partial = json!({"index":0,"content":" next ","stop":false,
            "timings":{"cache_n":93,"prompt_n":28}});
        let (client, task) = fixture_with_stall(
            format!("data: {partial}\n\n"),
            Duration::ZERO,
            "text/event-stream",
            Some(Duration::from_millis(150)),
        )
        .await;
        let observed = client
            .lab_stream(json!({"prompt":"The"}), 30, CancellationToken::new(), None)
            .await
            .expect("deadline");
        assert_eq!(observed.raw, " next ");
        assert!(observed.deadline && !observed.stopped);
        assert_eq!(observed.reused_prompt_tokens, None);
        assert_eq!(observed.newly_evaluated_prompt_tokens, None);
        task.await.expect("server");
    }

    #[tokio::test]
    async fn lab_budget_starts_before_headers_and_cancellation_wins() {
        let (client, task) = fixture(
            String::new(),
            Duration::from_millis(80),
            "text/event-stream",
        )
        .await;
        let observed = client
            .lab_stream(json!({"prompt":"The"}), 20, CancellationToken::new(), None)
            .await
            .expect("deadline");
        assert!(observed.deadline && observed.raw.is_empty());
        assert!(observed.latency_ms < 75.0);
        task.await.expect("server");
        let (client, task) = fixture(
            String::new(),
            Duration::from_millis(80),
            "text/event-stream",
        )
        .await;
        let cancellation = CancellationToken::new();
        cancellation.cancel();
        assert!(matches!(
            client
                .lab_stream(json!({"prompt":"The"}), 0, cancellation, None)
                .await,
            Err(ClientError::Cancelled)
        ));
        task.abort();
    }

    #[tokio::test]
    async fn lab_rejects_a_truncated_context_or_missing_terminal_event() {
        for body in [
            "data: {\"index\":0,\"content\":\" next\",\"stop\":true,\"truncated\":true,\"stop_type\":\"eos\"}\n\n",
            "data: {\"index\":0,\"content\":\" next\",\"stop\":false}\n\n",
        ] {
            let (client, task) =
                fixture(body.to_owned(), Duration::ZERO, "text/event-stream").await;
            assert!(matches!(
                client
                    .lab_stream(json!({"prompt":"The"}), 550, CancellationToken::new(), None)
                    .await,
                Err(ClientError::MalformedStream)
            ));
            task.await.expect("server");
        }
    }

    fn observation_result(raw: &str) -> LabStream {
        LabStream {
            raw: raw.to_owned(),
            ttft_ms: None,
            first_word_ms: None,
            first_four_words_ms: None,
            latency_ms: 0.0,
            stopped: false,
            terminal_received: false,
            deadline: false,
            tokens_predicted: None,
            tokens_evaluated: None,
            slot_tokens_cached: None,
            reused_prompt_tokens: None,
            newly_evaluated_prompt_tokens: None,
        }
    }

    #[test]
    fn candidate_readiness_requires_complete_words_and_preserves_the_first_timestamp() {
        let observation = LabObservation {
            before: "Please review the docum",
            language: "en",
            echo: "docum",
            started: Instant::now()
                .checked_sub(Duration::from_millis(80))
                .expect("preflight clock"),
        };
        let mut result = observation_result("docum");
        observation.record(&mut result, false);
        assert!(result.first_word_ms.is_none());
        result.raw.push_str("ent ");
        observation.record(&mut result, false);
        let first = result.first_word_ms.expect("healed complete word");
        assert!(first >= 80.0);
        assert!(result.first_four_words_ms.is_none());
        result.raw.push_str("and provide a ");
        observation.record(&mut result, false);
        assert_eq!(result.first_word_ms, Some(first));
        assert!(result.first_four_words_ms.expect("four complete words") >= first);
    }

    #[test]
    fn candidate_readiness_checks_exact_echo_language_and_stem() {
        for (before, language, echo, raw) in [
            ("Please review the docum", "en", "docum", "report "),
            ("Please review the docum", "en", "docum", "docu "),
            ("I use Badi", "en", "", "ology "),
            ("لطفا", "fa", "", " report "),
        ] {
            let observation = LabObservation {
                before,
                language,
                echo,
                started: Instant::now(),
            };
            let mut result = observation_result(raw);
            observation.record(&mut result, true);
            assert!(
                result.first_word_ms.is_none(),
                "unexpected readiness for {before}"
            );
            assert!(result.first_four_words_ms.is_none());
        }
    }

    #[test]
    fn only_a_natural_stop_can_certify_an_unterminated_final_word() {
        let observation = LabObservation {
            before: "The",
            language: "en",
            echo: "",
            started: Instant::now(),
        };
        let mut result = observation_result(" next");
        observation.record(&mut result, false);
        assert!(result.first_word_ms.is_none());
        observation.record(&mut result, true);
        assert!(result.first_word_ms.is_some());
        assert!(
            result.first_four_words_ms.is_none(),
            "one final word is not four words"
        );
    }
}
