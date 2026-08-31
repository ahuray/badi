use std::io;
use std::net::Ipv4Addr;
#[cfg(test)]
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
#[cfg(test)]
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

const MAX_HEADER_BYTES: usize = 32 * 1_024;
const MAX_REQUEST_BYTES: usize = 64 * 1_024;

#[derive(Debug, Default)]
pub struct FixtureAudit {
    connections: AtomicU64,
    request_body_bytes: AtomicU64,
}

#[cfg_attr(test, allow(dead_code))]
impl FixtureAudit {
    #[cfg(test)]
    #[must_use]
    pub fn connections(&self) -> u64 {
        self.connections.load(Ordering::SeqCst)
    }

    #[cfg(test)]
    #[must_use]
    pub fn request_body_bytes(&self) -> u64 {
        self.request_body_bytes.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub struct FixtureBackend {
    endpoint: SocketAddr,
    audit: Arc<FixtureAudit>,
    cancellation: CancellationToken,
    task: JoinHandle<Result<(), FixtureError>>,
}

#[cfg(test)]
#[allow(dead_code)]
impl FixtureBackend {
    pub async fn start(token: impl Into<String>) -> Result<Self, FixtureError> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(FixtureError::Bind)?;
        let endpoint = listener.local_addr().map_err(FixtureError::Bind)?;
        let audit = Arc::new(FixtureAudit::default());
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(serve(
            listener,
            token.into(),
            FixtureHealth::Ready,
            audit.clone(),
            cancellation.clone(),
        ));
        Ok(Self {
            endpoint,
            audit,
            cancellation,
            task,
        })
    }

    #[must_use]
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    #[must_use]
    pub fn audit(&self) -> &FixtureAudit {
        &self.audit
    }

    pub async fn shutdown(self) -> Result<(), FixtureError> {
        self.cancellation.cancel();
        self.task.await.map_err(FixtureError::Join)??;
        Ok(())
    }
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FixtureHealth {
    Ready,
    Malformed,
}

#[cfg_attr(test, allow(dead_code))]
#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture backend configuration is invalid")]
    Configuration,
    #[error("fixture backend could not bind its loopback endpoint")]
    Bind(#[source] io::Error),
    #[error("fixture backend I/O failed")]
    Io(#[source] io::Error),
    #[cfg(test)]
    #[error("fixture backend task failed")]
    Join(#[source] tokio::task::JoinError),
}

#[cfg_attr(test, allow(dead_code))]
pub async fn run_from_environment() -> Result<(), FixtureError> {
    let behavior = std::env::var("BADI_FIXTURE_BEHAVIOR").unwrap_or_default();
    if behavior == "early_exit" {
        return Ok(());
    }
    if behavior == "no_bind" {
        std::future::pending::<()>().await;
        return Ok(());
    }
    let host = std::env::var("LLAMA_ARG_HOST").map_err(|_| FixtureError::Configuration)?;
    let port = std::env::var("LLAMA_ARG_PORT")
        .map_err(|_| FixtureError::Configuration)?
        .parse::<u16>()
        .map_err(|_| FixtureError::Configuration)?;
    let token = std::env::var("LLAMA_API_KEY").map_err(|_| FixtureError::Configuration)?;
    let model = std::env::var("LLAMA_ARG_MODEL").map_err(|_| FixtureError::Configuration)?;
    let threads = std::env::var("LLAMA_ARG_THREADS").map_err(|_| FixtureError::Configuration)?;
    if host != Ipv4Addr::LOCALHOST.to_string()
        || port == 0
        || token.is_empty()
        || !std::path::Path::new(&model).is_absolute()
        || std::env::var("LLAMA_ARG_CTX_SIZE").as_deref() != Ok("512")
        || std::env::var("LLAMA_ARG_N_PARALLEL").as_deref() != Ok("1")
        || std::env::var("LLAMA_ARG_N_GPU_LAYERS").as_deref() != Ok("0")
        || std::env::var("LLAMA_ARG_UI").as_deref() != Ok("0")
        || std::env::var("LLAMA_ARG_OFFLINE").as_deref() != Ok("1")
        || std::env::var("LLAMA_ARG_CACHE_PROMPT").as_deref() != Ok("0")
        || threads.parse::<usize>().ok().is_none_or(|value| value == 0)
        || std::env::var("LLAMA_ARG_THREADS_BATCH").as_deref() != Ok(threads.as_str())
    {
        return Err(FixtureError::Configuration);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
        .await
        .map_err(FixtureError::Bind)?;
    let health = if behavior == "malformed_health" {
        FixtureHealth::Malformed
    } else {
        FixtureHealth::Ready
    };
    serve(
        listener,
        token,
        health,
        Arc::new(FixtureAudit::default()),
        CancellationToken::new(),
    )
    .await
}

async fn serve(
    listener: TcpListener,
    token: String,
    health: FixtureHealth,
    audit: Arc<FixtureAudit>,
    cancellation: CancellationToken,
) -> Result<(), FixtureError> {
    loop {
        let accepted = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Ok(()),
            accepted = listener.accept() => accepted.map_err(FixtureError::Io)?,
        };
        audit.connections.fetch_add(1, Ordering::SeqCst);
        let token = token.clone();
        let audit = audit.clone();
        tokio::spawn(async move {
            let _ = handle_connection(accepted.0, &token, health, &audit).await;
        });
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    token: &str,
    health: FixtureHealth,
    audit: &FixtureAudit,
) -> Result<(), FixtureError> {
    let request = read_request(&mut stream).await?;
    audit.request_body_bytes.fetch_add(
        u64::try_from(request.body.len()).unwrap_or(u64::MAX),
        Ordering::SeqCst,
    );
    let public_health = request.method == "GET" && request.path == "/health";
    if !public_health && request.authorization.as_deref() != Some(&format!("Bearer {token}")) {
        write_response(
            &mut stream,
            401,
            "application/json",
            br#"{"error":"unauthorized"}"#,
        )
        .await?;
        return Ok(());
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => {
            let body = match health {
                FixtureHealth::Ready => br#"{"status":"ok"}"#.as_slice(),
                FixtureHealth::Malformed => br#"{"unexpected":true}"#.as_slice(),
            };
            write_response(&mut stream, 200, "application/json", body).await?;
        }
        ("POST", "/completion") => {
            let before = extract_prompt(&request.body).ok_or(FixtureError::Configuration)?;
            write_stream(&mut stream, &before).await?;
        }
        ("POST", "/tokenize") => {
            let request: Value =
                serde_json::from_slice(&request.body).map_err(|_| FixtureError::Configuration)?;
            if request.get("content") != Some(&Value::String("badi-owned-runtime-challenge".into()))
                || request.get("add_special") != Some(&Value::Bool(false))
            {
                return Err(FixtureError::Configuration);
            }
            write_response(&mut stream, 200, "application/json", br#"{"tokens":[42]}"#).await?;
        }
        _ => {
            write_response(
                &mut stream,
                404,
                "application/json",
                br#"{"error":"not_found"}"#,
            )
            .await?;
        }
    }
    Ok(())
}

struct HttpRequest {
    method: String,
    path: String,
    authorization: Option<String>,
    body: Vec<u8>,
}

async fn read_request(stream: &mut TcpStream) -> Result<HttpRequest, FixtureError> {
    let mut bytes = Vec::new();
    let header_end = loop {
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(FixtureError::Configuration);
        }
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let mut chunk = [0_u8; 4_096];
        let read = stream.read(&mut chunk).await.map_err(FixtureError::Io)?;
        if read == 0 {
            return Err(FixtureError::Configuration);
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let headers =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| FixtureError::Configuration)?;
    let mut lines = headers.split("\r\n");
    let request_line = lines.next().ok_or(FixtureError::Configuration)?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts
        .next()
        .ok_or(FixtureError::Configuration)?
        .to_owned();
    let path = request_parts
        .next()
        .ok_or(FixtureError::Configuration)?
        .to_owned();
    let mut content_length = 0_usize;
    let mut authorization = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse()
                .map_err(|_| FixtureError::Configuration)?;
        } else if name.eq_ignore_ascii_case("authorization") {
            authorization = Some(value.trim().to_owned());
        }
    }
    if content_length > MAX_REQUEST_BYTES {
        return Err(FixtureError::Configuration);
    }
    while bytes.len().saturating_sub(header_end) < content_length {
        let mut chunk = [0_u8; 4_096];
        let read = stream.read(&mut chunk).await.map_err(FixtureError::Io)?;
        if read == 0 {
            return Err(FixtureError::Configuration);
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    Ok(HttpRequest {
        method,
        path,
        authorization,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn extract_prompt(body: &[u8]) -> Option<String> {
    let request: Value = serde_json::from_slice(body).ok()?;
    if request.get("stream") != Some(&Value::Bool(true))
        || request.get("cache_prompt") != Some(&Value::Bool(false))
        || request.get("n_predict")?.as_u64()? != 8
        || request.get("temperature")?.as_f64()? != 0.0
        || request.get("seed")?.as_u64()? != 42
        || request.get("stop")?.as_array()?.as_slice()
            != [
                Value::String(".".to_owned()),
                Value::String("\n".to_owned()),
            ]
    {
        return None;
    }
    request.get("prompt")?.as_str().map(str::to_owned)
}

async fn write_stream(stream: &mut TcpStream, before: &str) -> Result<(), FixtureError> {
    stream
        .write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
        )
        .await
        .map_err(FixtureError::Io)?;
    if before.contains("fixture:timeout") {
        tokio::time::sleep(Duration::from_secs(2)).await;
    } else {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    if before.contains("fixture:reasoning") {
        write_content(stream, "<think>hidden</think>").await?;
        finish_stream(stream, "eos", "").await?;
    } else if before.contains("fixture:arabic") {
        write_content(stream, " مرحبا").await?;
        finish_stream(stream, "eos", "").await?;
    } else if before.contains("fixture:cjk") {
        write_content(stream, " 世界").await?;
        finish_stream(stream, "eos", "").await?;
    } else if before.contains("fixture:emoji") {
        write_content(stream, " 👍").await?;
        finish_stream(stream, "eos", "").await?;
    } else if before.contains("fixture:truncated") {
        write_content(stream, " unfinished").await?;
        finish_stream(stream, "limit", "").await?;
    } else if before.contains("fixture:abstain") {
        finish_stream(stream, "eos", "").await?;
    } else {
        write_content(stream, " for").await?;
        tokio::time::sleep(Duration::from_millis(25)).await;
        write_content(stream, " your time").await?;
        finish_stream(stream, "word", ".").await?;
    }
    Ok(())
}

async fn write_content(stream: &mut TcpStream, content: &str) -> Result<(), FixtureError> {
    write_event(stream, &json!({"index":0,"content":content,"stop":false})).await
}

async fn finish_stream(
    stream: &mut TcpStream,
    stop_type: &str,
    stopping_word: &str,
) -> Result<(), FixtureError> {
    write_event(
        stream,
        &json!({"index":0,"content":"","stop":true,"truncated":false,"stop_type":stop_type,"stopping_word":stopping_word}),
    )
    .await
}

async fn write_event(stream: &mut TcpStream, value: &Value) -> Result<(), FixtureError> {
    let mut event = b"data: ".to_vec();
    event.extend_from_slice(&serde_json::to_vec(value).map_err(|_| FixtureError::Configuration)?);
    event.extend_from_slice(b"\n\n");
    stream.write_all(&event).await.map_err(FixtureError::Io)
}

async fn write_response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> Result<(), FixtureError> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        _ => "Error",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(headers.as_bytes())
        .await
        .map_err(FixtureError::Io)?;
    stream.write_all(body).await.map_err(FixtureError::Io)
}
