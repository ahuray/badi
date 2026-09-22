#![cfg(feature = "local-model")]

use std::error::Error;
use std::time::{Duration, Instant};

use badi_broker::provider::{CompletionProvider, ProviderRequest};
use badi_broker::semantic::client::{
    ClientError, CompletionDisposition, SemanticClient, SemanticClientConfig,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

fn request() -> ProviderRequest {
    ProviderRequest {
        before: "Thanks for this. I will".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    }
}

fn config(listener: &TcpListener) -> Result<SemanticClientConfig, Box<dyn Error>> {
    Ok(SemanticClientConfig::new(
        listener.local_addr()?,
        "header-deadline-fixture",
        "public-fixture-token",
    )?)
}

async fn read_request(socket: &mut TcpStream) -> serde_json::Value {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = socket.read(&mut buffer).await.expect("request bytes");
        assert_ne!(count, 0, "request ended before its body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() < 8192);
        let Some(end) = bytes.windows(4).position(|value| value == b"\r\n\r\n") else {
            continue;
        };
        let length: usize = std::str::from_utf8(&bytes[..end])
            .expect("headers")
            .lines()
            .find_map(|line| {
                let (key, value) = line.split_once(':')?;
                key.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse().expect("body length"))
            })
            .expect("content length");
        if bytes.len() >= end + 4 + length {
            return serde_json::from_slice(&bytes[end + 4..end + 4 + length])
                .expect("request JSON");
        }
    }
}

async fn expect_closed(socket: &mut TcpStream) {
    let mut byte = [0];
    let read = tokio::time::timeout(Duration::from_secs(2), socket.read(&mut byte))
        .await
        .expect("client must close the expired or cancelled request")
        .expect("read EOF");
    assert_eq!(read, 0);
}

async fn write_completion(socket: &mut TcpStream) {
    socket
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
            data: {\"index\":0,\"content\":\" review it\",\"stop\":false}\n\n\
            data: {\"index\":0,\"content\":\"\",\"stop\":true,\"stop_type\":\"eos\"}\n\n",
        )
        .await
        .expect("completion");
}

#[tokio::test]
async fn writing_header_budget_and_cancellation_allow_next_persistent_request()
-> Result<(), Box<dyn Error>> {
    for cancel_pending in [false, true] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let client = SemanticClient::new(config(&listener)?.for_writing())?;
        let cancellation = CancellationToken::new();
        let cancel = cancellation.clone();
        let server = tokio::spawn(async move {
            let (mut stalled, _) = listener.accept().await.expect("first connection");
            read_request(&mut stalled).await;
            if cancel_pending {
                cancel.cancel();
            }
            // Withhold headers completely, as llama.cpp can during prefill.
            expect_closed(&mut stalled).await;
            let (mut next, _) = listener.accept().await.expect("next connection");
            read_request(&mut next).await;
            write_completion(&mut next).await;
        });
        let observed = client.complete_observed(request(), cancellation).await;
        if cancel_pending {
            assert!(matches!(observed, Err(ClientError::Cancelled)));
        } else {
            let observed = observed?;
            assert_eq!(
                observed.disposition(),
                CompletionDisposition::ModelAbstained
            );
            assert_eq!(observed.output(), None);
            assert_eq!(observed.ttft(), None);
            assert!(observed.request_body_bytes() > 0);
            assert_eq!(observed.response_body_bytes(), 0);
            assert!(observed.elapsed() >= Duration::from_millis(550));
            assert!(observed.elapsed() < Duration::from_millis(700));
        }
        let next = client
            .complete_observed(request(), CancellationToken::new())
            .await?;
        assert_eq!(next.disposition(), CompletionDisposition::Suggested);
        assert_eq!(next.output(), Some(" review it"));
        server.await?;
    }
    Ok(())
}

#[tokio::test]
async fn exhausted_spelling_header_budget_does_not_submit_a_continuation()
-> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(config(&listener)?.for_writing())?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("correction connection");
        let payload = read_request(&mut socket).await;
        assert!(payload["prompt"].as_str().expect("prompt").contains("teh"));
        expect_closed(&mut socket).await;
        assert!(
            tokio::time::timeout(Duration::from_millis(100), listener.accept())
                .await
                .is_err(),
            "spent correction budget cannot enqueue a continuation"
        );
    });
    let mut typo = request();
    typo.before = "This is teh".to_owned();
    assert!(
        client
            .propose(typo, CancellationToken::new(), true)
            .await?
            .is_none()
    );
    server.await?;
    Ok(())
}

#[tokio::test]
async fn writing_header_deadline_preserves_actual_http_errors() -> Result<(), Box<dyn Error>> {
    for (response, expected) in [
        (
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n",
            "status",
        ),
        (
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n",
            "status",
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 0\r\n\r\n",
            "type",
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: invalid\n\n",
            "stream",
        ),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let client = SemanticClient::new(config(&listener)?.for_writing())?;
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("connection");
            read_request(&mut socket).await;
            socket
                .write_all(response.as_bytes())
                .await
                .expect("response");
        });
        let error = client
            .complete_observed(request(), CancellationToken::new())
            .await
            .expect_err("real error must not become a budget abstention");
        assert!(match expected {
            "status" => matches!(error, ClientError::UnexpectedStatus(_)),
            "type" => matches!(error, ClientError::UnexpectedContentType),
            "stream" => matches!(error, ClientError::MalformedStream),
            _ => unreachable!(),
        });
        server.await?;
    }
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(config(&listener)?.for_writing())?;
    drop(listener);
    assert!(matches!(
        client
            .complete_observed(request(), CancellationToken::new())
            .await,
        Err(ClientError::Transport(_))
    ));
    Ok(())
}

#[tokio::test]
async fn historical_semantic_header_wait_keeps_its_configured_deadline()
-> Result<(), Box<dyn Error>> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(config(&listener)?)?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("connection");
        read_request(&mut socket).await;
        tokio::time::sleep(Duration::from_millis(600)).await;
        write_completion(&mut socket).await;
    });
    let observed = client
        .complete_observed(request(), CancellationToken::new())
        .await?;
    assert_eq!(observed.disposition(), CompletionDisposition::Suggested);
    server.await?;

    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(
        config(&listener)?.with_timeouts(Duration::from_millis(50), Duration::from_millis(100))?,
    )?;
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("connection");
        read_request(&mut socket).await;
        expect_closed(&mut socket).await;
    });
    let started = Instant::now();
    assert!(matches!(
        client
            .complete_observed(request(), CancellationToken::new())
            .await,
        Err(ClientError::Timeout)
    ));
    assert!(started.elapsed() < Duration::from_millis(500));
    server.await?;
    Ok(())
}
