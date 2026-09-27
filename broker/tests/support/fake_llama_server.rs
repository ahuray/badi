//! A fake llama-server for owned-runtime tests. The runtime launches the test
//! executable itself with [`ARGUMENT`] and the environment it gives the real
//! server; `BADI_FIXTURE_BEHAVIOR` selects a startup failure. The server
//! enforces the bearer token, answers the health and authorization probes,
//! streams one fixed continuation, and closes any other completion, such as
//! the non-streaming warm-up, without a reply.

use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::process::ExitCode;

use serde_json::{Value, json};

/// Fixed by `LlamaCppLaunch::for_fixture`.
pub const ARGUMENT: &str = "__fixture-backend";
const MAX_REQUEST_BYTES: usize = 64 * 1_024;

pub fn run() -> ExitCode {
    let behavior = std::env::var("BADI_FIXTURE_BEHAVIOR").unwrap_or_default();
    match behavior.as_str() {
        "early_exit" => return ExitCode::SUCCESS,
        "no_bind" => loop {
            std::thread::park();
        },
        _ => {}
    }
    let Some((port, token)) = launch_contract() else {
        return ExitCode::FAILURE;
    };
    let Ok(listener) = TcpListener::bind((Ipv4Addr::LOCALHOST, port)) else {
        return ExitCode::FAILURE;
    };
    let health: &'static [u8] = if behavior == "malformed_health" {
        br#"{"unexpected":true}"#
    } else {
        br#"{"status":"ok"}"#
    };
    for stream in listener.incoming().flatten() {
        let token = token.clone();
        std::thread::spawn(move || respond(stream, &token, health));
    }
    ExitCode::FAILURE
}

/// The owned-runtime launch environment; a mismatch fails startup like a
/// misconfigured server would.
fn launch_contract() -> Option<(u16, String)> {
    let variable = |name| std::env::var(name).ok();
    let port = variable("LLAMA_ARG_PORT")?.parse::<u16>().ok()?;
    let token = variable("LLAMA_API_KEY")?;
    let threads = variable("LLAMA_ARG_THREADS")?;
    let expected = [
        ("LLAMA_ARG_HOST", "127.0.0.1"),
        ("LLAMA_ARG_CTX_SIZE", "512"),
        ("LLAMA_ARG_N_PARALLEL", "1"),
        ("LLAMA_ARG_N_GPU_LAYERS", "0"),
        ("LLAMA_ARG_UI", "0"),
        ("LLAMA_ARG_OFFLINE", "1"),
        ("LLAMA_ARG_CACHE_PROMPT", "0"),
        ("LLAMA_ARG_THREADS_BATCH", threads.as_str()),
    ];
    let valid = port != 0
        && !token.is_empty()
        && variable("LLAMA_ARG_MODEL").is_some_and(|model| model.starts_with('/'))
        && threads.parse::<usize>().is_ok_and(|threads| threads > 0)
        && expected
            .iter()
            .all(|(name, value)| variable(name).as_deref() == Some(value));
    valid.then_some((port, token))
}

struct Request {
    method: String,
    path: String,
    authorized: bool,
    body: Value,
}

fn respond(mut stream: TcpStream, token: &str, health: &[u8]) -> io::Result<()> {
    let request = read_request(&mut stream, token)?;
    let public = request.method == "GET" && request.path == "/health";
    if !request.authorized && !public {
        return write_json(
            &mut stream,
            "401 Unauthorized",
            br#"{"error":"unauthorized"}"#,
        );
    }
    match (request.method.as_str(), request.path.as_str()) {
        ("GET", "/health") => write_json(&mut stream, "200 OK", health),
        ("POST", "/tokenize")
            if request.body
                == json!({"content": "badi-owned-runtime-challenge", "add_special": false}) =>
        {
            write_json(&mut stream, "200 OK", br#"{"tokens":[42]}"#)
        }
        ("POST", "/completion") if request.body["stream"] == json!(true) => {
            stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )?;
            for event in [
                json!({"index": 0, "content": " for", "stop": false}),
                json!({"index": 0, "content": " your time", "stop": false}),
                json!({"index": 0, "content": "", "stop": true, "stop_type": "word", "stopping_word": "."}),
            ] {
                write!(stream, "data: {event}\n\n")?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn read_request(stream: &mut TcpStream, token: &str) -> io::Result<Request> {
    let invalid = || io::Error::from(io::ErrorKind::InvalidData);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4_096];
    let header_end = loop {
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
        let read = stream.read(&mut chunk)?;
        if read == 0 || bytes.len() > MAX_REQUEST_BYTES {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk[..read]);
    };
    let head = std::str::from_utf8(&bytes[..header_end]).map_err(|_| invalid())?;
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next().unwrap_or_default().split(' ');
    let method = request_line.next().unwrap_or_default().to_owned();
    let path = request_line.next().unwrap_or_default().to_owned();
    let mut content_length = 0;
    let mut authorized = false;
    for (name, value) in lines.filter_map(|line| line.split_once(':')) {
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.trim().parse().map_err(|_| invalid())?;
        } else if name.eq_ignore_ascii_case("authorization") {
            authorized = value.trim() == format!("Bearer {token}");
        }
    }
    if content_length > MAX_REQUEST_BYTES {
        return Err(invalid());
    }
    while bytes.len() < header_end + content_length {
        let read = stream.read(&mut chunk)?;
        if read == 0 {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = &bytes[header_end..header_end + content_length];
    Ok(Request {
        method,
        path,
        authorized,
        body: serde_json::from_slice(body).unwrap_or(Value::Null),
    })
}

fn write_json(stream: &mut TcpStream, status: &str, body: &[u8]) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}
