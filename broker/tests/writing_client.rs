#![cfg(feature = "local-model")]

use std::error::Error;
use std::time::Duration;

use badi_broker::provider::{CompletionProvider, ProviderRequest};
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
    for input in [
        request("fixture:valid", Some("de--DE")),
        request("می‌ شود", Some("fa")),
    ] {
        assert!(matches!(
            client
                .complete_observed(input, CancellationToken::new())
                .await,
            Err(ClientError::InvalidRequest)
        ));
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
            let value = serde_json::json!({"index":0,"content":"","stop":true,"stop_type":"eos"});
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
        .propose(request(before, Some(language)), cancellation, false)
        .await;
    server.await?;
    Ok(result?.map(|proposal| proposal.text))
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
