#![cfg(feature = "local-model")]

use std::error::Error;
use std::fmt::Write as _;
use std::time::{Duration, Instant};

use badi_broker::NoSuggestionReason;
use badi_broker::provider::{CompletionProvider, ProviderOutcome, ProviderRequest, RequestTrigger};
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
        let client = SemanticClient::new(config(&listener)?)?;
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
            assert_eq!(
                observed.no_suggestion_reason(),
                Some(NoSuggestionReason::BudgetPrefill)
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
    let client = SemanticClient::new(config(&listener)?)?;
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
    assert_eq!(
        client
            .propose_outcome(
                typo,
                CancellationToken::new(),
                true,
                RequestTrigger::Automatic
            )
            .await?,
        ProviderOutcome::NoSuggestion(NoSuggestionReason::BudgetPrefill)
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
        let client = SemanticClient::new(config(&listener)?)?;
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
    let client = SemanticClient::new(config(&listener)?)?;
    drop(listener);
    assert!(matches!(
        client
            .complete_observed(request(), CancellationToken::new())
            .await,
        Err(ClientError::Transport(_))
    ));
    Ok(())
}

/// Runtime requests without a writing budget, such as the authorization
/// challenge, end at the configured request timeout.
#[tokio::test]
async fn configured_request_timeout_bounds_requests_without_a_budget() -> Result<(), Box<dyn Error>>
{
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
            .probe_authorization_challenge(CancellationToken::new())
            .await,
        Err(ClientError::Timeout)
    ));
    let elapsed = started.elapsed();
    assert!(elapsed >= Duration::from_millis(100) && elapsed < Duration::from_millis(500));
    server.await?;
    Ok(())
}

/// Withholds headers for `header_delay`, then streams `pieces` and, unless
/// `stall`, a terminal event. Writes after the client left are ignored.
async fn serve_delayed(
    listener: TcpListener,
    header_delay: Duration,
    pieces: &'static [&'static str],
    stall: bool,
) {
    let (mut socket, _) = listener.accept().await.expect("connection");
    read_request(&mut socket).await;
    tokio::time::sleep(header_delay).await;
    let mut response =
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
            .to_owned();
    for piece in pieces {
        let value = serde_json::json!({"index":0,"content":piece,"stop":false});
        write!(response, "data: {value}\n\n").expect("event");
    }
    if !stall {
        response.push_str(
            "data: {\"index\":0,\"content\":\"\",\"stop\":true,\"stop_type\":\"eos\"}\n\n",
        );
    }
    let _ = socket.write_all(response.as_bytes()).await;
    if stall {
        tokio::time::sleep(Duration::from_millis(1_500)).await;
    }
}

#[tokio::test]
async fn explicit_requests_have_their_own_budget_and_deadline_classes() -> Result<(), Box<dyn Error>>
{
    // (trigger, header delay, pieces, stall, expected output or class, elapsed window)
    for (trigger, delay, pieces, stall, expected, window) in [
        (
            RequestTrigger::Automatic,
            800,
            &[" review it"][..],
            false,
            Err(NoSuggestionReason::BudgetPrefill),
            (550, 700),
        ),
        (
            RequestTrigger::Explicit,
            800,
            &[" review it"][..],
            false,
            Ok(" review it"),
            (800, 1_200),
        ),
        (
            RequestTrigger::Explicit,
            1_500,
            &[" review it"][..],
            false,
            Err(NoSuggestionReason::BudgetPrefill),
            (1_200, 1_350),
        ),
        (
            RequestTrigger::Explicit,
            900,
            &[" review the draft", " tomor"][..],
            true,
            Ok(" review the draft"),
            (1_200, 1_350),
        ),
        (
            RequestTrigger::Explicit,
            900,
            &[" reviewi"][..],
            true,
            Err(NoSuggestionReason::BudgetStream),
            (1_200, 1_350),
        ),
    ] {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let client = SemanticClient::new(config(&listener)?)?;
        let server = tokio::spawn(serve_delayed(
            listener,
            Duration::from_millis(delay),
            pieces,
            stall,
        ));
        let started = Instant::now();
        let observed = client
            .complete_observed_within(request(), CancellationToken::new(), trigger)
            .await?;
        let elapsed = started.elapsed();
        match expected {
            Ok(text) => {
                assert_eq!(observed.disposition(), CompletionDisposition::Suggested);
                assert_eq!(observed.output(), Some(text), "{trigger:?} {delay}");
            }
            Err(reason) => {
                assert_eq!(observed.output(), None, "{trigger:?} {delay}");
                assert_eq!(observed.no_suggestion_reason(), Some(reason));
            }
        }
        assert!(
            elapsed >= Duration::from_millis(window.0) && elapsed < Duration::from_millis(window.1),
            "{trigger:?} {delay}: {elapsed:?}"
        );
        server.abort();
    }
    Ok(())
}

#[tokio::test]
async fn explicit_spelling_and_continuation_share_the_explicit_budget() -> Result<(), Box<dyn Error>>
{
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let client = SemanticClient::new(config(&listener)?)?;
    let server = tokio::spawn(async move {
        // The ambiguous typo spends 700 ms of the explicit budget without an
        // answer; the continuation must still arrive before 1,200 ms.
        let (mut correction, _) = listener.accept().await.expect("correction connection");
        read_request(&mut correction).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let _ = correction
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
                data: {\"index\":0,\"content\":\"teh\",\"stop\":false}\n\n\
                data: {\"index\":0,\"content\":\"\",\"stop\":true,\"stop_type\":\"eos\"}\n\n",
            )
            .await;
        let (mut next, _) = listener.accept().await.expect("continuation connection");
        let payload = read_request(&mut next).await;
        // The unknown word is healed: the model must reproduce it first.
        assert_eq!(payload["prompt"], "This is ");
        next.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n\
            data: {\"index\":0,\"content\":\"teh review it\",\"stop\":false}\n\n\
            data: {\"index\":0,\"content\":\"\",\"stop\":true,\"stop_type\":\"eos\"}\n\n",
        )
        .await
        .expect("continuation");
    });
    let mut typo = request();
    typo.before = "This is teh".to_owned();
    let started = Instant::now();
    let automatic_budget = RequestTrigger::Automatic.writing_budget();
    let outcome = client
        .propose_outcome(
            typo,
            CancellationToken::new(),
            true,
            RequestTrigger::Explicit,
        )
        .await?;
    assert!(started.elapsed() > automatic_budget);
    assert!(started.elapsed() < RequestTrigger::Explicit.writing_budget());
    assert_eq!(
        outcome
            .into_proposal()
            .map(|proposal| proposal.text)
            .as_deref(),
        Some(" review it")
    );
    server.await?;
    Ok(())
}
