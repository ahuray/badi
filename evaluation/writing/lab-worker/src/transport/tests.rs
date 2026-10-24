use super::*;
use badi_broker::semantic::client::SemanticClientConfig;
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
        let payload: Value = serde_json::from_slice(&bytes[end..end + length]).expect("payload");
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
        SemanticClientConfig::new(endpoint, "fixture", "local-test-secret").expect("config"),
    )
    .expect("client");
    (client, task)
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
        let observed = stream(
            &client,
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
async fn token_logprob_capture_is_explicit_and_missing_data_stays_unknown() {
    for include_probability in [true, false] {
        let mut partial = json!({"index":0,"content":"report","stop":false});
        if include_probability {
            partial["completion_probabilities"] = json!([{
                "id": 41,
                "bytes": [114, 101, 112, 111, 114, 116],
                "logprob": -0.4,
                "top_logprobs": []
            }]);
        }
        let body = format!(
            "data: {partial}\n\ndata: {}\n\n",
            json!({"index":0,"content":"","stop":true,"stop_type":"eos"})
        );
        let (client, server) = fixture(body, Duration::ZERO, "text/event-stream").await;
        let observed = stream(
            &client,
            json!({"prompt":"fixture","n_probs":1}),
            550,
            CancellationToken::new(),
            None,
        )
        .await
        .unwrap();
        let payload = server.await.unwrap();
        assert_eq!(payload["n_probs"], 1);
        assert_eq!(observed.raw, "report");
        assert_eq!(observed.token_logprobs.is_some(), include_probability);
        if let Some(tokens) = observed.token_logprobs {
            assert_eq!(tokens.len(), 1);
            assert_eq!(tokens[0].bytes, b"report");
            assert!((tokens[0].logprob + 0.4).abs() < 1e-12);
        }
    }
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
        apply_template(&client, messages.clone(), 550, CancellationToken::new())
            .await
            .unwrap(),
        "<native-assistant>"
    );
    assert_eq!(task.await.unwrap(), json!({"messages":messages}));
    for body in [json!({}), json!({"prompt":""}), json!({"prompt":false})] {
        let (client, task) = fixture(body.to_string(), Duration::ZERO, "application/json").await;
        assert!(matches!(
            apply_template(&client, messages.clone(), 550, CancellationToken::new()).await,
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
        apply_template(&client, json!([]), 550, cancelled).await,
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
        token_count(
            &client,
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
    let observed = stream(
        &client,
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
    let observed = stream(
        &client,
        payload.clone(),
        550,
        CancellationToken::new(),
        None,
    )
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
        let observed = stream(
            &client,
            json!({"prompt":"The"}),
            550,
            CancellationToken::new(),
            None,
        )
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
    let observed = stream(
        &client,
        json!({"prompt":"The"}),
        30,
        CancellationToken::new(),
        None,
    )
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
    let observed = stream(
        &client,
        json!({"prompt":"The"}),
        20,
        CancellationToken::new(),
        None,
    )
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
        stream(&client, json!({"prompt":"The"}), 0, cancellation, None).await,
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
        let (client, task) = fixture(body.to_owned(), Duration::ZERO, "text/event-stream").await;
        assert!(matches!(
            stream(
                &client,
                json!({"prompt":"The"}),
                550,
                CancellationToken::new(),
                None
            )
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
        token_logprobs: None,
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
