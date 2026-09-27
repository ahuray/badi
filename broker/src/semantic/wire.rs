//! The owned llama.cpp runtime's loopback HTTP/1.1 exchange and the framing
//! of its native replies.
//!
//! Each request opens its own connection to the loopback runtime, asks it to
//! close afterwards, and is never pooled or retried. Dropping a pending
//! exchange or its [`Response`] closes the connection at once, which is how
//! the runtime learns that a request was abandoned. Response heads, chunk
//! metadata and read buffers are bounded here, and every read ends at the
//! request's deadline. The body and event readers enforce a byte limit
//! before they keep any response data.

use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::time::Duration;

use serde::Deserialize;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout_at};

use super::client::ClientError;

/// Largest accepted response head. The runtime's are a few hundred bytes.
const MAX_HEAD_BYTES: usize = 8 * 1_024;
/// Largest accepted chunk-size line, extensions included.
const MAX_CHUNK_LINE_BYTES: usize = 128;
/// Spare buffer capacity offered to each socket read.
const READ_BYTES: usize = 8 * 1_024;

/// The status code of a runtime reply.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct StatusCode(u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const SERVICE_UNAVAILABLE: Self = Self(503);

    #[must_use]
    pub const fn as_u16(self) -> u16 {
        self.0
    }
}

impl fmt::Display for StatusCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Method {
    Get,
    Post,
}

/// One request to the runtime. The client validates `path` (one lowercase
/// segment) and `authorization` (a header-safe bearer value).
#[derive(Clone, Copy)]
pub(crate) struct Request<'a> {
    pub(crate) method: Method,
    pub(crate) path: &'a str,
    pub(crate) authorization: &'a str,
    /// A JSON body, sent with its exact length.
    pub(crate) body: Option<&'a [u8]>,
}

impl Request<'_> {
    /// The whole request, with lowercase header names.
    fn encode(&self, endpoint: SocketAddr) -> Vec<u8> {
        let method = match self.method {
            Method::Get => "GET",
            Method::Post => "POST",
        };
        let body = self.body.unwrap_or_default();
        let body_headers = self.body.map_or_else(String::new, |body| {
            format!(
                "content-type: application/json\r\ncontent-length: {}\r\n",
                body.len()
            )
        });
        let mut bytes = format!(
            "{method} {} HTTP/1.1\r\nhost: {endpoint}\r\nauthorization: {}\r\n{body_headers}connection: close\r\n\r\n",
            self.path, self.authorization
        )
        .into_bytes();
        bytes.extend_from_slice(body);
        bytes
    }
}

/// Sends `request` to the runtime at the loopback `endpoint` on a new
/// connection and reads the reply's head. `connect_timeout` bounds the
/// connection; `deadline` bounds the whole exchange, body included.
pub(crate) async fn exchange(
    endpoint: SocketAddr,
    request: Request<'_>,
    connect_timeout: Duration,
    deadline: Instant,
) -> Result<Response, ClientError> {
    if !endpoint.ip().is_loopback() {
        return Err(ClientError::InvalidConfig("endpoint"));
    }
    let connect_deadline = deadline.min(Instant::now() + connect_timeout);
    let stream = timeout_at(connect_deadline, TcpStream::connect(endpoint))
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(ClientError::Transport)?;
    stream.set_nodelay(true).map_err(ClientError::Transport)?;
    let mut connection = Connection {
        stream,
        buffered: Vec::new(),
        deadline,
    };
    // One write for the head and body. The request side is never shut down:
    // the runtime may read end of input as an abandoned request.
    connection.write_all(&request.encode(endpoint)).await?;
    connection.read_head().await
}

/// A runtime reply whose head has been read. [`Response::chunk`] reads the
/// body by the request's deadline; dropping the reply closes its connection.
pub struct Response {
    status: StatusCode,
    content_type: Option<Vec<u8>>,
    content_length: Option<u64>,
    framing: Framing,
    connection: Connection,
}

impl Response {
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// The first `Content-Type` value, if it is visible ASCII.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        let value = std::str::from_utf8(self.content_type.as_deref()?).ok()?;
        value
            .bytes()
            .all(|byte| byte == b'\t' || (b' '..=b'~').contains(&byte))
            .then_some(value)
    }

    /// The declared length of a length-delimited body.
    #[must_use]
    pub const fn content_length(&self) -> Option<u64> {
        self.content_length
    }

    /// The next decoded piece of the body, or `None` after its end. A reply
    /// that breaks its framing or ends early is a transport failure. It is
    /// cancel-safe: dropping the future mid-read loses no body bytes.
    pub async fn chunk(&mut self) -> Result<Option<Vec<u8>>, ClientError> {
        loop {
            match self.framing {
                Framing::Length(0) | Framing::Complete => return Ok(None),
                Framing::Length(remaining) => {
                    let (piece, remaining) = self.connection.take_available(remaining).await?;
                    self.framing = Framing::Length(remaining);
                    return Ok(Some(piece));
                }
                Framing::UntilClose => {
                    if self.connection.buffered.is_empty() && self.connection.read().await? == 0 {
                        self.framing = Framing::Complete;
                        return Ok(None);
                    }
                    return Ok(Some(std::mem::take(&mut self.connection.buffered)));
                }
                Framing::ChunkSize => {
                    let line = self.connection.line(MAX_CHUNK_LINE_BYTES).await?;
                    self.framing = match parse_chunk_size(&line) {
                        Some(0) => Framing::Trailers,
                        Some(size) => Framing::ChunkData(size),
                        None => return Err(malformed("invalid chunk size")),
                    };
                }
                Framing::ChunkData(remaining) => {
                    let (piece, remaining) = self.connection.take_available(remaining).await?;
                    self.framing = if remaining == 0 {
                        Framing::ChunkEnd
                    } else {
                        Framing::ChunkData(remaining)
                    };
                    return Ok(Some(piece));
                }
                Framing::ChunkEnd => {
                    // A zero limit accepts only the empty line after the data.
                    self.connection.line(0).await?;
                    self.framing = Framing::ChunkSize;
                }
                Framing::Trailers => {
                    // Trailer fields are discarded one bounded line at a time.
                    if self.connection.line(MAX_HEAD_BYTES).await?.is_empty() {
                        self.framing = Framing::Complete;
                    }
                }
            }
        }
    }
}

impl fmt::Debug for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Response")
            .field("status", &self.status)
            .field("content_type", &self.content_type())
            .field("content_length", &self.content_length)
            .finish_non_exhaustive()
    }
}

/// How the rest of a body is delimited.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Framing {
    /// This many bytes remain.
    Length(u64),
    /// Everything until the runtime closes the connection.
    UntilClose,
    /// Chunked: a chunk-size line comes next.
    ChunkSize,
    /// Chunked: this many bytes of the current chunk remain.
    ChunkData(u64),
    /// Chunked: the line break after a chunk's data comes next.
    ChunkEnd,
    /// Chunked: trailer fields or the final blank line come next.
    Trailers,
    /// The body has ended.
    Complete,
}

/// One connection and the bytes read from it but not yet consumed.
struct Connection {
    stream: TcpStream,
    buffered: Vec<u8>,
    deadline: Instant,
}

impl Connection {
    async fn write_all(&mut self, bytes: &[u8]) -> Result<(), ClientError> {
        timeout_at(self.deadline, self.stream.write_all(bytes))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Transport)
    }

    /// Appends the next bytes received; `0` is the end of the stream.
    async fn read(&mut self) -> Result<usize, ClientError> {
        self.buffered.reserve(READ_BYTES);
        timeout_at(self.deadline, self.stream.read_buf(&mut self.buffered))
            .await
            .map_err(|_| ClientError::Timeout)?
            .map_err(ClientError::Transport)
    }

    /// [`Connection::read`] where the reply cannot end yet.
    async fn read_more(&mut self) -> Result<(), ClientError> {
        if self.read().await? == 0 {
            Err(ClientError::Transport(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "runtime closed the connection before its reply ended",
            )))
        } else {
            Ok(())
        }
    }

    /// Up to `remaining` bytes, reading only when none are buffered, and how
    /// many of `remaining` are left after them.
    async fn take_available(&mut self, remaining: u64) -> Result<(Vec<u8>, u64), ClientError> {
        if self.buffered.is_empty() {
            self.read_more().await?;
        }
        let count = usize::try_from(remaining).map_or(self.buffered.len(), |remaining| {
            remaining.min(self.buffered.len())
        });
        let rest = self.buffered.split_off(count);
        let piece = std::mem::replace(&mut self.buffered, rest);
        Ok((piece, remaining - count as u64))
    }

    /// Consumes the next CRLF-terminated line of at most `limit` bytes and
    /// returns it without the CRLF.
    async fn line(&mut self, limit: usize) -> Result<Vec<u8>, ClientError> {
        loop {
            if let Some(end) = find(&self.buffered, b"\r\n") {
                if end > limit {
                    return Err(malformed("line too long"));
                }
                let line = self.buffered[..end].to_vec();
                self.buffered.drain(..end + 2);
                return Ok(line);
            }
            if self.buffered.len() >= limit.saturating_add(2) {
                return Err(malformed("line too long"));
            }
            self.read_more().await?;
        }
    }

    async fn read_head(mut self) -> Result<Response, ClientError> {
        let end = loop {
            if let Some(end) = find(&self.buffered, b"\r\n\r\n") {
                break end;
            }
            if self.buffered.len() >= MAX_HEAD_BYTES {
                return Err(malformed("response head too large"));
            }
            self.read_more().await?;
        };
        if end + 4 > MAX_HEAD_BYTES {
            return Err(malformed("response head too large"));
        }
        let head = parse_head(&self.buffered[..end])?;
        self.buffered.drain(..end + 4);
        Ok(Response {
            status: head.status,
            content_type: head.content_type,
            content_length: head.content_length,
            framing: head.framing,
            connection: self,
        })
    }
}

struct Head {
    status: StatusCode,
    content_type: Option<Vec<u8>>,
    content_length: Option<u64>,
    framing: Framing,
}

/// Parses a response head without its final blank line.
fn parse_head(head: &[u8]) -> Result<Head, ClientError> {
    let mut lines = head
        .split(|&byte| byte == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line));
    let status = lines
        .next()
        .and_then(parse_status_line)
        .ok_or_else(|| malformed("invalid status line"))?;
    let mut content_type = None;
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        let colon = line
            .iter()
            .position(|&byte| byte == b':')
            .ok_or_else(|| malformed("invalid header line"))?;
        let (name, value) = (&line[..colon], line[colon + 1..].trim_ascii());
        if name.is_empty() || !name.iter().copied().all(is_token_byte) {
            return Err(malformed("invalid header name"));
        }
        if name.eq_ignore_ascii_case(b"content-type") {
            content_type.get_or_insert_with(|| value.to_vec());
        } else if name.eq_ignore_ascii_case(b"content-length") {
            let length = parse_decimal(value).ok_or_else(|| malformed("invalid content length"))?;
            if content_length
                .replace(length)
                .is_some_and(|previous| previous != length)
            {
                return Err(malformed("conflicting content lengths"));
            }
        } else if name.eq_ignore_ascii_case(b"transfer-encoding") {
            // The request accepts no other coding, so only one plain
            // `chunked` is valid.
            if chunked || !value.eq_ignore_ascii_case(b"chunked") {
                return Err(malformed("unsupported transfer coding"));
            }
            chunked = true;
        }
    }
    // Chunked coding overrides a declared length.
    let content_length = content_length.filter(|_| !chunked);
    let framing = if matches!(status.0, 100..=199 | 204 | 304) {
        Framing::Length(0)
    } else if chunked {
        Framing::ChunkSize
    } else {
        content_length.map_or(Framing::UntilClose, Framing::Length)
    };
    Ok(Head {
        status,
        content_type,
        content_length,
        framing,
    })
}

fn parse_status_line(line: &[u8]) -> Option<StatusCode> {
    let rest = line
        .strip_prefix(b"HTTP/1.1 ")
        .or_else(|| line.strip_prefix(b"HTTP/1.0 "))?;
    let (code, reason) = rest.split_at_checked(3)?;
    if !code.iter().all(u8::is_ascii_digit) || reason.first().is_some_and(|&byte| byte != b' ') {
        return None;
    }
    let code = code
        .iter()
        .fold(0_u16, |code, digit| code * 10 + u16::from(digit - b'0'));
    (code >= 100).then_some(StatusCode(code))
}

fn parse_decimal(value: &[u8]) -> Option<u64> {
    if value.is_empty() {
        return None;
    }
    value.iter().try_fold(0_u64, |total, &byte| {
        if byte.is_ascii_digit() {
            total.checked_mul(10)?.checked_add(u64::from(byte - b'0'))
        } else {
            None
        }
    })
}

/// A chunk size in hexadecimal, ignoring any chunk extensions.
fn parse_chunk_size(line: &[u8]) -> Option<u64> {
    let digits = line
        .iter()
        .position(|byte| !byte.is_ascii_hexdigit())
        .unwrap_or(line.len());
    let (digits, rest) = line.split_at(digits);
    let rest = rest.trim_ascii_start();
    if digits.is_empty() || digits.len() > 16 || rest.first().is_some_and(|&byte| byte != b';') {
        return None;
    }
    u64::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
}

/// An RFC 9110 `tchar`.
const fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// A reply that breaks HTTP/1.1 framing fails like a broken connection.
fn malformed(reason: &'static str) -> ClientError {
    ClientError::Transport(io::Error::new(io::ErrorKind::InvalidData, reason))
}

/// One event of the native streaming completion endpoint. Only the terminal
/// event (`stop: true`) may carry the stop fields.
#[derive(Debug, Deserialize)]
pub struct NativeStreamChunk {
    pub index: u32,
    #[serde(default)]
    pub content: String,
    pub stop: bool,
    #[serde(default)]
    pub truncated: Option<bool>,
    #[serde(default)]
    pub stop_type: Option<String>,
    #[serde(default)]
    pub stopped_limit: Option<bool>,
    #[serde(default)]
    pub stopped_word: Option<bool>,
    #[serde(default)]
    pub stopped_eos: Option<bool>,
    #[serde(default)]
    pub stopping_word: Option<String>,
}

/// The tokenize endpoint's reply.
#[derive(Debug, Deserialize)]
pub struct TokenizeResponse {
    pub tokens: Vec<u32>,
}

/// The end of the first complete event in `buffer` and the bytes it
/// consumes with its blank-line separator, if one is complete.
#[must_use]
pub fn next_event_boundary(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = find(buffer, b"\n\n").map(|index| (index, index + 2));
    let crlf = find(buffer, b"\r\n\r\n").map(|index| (index, index + 4));
    match (lf, crlf) {
        (Some(left), Some(right)) => Some(if left.0 <= right.0 { left } else { right }),
        (Some(boundary), None) | (None, Some(boundary)) => Some(boundary),
        (None, None) => None,
    }
}

/// The joined `data:` lines of one event, or `None` for an event without data.
pub fn event_data(event: &[u8]) -> Result<Option<String>, ClientError> {
    let event = std::str::from_utf8(event).map_err(|_| ClientError::MalformedStream)?;
    let mut lines = Vec::new();
    for line in event.lines() {
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        if let Some(data) = line.strip_prefix("data:") {
            lines.push(data.strip_prefix(' ').unwrap_or(data));
        }
    }
    if lines.is_empty() {
        Ok(None)
    } else {
        Ok(Some(lines.join("\n")))
    }
}

/// Fails unless the reply's media type is `expected`, ignoring parameters.
pub fn ensure_content_type(response: &Response, expected: &str) -> Result<(), ClientError> {
    let matches = response
        .content_type()
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case(expected));
    if matches {
        Ok(())
    } else {
        Err(ClientError::UnexpectedContentType)
    }
}

/// Reads the whole body, failing as soon as it would exceed `limit` bytes.
pub async fn read_bounded_body(
    mut response: Response,
    limit: usize,
) -> Result<Vec<u8>, ClientError> {
    let limit_u64 = u64::try_from(limit).unwrap_or(u64::MAX);
    if response
        .content_length()
        .is_some_and(|length| length > limit_u64)
    {
        return Err(ClientError::ResponseTooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ClientError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
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
            post.encode("127.0.0.1:8080".parse().expect("endpoint")),
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
            get.encode("[::1]:9".parse().expect("endpoint")),
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
}
