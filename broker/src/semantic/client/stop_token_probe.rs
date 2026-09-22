//! Fixed synthetic stop-token diagnostic. The authenticated client stays
//! private; callers select only a compiled word/arm, never an HTTP payload.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{
    AUTHORIZATION, ClientError, Duration, Instant, NativeStreamChunk, SemanticClient, StatusCode,
    ensure_content_type, event_data, next_event_boundary, read_bounded_body, transport_error,
};

const MAX_BYTES: usize = 64 * 1024;
pub(crate) const REQUEST_MS: u64 = 5000;
pub(crate) const TOKEN_ID: u32 = 715;
const PROMPT: &str = "A disposable timing probe follows. Complete its fixed word: ";

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Arm {
    SpaceEos,
    StopToken,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Word {
    Copper,
    Garten,
    Window,
}

impl Word {
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Copper => "copper",
            Self::Garten => "Garten",
            Self::Window => "پنجره",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Step {
    Primer,
    Target(Word, Arm),
}

impl Step {
    pub(crate) fn payload(self) -> Value {
        let grammar = match self {
            Self::Primer => "root ::= \".\"".to_owned(),
            Self::Target(word, Arm::SpaceEos) => {
                format!("root ::= {}", json!(format!("{} ", word.text())))
            }
            Self::Target(word, Arm::StopToken) => {
                format!("root ::= {} <[715]>", json!(word.text()))
            }
        };
        json!({"prompt":PROMPT,"grammar":grammar,"n_predict":16,"temperature":0,"seed":42,
            "cache_prompt":true,"stream":true,"return_tokens":true,
            "stop":["\n","<|im_end|>","<|endoftext|>"]})
    }

    pub(crate) fn validate(self, result: &Observation) -> Result<(), &'static str> {
        let expected = match self {
            Self::Primer => ".".to_owned(),
            Self::Target(word, _) => format!("{} ", word.text()),
        };
        if result.raw != expected {
            return Err("literal_output_mismatch");
        }
        let terminal = result.terminal.as_ref().ok_or("terminal_missing")?;
        let word_stop = matches!(self, Self::Target(_, Arm::StopToken));
        if !terminal.stop
            || terminal.truncated == Some(true)
            || terminal.stopped_limit == Some(true)
            || terminal.stop_type.as_deref() != Some(if word_stop { "word" } else { "eos" })
            || terminal.stopping_word.as_deref() != Some(if word_stop { "\n" } else { "" })
            || terminal
                .stopped_word
                .is_some_and(|value| value != word_stop)
            || terminal.stopped_eos.is_some_and(|value| value == word_stop)
        {
            return Err("native_stop_mismatch");
        }
        if ![terminal.tokens_predicted, terminal.predicted_n]
            .into_iter()
            .all(|value| value.is_some_and(|count| (1..=16).contains(&count)))
        {
            return Err("native_decode_counts_unavailable");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct TokenCheck {
    pub decoded_bytes: Vec<u8>,
    pub reencoded_tokens: Vec<u32>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct Terminal {
    pub stop: bool,
    pub stop_type: Option<String>,
    pub stopping_word: Option<String>,
    pub stopped_eos: Option<bool>,
    pub stopped_word: Option<bool>,
    pub stopped_limit: Option<bool>,
    pub truncated: Option<bool>,
    pub tokens_predicted: Option<u64>,
    pub predicted_n: Option<u64>,
    pub prompt_n: Option<u64>,
    pub cache_n: Option<u64>,
}

#[derive(Debug, Serialize)]
pub(crate) struct Chunk {
    arrival_ms: f64,
    content: String,
    stop: bool,
    returned_token_ids: Option<Vec<u32>>,
}

#[derive(Debug, Default, Serialize)]
pub(crate) struct Observation {
    pub raw: String,
    pub latency_ms: f64,
    pub ttft_ms: Option<f64>,
    pub chunks: Vec<Chunk>,
    pub terminal: Option<Terminal>,
}

impl Observation {
    fn observe(&mut self, value: &Value, elapsed_ms: f64) -> Result<bool, ClientError> {
        let chunk: NativeStreamChunk =
            serde_json::from_value(value.clone()).map_err(|_| ClientError::MalformedStream)?;
        if chunk.index != 0 || self.terminal.is_some() || self.chunks.len() >= 64 {
            return Err(ClientError::MalformedStream);
        }
        let returned_token_ids = value
            .get("tokens")
            .map(|tokens| {
                let tokens = tokens.as_array().ok_or(ClientError::MalformedStream)?;
                if tokens.len() > 256 {
                    return Err(ClientError::ResponseTooLarge);
                }
                tokens
                    .iter()
                    .map(|token| {
                        token
                            .as_u64()
                            .and_then(|n| u32::try_from(n).ok())
                            .ok_or(ClientError::MalformedStream)
                    })
                    .collect::<Result<Vec<_>, _>>()
            })
            .transpose()?;
        self.raw.push_str(&chunk.content);
        if self.raw.len() > 512 {
            return Err(ClientError::ResponseTooLarge);
        }
        if !chunk.content.is_empty() && self.ttft_ms.is_none() {
            self.ttft_ms = Some(elapsed_ms);
        }
        if chunk.stop {
            let count = |key| value["timings"][key].as_u64();
            self.terminal = Some(Terminal {
                stop: true,
                stop_type: chunk.stop_type,
                stopping_word: chunk.stopping_word,
                stopped_eos: chunk.stopped_eos,
                stopped_word: chunk.stopped_word,
                stopped_limit: chunk.stopped_limit,
                truncated: chunk.truncated,
                tokens_predicted: value["tokens_predicted"].as_u64(),
                predicted_n: count("predicted_n"),
                prompt_n: count("prompt_n"),
                cache_n: count("cache_n"),
            });
        } else if chunk.stop_type.is_some()
            || chunk.stopping_word.is_some()
            || chunk.stopped_eos.is_some()
            || chunk.stopped_word.is_some()
            || chunk.stopped_limit.is_some()
            || chunk.truncated.is_some()
        {
            return Err(ClientError::MalformedStream);
        }
        self.chunks.push(Chunk {
            arrival_ms: elapsed_ms,
            content: chunk.content,
            stop: chunk.stop,
            returned_token_ids,
        });
        self.latency_ms = elapsed_ms;
        Ok(chunk.stop)
    }
}

fn validate_round_trip(decoded: &Value, encoded: Value) -> Result<TokenCheck, ClientError> {
    let decoded = decoded["content"]
        .as_str()
        .ok_or(ClientError::MalformedStream)?;
    let encoded: super::TokenizeResponse =
        serde_json::from_value(encoded).map_err(|_| ClientError::MalformedStream)?;
    if decoded.as_bytes() != b" \n" || encoded.tokens != [TOKEN_ID] {
        return Err(ClientError::MalformedStream);
    }
    Ok(TokenCheck {
        decoded_bytes: decoded.as_bytes().to_vec(),
        reencoded_tokens: encoded.tokens,
    })
}

impl SemanticClient {
    async fn stop_probe_json(&self, route: &str, payload: Value) -> Result<Value, ClientError> {
        let url = self
            .completion_url
            .join(route)
            .map_err(|_| ClientError::InvalidEndpoint)?;
        let response = self
            .client
            .post(url)
            .header(AUTHORIZATION, self.config.authorization.clone())
            .timeout(Duration::from_millis(REQUEST_MS))
            .json(&payload)
            .send()
            .await
            .map_err(transport_error)?;
        if response.status() != StatusCode::OK {
            return Err(ClientError::UnexpectedStatus(response.status()));
        }
        ensure_content_type(response.headers(), "application/json")?;
        serde_json::from_slice(&read_bounded_body(response, MAX_BYTES).await?)
            .map_err(|_| ClientError::MalformedStream)
    }

    pub(crate) async fn stop_token_round_trip(
        &self,
        cancellation: CancellationToken,
    ) -> Result<TokenCheck, ClientError> {
        tokio::select! { biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(REQUEST_MS), async {
                let decoded = self.stop_probe_json("detokenize", json!({"tokens":[TOKEN_ID]})).await?;
                // Both requests are fixed; no decoded server text is reflected into another request.
                let encoded = self.stop_probe_json("tokenize", json!({"content":" \n","add_special":false,"parse_special":false})).await?;
                validate_round_trip(&decoded, encoded)
            }) => result.map_err(|_| ClientError::Timeout)?,
        }
    }

    pub(crate) async fn stop_token_completion(
        &self,
        step: Step,
        cancellation: CancellationToken,
    ) -> Result<Observation, ClientError> {
        let started = Instant::now();
        let operation = async {
            let mut response = self
                .client
                .post(self.completion_url.clone())
                .header(AUTHORIZATION, self.config.authorization.clone())
                .timeout(Duration::from_millis(REQUEST_MS))
                .json(&step.payload())
                .send()
                .await
                .map_err(transport_error)?;
            if response.status() != StatusCode::OK {
                return Err(ClientError::UnexpectedStatus(response.status()));
            }
            ensure_content_type(response.headers(), "text/event-stream")?;
            let mut result = Observation::default();
            let mut pending = Vec::new();
            let mut received = 0;
            while let Some(bytes) = response.chunk().await.map_err(transport_error)? {
                received += bytes.len();
                if received > MAX_BYTES {
                    return Err(ClientError::ResponseTooLarge);
                }
                pending.extend_from_slice(&bytes);
                while let Some((end, consumed)) = next_event_boundary(&pending) {
                    let data = event_data(&pending[..end])?;
                    pending.drain(..consumed);
                    if let Some(data) = data {
                        let value = serde_json::from_str(&data)
                            .map_err(|_| ClientError::MalformedStream)?;
                        if result.observe(&value, started.elapsed().as_secs_f64() * 1000.0)? {
                            return Ok(result);
                        }
                    }
                }
            }
            Err(ClientError::MalformedStream)
        };
        tokio::select! { biased;
            () = cancellation.cancelled() => Err(ClientError::Cancelled),
            result = tokio::time::timeout(Duration::from_millis(REQUEST_MS), operation) => result.map_err(|_| ClientError::Timeout)?,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(raw: &str, kind: &str, stop: &str) -> Value {
        json!({"index":0,"content":raw,"stop":true,"stop_type":kind,"stopping_word":stop,
            "truncated":false,"tokens_predicted":4,"timings":{"predicted_n":3,"prompt_n":2,"cache_n":9},"tokens":[715]})
    }

    async fn fixture_request(socket: &mut tokio::net::TcpStream) -> (String, Value) {
        use tokio::io::AsyncReadExt;
        let mut bytes = Vec::new();
        loop {
            let mut buffer = [0; 4096];
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            assert!(bytes.len() < 8192);
            let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&bytes[..end]).unwrap();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().unwrap())
                })
                .unwrap();
            if bytes.len() < end + 4 + length {
                continue;
            }
            assert!(headers.lines().any(|line| {
                line.eq_ignore_ascii_case("authorization: Bearer public-fixture-token")
            }));
            return (
                headers.lines().next().unwrap().to_owned(),
                serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap(),
            );
        }
    }

    fn fixture_client(endpoint: std::net::SocketAddr) -> SemanticClient {
        SemanticClient::new(
            super::super::SemanticClientConfig::new(
                endpoint,
                "fixed-probe-fixture",
                "public-fixture-token",
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn authenticated_fixed_routes_and_native_stream_survive_fragmented_http() {
        use tokio::io::AsyncWriteExt;
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let client = fixture_client(listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            for (route, expected, response) in [
                (
                    "POST /detokenize HTTP/1.1",
                    json!({"tokens":[715]}),
                    json!({"content":" \n"}),
                ),
                (
                    "POST /tokenize HTTP/1.1",
                    json!({"content":" \n","add_special":false,"parse_special":false}),
                    json!({"tokens":[715]}),
                ),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let (actual, payload) = fixture_request(&mut socket).await;
                assert_eq!(actual, route);
                assert_eq!(payload, expected);
                let body = response.to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
            let (mut socket, _) = listener.accept().await.unwrap();
            let (route, payload) = fixture_request(&mut socket).await;
            assert_eq!(route, "POST /completion HTTP/1.1");
            assert_eq!(
                payload,
                Step::Target(Word::Copper, Arm::StopToken).payload()
            );
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"index\":0,\"content\":\"cop").await.unwrap();
            socket
                .write_all(b"per \",\"stop\":false,\"tokens\":[1,715]}\n\n")
                .await
                .unwrap();
            socket
                .write_all(format!("data: {}\n\n", terminal("", "word", "\n")).as_bytes())
                .await
                .unwrap();
        });
        let checked = client
            .stop_token_round_trip(CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(checked.decoded_bytes, [32, 10]);
        assert_eq!(checked.reencoded_tokens, [715]);
        let result = client
            .stop_token_completion(
                Step::Target(Word::Copper, Arm::StopToken),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        Step::Target(Word::Copper, Arm::StopToken)
            .validate(&result)
            .unwrap();
        assert_eq!(result.chunks.len(), 2);
        assert!(result.ttft_ms.is_some());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn cancellation_closes_active_stream_and_never_fabricates_terminal() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let client = fixture_client(listener.local_addr().unwrap());
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            fixture_request(&mut socket).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"index\":0,\"content\":\"copper \",\"stop\":false}\n\n").await.unwrap();
            cancel.cancel();
            let mut byte = [0; 1];
            assert_eq!(
                tokio::time::timeout(Duration::from_secs(1), socket.read(&mut byte))
                    .await
                    .unwrap()
                    .unwrap(),
                0
            );
        });
        assert!(matches!(
            client
                .stop_token_completion(Step::Target(Word::Copper, Arm::StopToken), cancellation)
                .await,
            Err(ClientError::Cancelled)
        ));
        server.await.unwrap();
    }

    #[test]
    fn token_verification_requires_exact_bytes_and_one_exact_id() {
        assert!(validate_round_trip(&json!({"content":" \n"}), json!({"tokens":[715]})).is_ok());
        for text in [" ", "\n", "\n ", " \n\n", "\u{a0}\n"] {
            assert!(
                validate_round_trip(&json!({"content":text}), json!({"tokens":[715]})).is_err()
            );
        }
        for tokens in [
            json!([]),
            json!([220, 198]),
            json!([715, 715]),
            json!(["715"]),
        ] {
            assert!(
                validate_round_trip(&json!({"content":" \n"}), json!({"tokens":tokens})).is_err()
            );
        }
    }

    #[test]
    fn only_grammar_differs_and_all_requests_are_fixed() {
        for word in [Word::Copper, Word::Garten, Word::Window] {
            let mut a = Step::Target(word, Arm::SpaceEos).payload();
            let mut b = Step::Target(word, Arm::StopToken).payload();
            assert_ne!(a["grammar"], b["grammar"]);
            a.as_object_mut().unwrap().remove("grammar");
            b.as_object_mut().unwrap().remove("grammar");
            assert_eq!(a, b);
            assert_eq!(a["n_predict"], 16);
            assert_eq!(a["prompt"], PROMPT);
        }
    }

    #[test]
    fn actual_terminal_and_separator_are_required_and_native_counters_stay_distinct() {
        let step = Step::Target(Word::Copper, Arm::StopToken);
        let mut result = Observation::default();
        result
            .observe(&terminal("copper ", "word", "\n"), 42.0)
            .unwrap();
        step.validate(&result).unwrap();
        let measured = result.terminal.as_ref().unwrap();
        assert_eq!(measured.tokens_predicted, Some(4));
        assert_eq!(measured.predicted_n, Some(3));
        assert_eq!(result.chunks[0].returned_token_ids, Some(vec![715]));
        for (raw, kind, stop) in [
            ("copper", "word", "\n"),
            ("copper ", "eos", ""),
            ("copper ", "limit", ""),
            ("copper ", "word", " "),
        ] {
            let mut result = Observation::default();
            result.observe(&terminal(raw, kind, stop), 10.0).unwrap();
            assert!(step.validate(&result).is_err());
        }
        assert!(step.validate(&Observation::default()).is_err());
        let mut duplicate = Observation::default();
        duplicate.observe(&terminal(".", "eos", ""), 1.0).unwrap();
        assert!(duplicate.observe(&terminal(".", "eos", ""), 2.0).is_err());
    }
}
