//! Framing of the owned llama.cpp runtime's native HTTP replies: bounded
//! bodies, server-sent events and content types. Every reader here enforces
//! a byte limit before it keeps any response data.

use reqwest::header::{CONTENT_TYPE, HeaderMap};
pub use reqwest::{Response, StatusCode};
use serde::Deserialize;

use super::client::ClientError;

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
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|index| (index, index + 2));
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| (index, index + 4));
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

pub fn ensure_content_type(headers: &HeaderMap, expected: &str) -> Result<(), ClientError> {
    let matches = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
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
    while let Some(chunk) = response.chunk().await.map_err(transport_error)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ClientError::ResponseTooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Classifies a transport failure, keeping timeouts distinct.
#[must_use]
pub fn transport_error(error: reqwest::Error) -> ClientError {
    if error.is_timeout() {
        ClientError::Timeout
    } else {
        ClientError::Transport(error)
    }
}
