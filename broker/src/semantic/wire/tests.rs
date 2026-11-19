use tokio::net::TcpListener;

use super::*;

/// Serves one connection: reads the request head, writes each reply
/// piece after a short pause, then keeps the connection open for `hold`.
async fn serve(pieces: Vec<Vec<u8>>, hold: Duration) -> SocketAddr {
    serve_paced(pieces, Duration::from_millis(5), hold).await
}

async fn serve_paced(pieces: Vec<Vec<u8>>, pause: Duration, hold: Duration) -> SocketAddr {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await.expect("listener");
    let endpoint = listener.local_addr().expect("endpoint");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("connection");
        let mut request = Vec::new();
        while find(&request, b"\r\n\r\n").is_none() {
            let mut buffer = [0; 1_024];
            match socket.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => request.extend_from_slice(&buffer[..read]),
            }
        }
        for piece in pieces {
            if socket.write_all(&piece).await.is_err() {
                return;
            }
            tokio::time::sleep(pause).await;
        }
        tokio::time::sleep(hold).await;
    });
    endpoint
}

async fn get(endpoint: SocketAddr, timeout: Duration) -> Result<Response, ClientError> {
    let request = Request {
        method: Method::Get,
        path: "/health",
        authorization: "Bearer fixture",
        body: None,
    };
    exchange(
        endpoint,
        request,
        Duration::from_millis(200),
        Instant::now() + timeout,
    )
    .await
}

async fn body_of(pieces: &[&[u8]]) -> Result<Vec<u8>, ClientError> {
    let pieces = pieces.iter().map(|piece| piece.to_vec()).collect();
    let response = get(serve(pieces, Duration::ZERO).await, Duration::from_secs(2)).await?;
    read_bounded_body(response, 1_024).await
}

#[test]
fn requests_are_encoded_once_with_exact_length_and_close() {
    let post = Request {
        method: Method::Post,
        path: "/completion",
        authorization: "Bearer token",
        body: Some(b"{\"a\":1}"),
    };
    assert_eq!(
        post.encode("127.0.0.1:8080"),
        b"POST /completion HTTP/1.1\r\nhost: 127.0.0.1:8080\r\nauthorization: Bearer token\r\n\
          content-type: application/json\r\ncontent-length: 7\r\nconnection: close\r\n\r\n{\"a\":1}"
    );
    let get = Request {
        method: Method::Get,
        path: "/health",
        body: None,
        ..post
    };
    assert_eq!(
        get.encode("[::1]:9"),
        b"GET /health HTTP/1.1\r\nhost: [::1]:9\r\nauthorization: Bearer token\r\nconnection: close\r\n\r\n"
    );
}

#[tokio::test]
async fn only_loopback_endpoints_are_contacted() {
    for endpoint in ["192.0.2.1:80", "[2001:db8::1]:80", "0.0.0.0:80"] {
        assert!(matches!(
            get(endpoint.parse().expect("endpoint"), Duration::from_secs(1)).await,
            Err(ClientError::InvalidConfig("endpoint"))
        ));
    }
}

#[tokio::test]
async fn bodies_decode_by_chunks_length_or_close_across_split_reads() {
    let chunked = body_of(&[
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n5",
        b"\r\nhel",
        b"lo\r",
        b"\n6;name=value\r\n world\r\n0\r\n",
        b"Trailer-Field: ignored\r\n\r\n",
    ])
    .await;
    assert_eq!(chunked.expect("chunked body"), b"hello world");
    let length = body_of(&[b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhel", b"lo"]).await;
    assert_eq!(length.expect("length body"), b"hello");
    // Bytes after the declared length are not part of the body.
    let exact = body_of(&[b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\n\r\nabcdef"]).await;
    assert_eq!(exact.expect("exact body"), b"abc");
    let close = body_of(&[b"HTTP/1.0 200 OK\r\n\r\nab", b"cd"]).await;
    assert_eq!(close.expect("close-delimited body"), b"abcd");
    let empty = body_of(&[b"HTTP/1.1 204 No Content\r\nContent-Length: 9\r\n\r\n"]).await;
    assert_eq!(empty.expect("no body"), b"");
}

#[tokio::test]
async fn broken_framing_is_a_transport_failure() {
    let long_chunk_line = [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1;"[..],
        &[b'x'; 200],
    ]
    .concat();
    let long_head = [
        &b"HTTP/1.1 200 OK\r\nX-Long: "[..],
        &[b'x'; MAX_HEAD_BYTES],
        b"\r\n\r\n",
    ]
    .concat();
    for reply in [
        &b"HTTP/1.1 200"[..],
        b"HTTP/2 200 OK\r\n\r\n",
        b"HTTP/1.1 20 OK\r\n\r\n",
        b"HTTP/1.1 200OK\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nno colon\r\n\r\n",
        b"HTTP/1.1 200 OK\r\n folded: value\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nabc",
        b"HTTP/1.1 200 OK\r\nContent-Length: 1\r\nContent-Length: 2\r\n\r\nab",
        b"HTTP/1.1 200 OK\r\nContent-Length: -1\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n11111111111111111\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nabX\r\n0\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nab\r\n",
        &long_chunk_line,
        &long_head,
    ] {
        let result = body_of(&[reply]).await;
        assert!(
            matches!(result, Err(ClientError::Transport(_))),
            "{:?}: {result:?}",
            String::from_utf8_lossy(&reply[..reply.len().min(64)])
        );
    }
}

#[tokio::test]
async fn body_limits_hold_for_declared_and_undeclared_lengths() {
    let declared = [
        &b"HTTP/1.1 200 OK\r\nContent-Length: 1025\r\n\r\n"[..],
        &[b'x'; 1_025],
    ]
    .concat();
    let chunked = [
        &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n401\r\n"[..],
        &[b'x'; 1_025],
        b"\r\n0\r\n\r\n",
    ]
    .concat();
    for reply in [declared, chunked] {
        assert!(matches!(
            body_of(&[&reply]).await,
            Err(ClientError::ResponseTooLarge)
        ));
    }
}

#[tokio::test]
async fn the_deadline_bounds_the_head_and_every_body_read() {
    let started = std::time::Instant::now();
    let withheld = serve(Vec::new(), Duration::from_secs(2)).await;
    assert!(matches!(
        get(withheld, Duration::from_millis(100)).await,
        Err(ClientError::Timeout)
    ));
    let stalled = serve(
        vec![b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc".to_vec()],
        Duration::from_secs(2),
    )
    .await;
    let mut response = get(stalled, Duration::from_millis(150))
        .await
        .expect("head before the deadline");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.content_length(), Some(10));
    assert_eq!(
        response.chunk().await.expect("first bytes"),
        Some(b"abc".to_vec())
    );
    assert!(matches!(response.chunk().await, Err(ClientError::Timeout)));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[tokio::test]
async fn an_abandoned_body_read_loses_no_bytes() {
    let endpoint = serve_paced(
        vec![
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nab".to_vec(),
            b"c\r\n0\r\n\r\n".to_vec(),
        ],
        Duration::from_millis(150),
        Duration::ZERO,
    )
    .await;
    let mut response = get(endpoint, Duration::from_secs(2)).await.expect("head");
    assert_eq!(response.chunk().await.expect("first"), Some(b"ab".to_vec()));
    assert!(
        tokio::time::timeout(Duration::from_millis(20), response.chunk())
            .await
            .is_err()
    );
    assert_eq!(response.chunk().await.expect("rest"), Some(b"c".to_vec()));
    assert_eq!(response.chunk().await.expect("end"), None);
}

#[test]
fn heads_keep_the_first_content_type_and_let_chunks_override_length() {
    let head = parse_head(
        b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\
          content-type: application/json\r\nContent-Length: 4\r\nTransfer-Encoding: Chunked",
    )
    .expect("head");
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(
        head.content_type.as_deref(),
        Some(&b"text/event-stream; charset=utf-8"[..])
    );
    assert_eq!(head.content_length, None);
    assert_eq!(head.framing, Framing::ChunkSize);
    let head = parse_head(b"HTTP/1.1 503 Service Unavailable").expect("bare head");
    assert_eq!(head.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(head.framing, Framing::UntilClose);
    assert_eq!(StatusCode::UNAUTHORIZED.to_string(), "401");
}
