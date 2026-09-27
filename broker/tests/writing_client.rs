#![cfg(feature = "local-model")]

use std::error::Error;
use std::time::Duration;

use badi_broker::NoSuggestionReason;
use badi_broker::provider::{CompletionProvider, ProviderOutcome, ProviderRequest, RequestTrigger};
use badi_broker::semantic::client::{
    ClientError, CompletionDisposition, SemanticClient, SemanticClientConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

fn request(before: &str, language: Option<&str>) -> ProviderRequest {
    ProviderRequest {
        before: before.to_owned(),
        after: String::new(),
        language: language.map(str::to_owned),
    }
}

#[tokio::test]
async fn unsupported_writing_requests_never_reach_the_runtime() -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(
        SemanticClientConfig::new(
            listener.local_addr()?,
            "writing-fixture",
            "public-fixture-token",
        )?
        .for_writing(),
    )?;
    for language in [None, Some("fr"), Some("ar")] {
        let result = client
            .complete_observed(request("fixture:valid", language), CancellationToken::new())
            .await?;
        assert_eq!(
            result.disposition(),
            CompletionDisposition::LanguageAbstained
        );
        assert_eq!(
            result.no_suggestion_reason(),
            Some(NoSuggestionReason::RequestAbstained)
        );
        assert_eq!(result.request_body_bytes(), 0);
    }
    let mut middle = request("متن", Some("fa"));
    middle.after = "other text".to_owned();
    assert_eq!(
        client
            .complete_observed(middle, CancellationToken::new())
            .await?
            .disposition(),
        CompletionDisposition::LanguageAbstained
    );
    assert!(matches!(
        client
            .complete_observed(
                request("fixture:valid", Some("de--DE")),
                CancellationToken::new()
            )
            .await,
        Err(ClientError::InvalidRequest)
    ));
    // A joiner just typed, or beside a space, abstains as a request-side class
    // rather than counting as a runtime failure.
    for before in ["می‌ شود", "می‌", "\u{200c}"] {
        let result = client
            .complete_observed(request(before, Some("fa")), CancellationToken::new())
            .await?;
        assert_eq!(
            result.no_suggestion_reason(),
            Some(NoSuggestionReason::RequestAbstained)
        );
        assert_eq!(result.request_body_bytes(), 0);
        for allow_replacement in [false, true] {
            assert!(matches!(
                client
                    .propose_outcome(
                        request(before, Some("fa")),
                        CancellationToken::new(),
                        allow_replacement,
                        RequestTrigger::Automatic,
                    )
                    .await,
                Ok(ProviderOutcome::NoSuggestion(
                    NoSuggestionReason::RequestAbstained
                ))
            ));
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn healing_that_empties_the_prompt_abstains_before_any_request() -> Result<(), Box<dyn Error>>
{
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(
        SemanticClientConfig::new(
            listener.local_addr()?,
            "writing-fixture",
            "public-fixture-token",
        )?
        .for_writing(),
    )?;
    // Only whitespace, healed or not, or one unfinished English word leaves no
    // context. The runtime answers that with a malformed chunk, so these
    // abstain before any request instead of counting an error.
    let spaces = " ".repeat(200);
    for (before, language, replacement) in [
        (" ", "en", true),
        ("   ", "de-DE", true),
        (" ", "fa", true),
        (spaces.as_str(), "en-US", true),
        ("Hel", "en", false),
    ] {
        let result = client
            .complete_observed(request(before, Some(language)), CancellationToken::new())
            .await?;
        assert_eq!(
            (result.no_suggestion_reason(), result.request_body_bytes()),
            (Some(NoSuggestionReason::RequestAbstained), 0),
            "{language} {before:?}"
        );
        for (allow_replacement, trigger) in [
            (false, RequestTrigger::Automatic),
            (replacement, RequestTrigger::Explicit),
        ] {
            assert!(matches!(
                client
                    .propose_outcome(
                        request(before, Some(language)),
                        CancellationToken::new(),
                        allow_replacement,
                        trigger,
                    )
                    .await,
                Ok(ProviderOutcome::NoSuggestion(
                    NoSuggestionReason::RequestAbstained
                ))
            ));
        }
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

async fn stream_probe(
    before: &str,
    language: &str,
    pieces: &[&str],
    stall: bool,
    cancellation: CancellationToken,
) -> Result<Option<String>, Box<dyn Error>> {
    Ok(
        stream_outcome(before, language, pieces, stall, cancellation)
            .await?
            .into_proposal()
            .map(|proposal| proposal.text),
    )
}

async fn stream_outcome(
    before: &str,
    language: &str,
    pieces: &[&str],
    stall: bool,
    cancellation: CancellationToken,
) -> Result<ProviderOutcome, Box<dyn Error>> {
    stream_outcome_until(before, language, pieces, stall, None, cancellation).await
}

/// `stopping_word` ends the stream at a runtime stop word instead of EOS.
async fn stream_outcome_until(
    before: &str,
    language: &str,
    pieces: &[&str],
    stall: bool,
    stopping_word: Option<&'static str>,
    cancellation: CancellationToken,
) -> Result<ProviderOutcome, Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = listener.local_addr()?;
    let pieces: Vec<_> = pieces.iter().map(|value| (*value).to_owned()).collect();
    let healing = before == "Please review the docum";
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("HTTP connection");
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let read = socket.read(&mut buffer).await.expect("HTTP request");
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            assert!(bytes.len() < 8192);
            let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&bytes[..end]).expect("headers");
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("body size"))
                })
                .expect("content length");
            if bytes.len() < end + 4 + length {
                continue;
            }
            let payload: serde_json::Value =
                serde_json::from_slice(&bytes[end + 4..]).expect("request JSON");
            assert_eq!(payload["cache_prompt"], true);
            if healing {
                assert_eq!(payload["prompt"], "Please review the ");
                assert!(
                    payload["grammar"]
                        .as_str()
                        .expect("grammar")
                        .starts_with("root ::= \"docum\"")
                );
            }
            break;
        }
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("HTTP headers");
        for piece in pieces {
            let value = serde_json::json!({"index":0,"content":piece,"stop":false});
            socket
                .write_all(format!("data: {value}\n\n").as_bytes())
                .await
                .expect("stream content");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        if stall {
            tokio::time::sleep(Duration::from_millis(650)).await;
        } else {
            let value = stopping_word.map_or_else(
                || serde_json::json!({"index":0,"content":"","stop":true,"stop_type":"eos"}),
                |word| serde_json::json!({"index":0,"content":"","stop":true,"stop_type":"word","stopping_word":word}),
            );
            let _ = socket
                .write_all(format!("data: {value}\n\n").as_bytes())
                .await;
        }
    });
    let client = SemanticClient::new(
        SemanticClientConfig::new(endpoint, "healing-fixture", "public-fixture-token")?
            .for_writing(),
    )?;
    let result = client
        .propose_outcome(
            request(before, Some(language)),
            cancellation,
            false,
            RequestTrigger::Automatic,
        )
        .await;
    server.await?;
    Ok(result?)
}

#[tokio::test]
async fn no_suggestion_outcomes_name_their_content_free_class() -> Result<(), Box<dyn Error>> {
    for (before, language, pieces, stall, expected) in [
        (
            "fixture:valid",
            "en",
            &[" بررسی کنید"][..],
            false,
            NoSuggestionReason::OutputRejected,
        ),
        (
            "Neue Dokum",
            "de",
            &["entation ist verfügbar"][..],
            false,
            NoSuggestionReason::OutputRejected,
        ),
        (
            "Please review the docum",
            "en",
            &["doc"][..],
            false,
            NoSuggestionReason::ModelAbstained,
        ),
        (
            "Please review the docum",
            "en",
            &["documentatio"][..],
            true,
            NoSuggestionReason::BudgetStream,
        ),
    ] {
        assert_eq!(
            stream_outcome(before, language, pieces, stall, CancellationToken::new()).await?,
            ProviderOutcome::NoSuggestion(expected),
            "{before:?} {pieces:?}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn writing_output_enforces_script_and_orthographic_boundaries() -> Result<(), Box<dyn Error>>
{
    for (language, output, accepted) in [
        ("en", " next step", true),
        ("de-DE", " für Ihre Unterstützung", true),
        ("fa-IR", " بررسی کنید", true),
        ("fa", " می‌شود", true),
        ("en", " بررسی کنید", false),
        ("de", " 世界", false),
        ("fa", " next step", false),
        ("fa", " 👍", false),
        ("fa", " می‌ شود", false),
        ("fa", "<think>hidden", false),
    ] {
        let result = stream_probe(
            "fixture:valid",
            language,
            &[output],
            false,
            CancellationToken::new(),
        )
        .await?;
        assert_eq!(
            result.as_deref(),
            accepted.then_some(output),
            "language={language}"
        );
    }
    Ok(())
}

#[tokio::test]
async fn token_healing_never_displays_echoes_or_different_stems() -> Result<(), Box<dyn Error>> {
    let before = "Please review the docum";
    assert_eq!(
        stream_probe(
            before,
            "en",
            &["doc", "ument and provide a "],
            false,
            CancellationToken::new()
        )
        .await?,
        Some("ent and provide a".to_owned())
    );
    for pieces in [&["doc"][..], &["docum"][..], &["report and provide a "][..]] {
        assert_eq!(
            stream_probe(before, "en", pieces, false, CancellationToken::new()).await?,
            None
        );
    }
    Ok(())
}

#[tokio::test]
async fn slow_stream_salvages_only_complete_words_and_respects_cancellation()
-> Result<(), Box<dyn Error>> {
    let before = "Please review the docum";
    assert_eq!(
        stream_probe(
            before,
            "en",
            &["document and prov"],
            true,
            CancellationToken::new()
        )
        .await?,
        Some("ent and".to_owned())
    );
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(40)).await;
        cancel.cancel();
    });
    assert!(
        stream_probe(before, "en", &["document and prov"], true, cancellation)
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn spelling_and_continuation_share_one_deadline() -> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = listener.local_addr()?;
    let server = tokio::spawn(async move {
        for (text, delay, terminal) in [("teh", 250, true), ("teh next word parti", 0, false)] {
            let (mut socket, _) = listener.accept().await.expect("HTTP connection");
            let mut data = Vec::new();
            let mut buffer = [0; 4096];
            while !data.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                let count = socket.read(&mut buffer).await.expect("request");
                assert_ne!(count, 0);
                data.extend_from_slice(&buffer[..count]);
            }
            let end = data
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .expect("header end");
            let length: usize = std::str::from_utf8(&data[..end])
                .expect("headers")
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("length"))
                })
                .expect("content length");
            while data.len() < end + 4 + length {
                let count = socket.read(&mut buffer).await.expect("request body");
                assert_ne!(count, 0);
                data.extend_from_slice(&buffer[..count]);
            }
            tokio::time::sleep(Duration::from_millis(delay)).await;
            socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await.expect("headers");
            let token = serde_json::json!({"index":0,"content":text,"stop":false});
            socket
                .write_all(format!("data: {token}\n\n").as_bytes())
                .await
                .expect("token");
            if terminal {
                let stop =
                    serde_json::json!({"index":0,"content":"","stop":true,"stop_type":"eos"});
                socket
                    .write_all(format!("data: {stop}\n\n").as_bytes())
                    .await
                    .expect("stop");
            } else {
                tokio::time::sleep(Duration::from_millis(650)).await;
            }
        }
    });
    let client = SemanticClient::new(
        SemanticClientConfig::new(endpoint, "deadline-fixture", "public-fixture-token")?
            .for_writing(),
    )?;
    let started = std::time::Instant::now();
    let result = client
        .propose(
            request("This is teh", Some("en")),
            CancellationToken::new(),
            true,
        )
        .await?;
    assert!(
        started.elapsed() < Duration::from_millis(600),
        "correction cannot reset the request budget"
    );
    assert_eq!(result.expect("separator-proven suffix").text, " next word");
    server.await?;
    Ok(())
}

#[tokio::test]
async fn unambiguous_spelling_does_not_require_an_inference_round_trip()
-> Result<(), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(
        SemanticClientConfig::new(
            listener.local_addr()?,
            "dictionary-fixture",
            "public-fixture-token",
        )?
        .for_writing(),
    )?;
    for (before, expected, original) in [
        ("The shipping adress", "address", "adress"),
        ("Please give an exampel", "example", "exampel"),
        ("That helps definately", "definitely", "definately"),
        ("The shipping adress ", "address ", "adress "),
        ("Please give an exampel ", "example ", "exampel "),
    ] {
        let result = client
            .propose(request(before, Some("en")), CancellationToken::new(), true)
            .await?
            .expect("dictionary correction");
        assert_eq!(result.text, expected);
        assert_eq!(result.replace_before.as_deref(), Some(original));
    }
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        client
            .propose(request("The shipping adress", Some("en")), cancelled, true)
            .await,
        Err(badi_broker::provider::ProviderError::Cancelled)
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
    drop(listener);
    // Without exact replacement authority or English eligibility, the same
    // typo cannot take the dictionary edit path, even if inference is down.
    for (language, replacement) in [("en", false), ("de", true)] {
        assert!(matches!(
            client
                .propose(
                    request("The shipping adress", Some(language)),
                    CancellationToken::new(),
                    replacement,
                )
                .await,
            Err(badi_broker::provider::ProviderError::Unavailable)
        ));
    }
    Ok(())
}

#[tokio::test]
async fn fact_fence_counts_invented_numbers_as_rejected_output() -> Result<(), Box<dyn Error>> {
    for (before, language, pieces, stall, stopping_word) in [
        (
            "Ich freue mich auf",
            "de",
            &[" die Teilnahme am 1"][..],
            false,
            Some("."),
        ),
        // A healed trailing space is echoed first, as the runtime grammar
        // requires; the fence then sees only the continuation.
        (
            "The survey ran in ",
            "en",
            &[" ", "2024, but"][..],
            false,
            None,
        ),
        (
            "This essay is ",
            "en",
            &[" 100% original and"][..],
            false,
            None,
        ),
        (
            "Our office is open from ",
            "en",
            &[" ", "10:00 AM to"][..],
            false,
            None,
        ),
        (
            "جلسه در اتاق ۴۲ است. بعد به اتاق",
            "fa",
            &[" ۴۳ برویم"][..],
            false,
            None,
        ),
        // A known number still cannot end at the re-appended stop word.
        (
            "Die Frist ist der 1. Mai. Wir treffen uns am",
            "de",
            &[" Montag, dem 1"][..],
            false,
            Some("."),
        ),
        // Words salvaged at the stream deadline pass the same fence.
        ("Our office opens at", "en", &[" 10 AM and"][..], true, None),
    ] {
        assert_eq!(
            stream_outcome_until(
                before,
                language,
                pieces,
                stall,
                stopping_word,
                CancellationToken::new()
            )
            .await?,
            ProviderOutcome::NoSuggestion(NoSuggestionReason::OutputRejected),
            "{before:?} {pieces:?}"
        );
    }
    for (before, language, pieces, stopping_word, expected) in [
        (
            "Meet me at room 42, then room",
            "en",
            &[" 42 again"][..],
            None,
            " 42 again",
        ),
        (
            "جلسه در اتاق ۴۲ است. بعد به اتاق",
            "fa",
            &[" ۴۲ برویم"][..],
            None,
            " ۴۲ برویم",
        ),
        (
            "Ich freue mich auf",
            "de",
            &[" die Teilnahme"][..],
            Some("."),
            " die Teilnahme.",
        ),
    ] {
        let outcome = stream_outcome_until(
            before,
            language,
            pieces,
            false,
            stopping_word,
            CancellationToken::new(),
        )
        .await?;
        assert_eq!(
            outcome.into_proposal().map(|proposal| proposal.text),
            Some(expected.to_owned()),
            "{before:?} {pieces:?}"
        );
    }
    Ok(())
}

enum WarmUpReply {
    Json,
    Status(&'static str),
    EventStream,
    Stall,
}

async fn warm_up_endpoint(
    reply: WarmUpReply,
) -> Result<
    (
        std::net::SocketAddr,
        tokio::task::JoinHandle<(String, serde_json::Value)>,
    ),
    Box<dyn Error>,
> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = listener.local_addr()?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("HTTP connection");
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let (headers, payload) = loop {
            let read = socket.read(&mut buffer).await.expect("HTTP request");
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let headers = std::str::from_utf8(&bytes[..end])
                .expect("headers")
                .to_owned();
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("body size"))
                })
                .expect("content length");
            if bytes.len() >= end + 4 + length {
                let payload = serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                    .expect("request JSON");
                break (headers, payload);
            }
        };
        let response = match reply {
            WarmUpReply::Json => {
                let body = r#"{"content":" time","stop":true}"#;
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
            }
            WarmUpReply::Status(status) => format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
            ),
            WarmUpReply::EventStream => {
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                    .to_owned()
            }
            WarmUpReply::Stall => {
                tokio::time::sleep(Duration::from_millis(800)).await;
                String::new()
            }
        };
        let _ = socket.write_all(response.as_bytes()).await;
        (headers, payload)
    });
    Ok((endpoint, server))
}

#[tokio::test]
async fn warm_up_sends_one_fixed_tiny_completion_and_discards_its_reply()
-> Result<(), Box<dyn Error>> {
    let (endpoint, server) = warm_up_endpoint(WarmUpReply::Json).await?;
    let client = SemanticClient::new(
        SemanticClientConfig::new(endpoint, "warm-up-fixture", "public-fixture-token")?
            .for_writing(),
    )?;
    client
        .warm_up(Duration::from_secs(2), CancellationToken::new())
        .await?;
    let (headers, payload) = server.await?;
    assert!(headers.starts_with("POST /completion HTTP/1.1\r\n"));
    assert!(
        headers
            .lines()
            .any(|line| line.eq_ignore_ascii_case("authorization: Bearer public-fixture-token"))
    );
    assert_eq!(
        payload,
        serde_json::json!({
            "prompt": "Thank you for your",
            "n_predict": 2,
            "temperature": 0.0,
            "seed": 42,
            "stream": false,
            "cache_prompt": false,
        })
    );
    Ok(())
}

#[tokio::test]
async fn warm_up_failures_are_bounded_and_classified_without_content() -> Result<(), Box<dyn Error>>
{
    for (reply, class) in [
        (
            WarmUpReply::Status("503 Service Unavailable"),
            "http_status",
        ),
        (WarmUpReply::EventStream, "malformed_response"),
        (WarmUpReply::Stall, "timeout"),
    ] {
        let (endpoint, server) = warm_up_endpoint(reply).await?;
        let client = SemanticClient::new(SemanticClientConfig::new(
            endpoint,
            "warm-up-fixture",
            "public-fixture-token",
        )?)?;
        let started = std::time::Instant::now();
        let error = client
            .warm_up(Duration::from_millis(150), CancellationToken::new())
            .await
            .expect_err("failed warm-up");
        assert!(started.elapsed() < Duration::from_millis(600), "{class}");
        assert_eq!(error.class(), class);
        server.abort();
    }

    let (endpoint, server) = warm_up_endpoint(WarmUpReply::Stall).await?;
    let client = SemanticClient::new(SemanticClientConfig::new(
        endpoint,
        "warm-up-fixture",
        "public-fixture-token",
    )?)?;
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
    });
    let started = std::time::Instant::now();
    assert!(matches!(
        client.warm_up(Duration::from_secs(2), cancellation).await,
        Err(ClientError::Cancelled)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    server.abort();

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(SemanticClientConfig::new(
        listener.local_addr()?,
        "warm-up-fixture",
        "public-fixture-token",
    )?)?;
    assert!(matches!(
        client
            .warm_up(Duration::ZERO, CancellationToken::new())
            .await,
        Err(ClientError::InvalidConfig(_))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(10), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

/// Serves one streamed completion and returns the payload the client sent.
async fn healed_exchange(
    before: &str,
    language: &str,
    pieces: &[&str],
) -> Result<(serde_json::Value, ProviderOutcome), Box<dyn Error>> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    let endpoint = listener.local_addr()?;
    let pieces: Vec<_> = pieces.iter().map(|value| (*value).to_owned()).collect();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("HTTP connection");
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        let payload: serde_json::Value = loop {
            let read = socket.read(&mut buffer).await.expect("HTTP request");
            assert_ne!(read, 0);
            bytes.extend_from_slice(&buffer[..read]);
            let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
                continue;
            };
            let length: usize = std::str::from_utf8(&bytes[..end])
                .expect("headers")
                .lines()
                .find_map(|line| {
                    let (key, value) = line.split_once(':')?;
                    key.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse().expect("body size"))
                })
                .expect("content length");
            if bytes.len() >= end + 4 + length {
                break serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                    .expect("request JSON");
            }
        };
        socket
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .await
            .expect("HTTP headers");
        for piece in pieces {
            let value = serde_json::json!({"index":0,"content":piece,"stop":false});
            socket
                .write_all(format!("data: {value}\n\n").as_bytes())
                .await
                .expect("stream content");
        }
        let stop = serde_json::json!({"index":0,"content":"","stop":true,"stop_type":"eos"});
        let _ = socket
            .write_all(format!("data: {stop}\n\n").as_bytes())
            .await;
        payload
    });
    let client = SemanticClient::new(
        SemanticClientConfig::new(endpoint, "space-fixture", "public-fixture-token")?.for_writing(),
    )?;
    let outcome = client
        .propose_outcome(
            request(before, Some(language)),
            CancellationToken::new(),
            false,
            RequestTrigger::Automatic,
        )
        .await?;
    Ok((server.await?, outcome))
}

#[tokio::test]
async fn production_heals_a_trailing_space_and_never_shows_the_echo() -> Result<(), Box<dyn Error>>
{
    for (before, language, pieces, prompt, echo, shown) in [
        (
            // Two words after the sentence end keep the previous sentence.
            "Thanks for sharing the draft. I will ",
            "en",
            &[" ", "read it", " tonight"][..],
            "Thanks for sharing the draft. I will",
            " ",
            " read it tonight",
        ),
        (
            "Vielen Dank für Ihre ",
            "de-DE",
            &[" Nachricht"][..],
            "Vielen Dank für Ihre",
            " ",
            " Nachricht",
        ),
        (
            "لطفا این گزارش را ",
            "fa",
            &[" ", "بررسی کنید"][..],
            "لطفا این گزارش را",
            " ",
            " بررسی کنید",
        ),
        (
            "Please review the  ",
            "en",
            &["  document"][..],
            "Please review the",
            "  ",
            "  document",
        ),
    ] {
        let (payload, outcome) = healed_exchange(before, language, pieces).await?;
        if !badi_broker::writing::heals_trailing_space(language) {
            // A language not promoted keeps the unhealed prompt.
            assert_eq!(payload["prompt"], format!("{prompt}{echo}"), "{before:?}");
            assert!(payload.get("grammar").is_none(), "{before:?}");
            continue;
        }
        assert_eq!(payload["prompt"], prompt, "{before:?}");
        assert_eq!(
            payload["grammar"],
            format!(
                "root ::= {} [^<>\\n\\r`]*",
                serde_json::to_string(echo).expect("echo literal")
            )
        );
        assert_eq!(payload["n_predict"], 8, "healing keeps the token budget");
        // The typed space stays in the document; the suggestion continues it.
        let text = outcome.into_proposal().map(|proposal| proposal.text);
        assert_eq!(text.as_deref(), shown.strip_prefix(echo), "{before:?}");
    }
    // An output that does not reproduce the removed bytes is never shown.
    for pieces in [&["Nachricht"][..], &["\u{a0}Nachricht"][..], &[" "][..]] {
        let (_, outcome) = healed_exchange("Vielen Dank für Ihre ", "de", pieces).await?;
        if !badi_broker::writing::heals_trailing_space("de") {
            break;
        }
        assert_eq!(
            outcome,
            ProviderOutcome::NoSuggestion(NoSuggestionReason::ModelAbstained),
            "{pieces:?}"
        );
    }
    // Controls without a trailing ASCII space keep the unhealed payload.
    for (before, language) in [
        ("Vielen Dank für Ihre", "de"),
        ("Please review the\t", "en"),
        ("لطفا این گزارش را", "fa"),
    ] {
        let (payload, _) = healed_exchange(before, language, &[" Nachricht"]).await?;
        assert_eq!(payload["prompt"], before);
        assert!(payload.get("grammar").is_none(), "{before:?}");
        assert_eq!(payload["n_predict"], 8);
    }
    Ok(())
}

/// Pinned-model trace (2026-09-26): after the healed echo, `the client should `
/// streamed ` be able to re-try the request`. The four-word limit counted the
/// UAX #29 pieces of `re-try` and displayed the partial word `be able to re`.
#[tokio::test]
async fn the_word_limit_never_displays_part_of_a_hyphenated_word() -> Result<(), Box<dyn Error>> {
    let before = "If the request times out, the client should ";
    let pieces = [" be", " able", " to", " re", "-", "try", " the", " request"];
    let (payload, outcome) = healed_exchange(before, "en", &pieces).await?;
    let text = outcome.into_proposal().map(|proposal| proposal.text);
    if badi_broker::writing::heals_trailing_space("en") {
        assert_eq!(payload["prompt"], before.trim_end());
        assert_eq!(text.as_deref(), Some("be able to re-try"));
    }
    for (before, language, pieces, shown) in [
        (
            "Bitte senden Sie",
            "de",
            &[" die", " E", "-", "Mail", " an", " Herrn", " Meyer"][..],
            " die E-Mail an Herrn",
        ),
        (
            "We need",
            "en",
            &[
                " a", " state", "-of-", "the", "-art", " tool", " for", " this",
            ][..],
            " a state-of-the-art tool for",
        ),
    ] {
        let (_, outcome) = healed_exchange(before, language, pieces).await?;
        let text = outcome.into_proposal().map(|proposal| proposal.text);
        assert_eq!(text.as_deref(), Some(shown), "{before:?}");
    }
    Ok(())
}
