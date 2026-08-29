use std::path::{Path, PathBuf};

use omatype_broker::ipc::{
    default_socket_path, read_envelope, verify_peer_uid, verify_socket_metadata, write_envelope,
};
use omatype_broker::metrics::MetricsSnapshot;
use omatype_broker::protocol::{
    ActiveLocator, AdapterDescriptor, AdapterKind, Capability, ControlAction,
    GlobalControlRequestPayload, HealthStatusPayload, HelloPayload, MessageType, PROTOCOL_VERSION,
    ProviderKind, SessionControlRequestPayload, WireEnvelope,
};
use serde::Serialize;
use tokio::net::UnixStream;

#[tokio::main]
async fn main() {
    let result = run().await;
    if let Err(error) = result {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), CliError> {
    let parsed = parse_arguments(std::env::args().skip(1))?;
    let ParsedCommand::Execute {
        socket_path,
        command,
    } = parsed
    else {
        print!("{CLI_USAGE}");
        return Ok(());
    };
    let mut stream = connect(&socket_path).await?;
    match command {
        Command::Status => {
            let status = request_health(&mut stream).await?;
            println!("{}", serde_json::to_string(&redact_health_status(&status))?);
        }
        Command::Global(action) => {
            let mut request = WireEnvelope::global(
                MessageType::ControlRequest,
                0,
                &GlobalControlRequestPayload { action },
            )?;
            request.id = Some(new_request_id());
            write_envelope(&mut stream, &request).await?;
            print_next_response(&mut stream).await?;
        }
        Command::Session(action) => {
            let status = request_health(&mut stream).await?;
            let active = status.active.ok_or(CliError::NoActiveSession)?;
            let request = addressed_control(action, active)?;
            write_envelope(&mut stream, &request).await?;
            print_next_response(&mut stream).await?;
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
struct RedactedActiveStatus {
    present: bool,
    has_suggestion: bool,
}

#[derive(Debug, Eq, PartialEq, Serialize)]
struct RedactedHealthStatus<'a> {
    provider: ProviderKind,
    paused: bool,
    sessions: u64,
    socket_mode: &'a str,
    max_frame_bytes: usize,
    metrics: MetricsSnapshot,
    active: RedactedActiveStatus,
}

fn redact_health_status(status: &HealthStatusPayload) -> RedactedHealthStatus<'_> {
    RedactedHealthStatus {
        provider: status.provider,
        paused: status.paused,
        sessions: status.sessions,
        socket_mode: &status.socket_mode,
        max_frame_bytes: status.max_frame_bytes,
        metrics: status.metrics,
        active: RedactedActiveStatus {
            present: status.active.is_some(),
            has_suggestion: status
                .active
                .as_ref()
                .is_some_and(|active| active.suggestion_id.is_some()),
        },
    }
}

async fn connect(path: &Path) -> Result<UnixStream, CliError> {
    verify_socket_metadata(path)?;
    let mut stream = UnixStream::connect(path).await?;
    verify_peer_uid(&stream)?;
    let mut hello = WireEnvelope::global(
        MessageType::Hello,
        0,
        &HelloPayload {
            min_v: PROTOCOL_VERSION,
            max_v: PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Cli,
                name: "omatypectl".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            },
            capabilities: vec![Capability::Control, Capability::Health],
        },
    )?;
    hello.id = Some(new_request_id());
    write_envelope(&mut stream, &hello).await?;
    let acknowledgment = read_envelope(&mut stream)
        .await?
        .ok_or(CliError::ConnectionClosed)?;
    if acknowledgment.message_type != MessageType::HelloAck {
        return Err(CliError::Handshake);
    }
    Ok(stream)
}

async fn request_health(stream: &mut UnixStream) -> Result<HealthStatusPayload, CliError> {
    let mut request = WireEnvelope::global(MessageType::HealthRequest, 0, &serde_json::json!({}))?;
    request.id = Some(new_request_id());
    write_envelope(stream, &request).await?;
    let response = read_envelope(stream)
        .await?
        .ok_or(CliError::ConnectionClosed)?;
    if response.message_type != MessageType::HealthStatus {
        return Err(CliError::UnexpectedResponse);
    }
    Ok(response.decode_payload()?)
}

fn addressed_control(
    action: ControlAction,
    active: ActiveLocator,
) -> Result<WireEnvelope, CliError> {
    let needs_suggestion = matches!(
        action,
        ControlAction::AcceptWord | ControlAction::AcceptAll | ControlAction::Dismiss
    );
    if needs_suggestion && active.suggestion_id.is_none() {
        return Err(CliError::NoSuggestion);
    }
    let mut envelope = WireEnvelope::session(
        MessageType::ControlRequest,
        omatype_broker::protocol::Coordinates {
            session_id: active.session_id,
            focus_epoch: active.focus_epoch,
            revision: active.revision,
        },
        0,
        &SessionControlRequestPayload {
            action,
            fingerprint: active.fingerprint,
            suggestion_id: active.suggestion_id,
        },
    )?;
    envelope.id = Some(new_request_id());
    Ok(envelope)
}

async fn print_next_response(stream: &mut UnixStream) -> Result<(), CliError> {
    let response = read_envelope(stream)
        .await?
        .ok_or(CliError::ConnectionClosed)?;
    println!("{}", serde_json::to_string(&response)?);
    if response.message_type == MessageType::Error {
        Err(CliError::Rejected)
    } else {
        Ok(())
    }
}

fn new_request_id() -> String {
    format!("ctl:{}", uuid::Uuid::new_v4())
}

const CLI_USAGE: &str = "Usage: omatypectl [--socket ABSOLUTE] COMMAND\n\
Commands:\n  status [--json]  Show content-free broker status as JSON\n  request          Request a suggestion for the sole active session\n  accept-word      Accept the authorized first word-part\n  accept-all       Accept the authorized full suggestion\n  dismiss          Dismiss the current suggestion\n  pause [MODE]     MODE is on, off, or toggle (default)\n\
Options:\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/omatype/broker.sock\n  -h, --help         Show this help\n";

fn parse_arguments<I>(arguments: I) -> Result<ParsedCommand, CliError>
where
    I: IntoIterator<Item = String>,
{
    let mut arguments = arguments.into_iter().peekable();
    if arguments
        .peek()
        .is_some_and(|argument| argument == "--help" || argument == "-h")
    {
        let _ = arguments.next();
        return if arguments.next().is_none() {
            Ok(ParsedCommand::Help)
        } else {
            Err(CliError::Arguments)
        };
    }

    let explicit_socket = if arguments
        .peek()
        .is_some_and(|argument| argument == "--socket")
    {
        let _ = arguments.next();
        let value = arguments.next().ok_or(CliError::Arguments)?;
        let path = PathBuf::from(value);
        if !path.is_absolute() {
            return Err(CliError::Arguments);
        }
        Some(path)
    } else {
        None
    };

    let command = match arguments.next().as_deref() {
        Some("--help" | "-h") => {
            if arguments.next().is_some() {
                return Err(CliError::Arguments);
            }
            return Ok(ParsedCommand::Help);
        }
        Some("status") => {
            if arguments
                .next()
                .as_deref()
                .is_some_and(|value| value != "--json")
            {
                return Err(CliError::Arguments);
            }
            Command::Status
        }
        Some("request") => Command::Session(ControlAction::Request),
        Some("accept-word") => Command::Session(ControlAction::AcceptWord),
        Some("accept-all") => Command::Session(ControlAction::AcceptAll),
        Some("dismiss") => Command::Session(ControlAction::Dismiss),
        Some("pause") => match arguments.next().as_deref() {
            None | Some("toggle") => Command::Global(ControlAction::PauseToggle),
            Some("on") => Command::Global(ControlAction::Pause),
            Some("off") => Command::Global(ControlAction::Resume),
            Some(_) => return Err(CliError::Arguments),
        },
        _ => return Err(CliError::Arguments),
    };
    if arguments.next().is_some() {
        return Err(CliError::Arguments);
    }
    let socket_path = explicit_socket.map_or_else(default_socket_path, Ok)?;
    Ok(ParsedCommand::Execute {
        socket_path,
        command,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Global(ControlAction),
    Session(ControlAction),
    Status,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParsedCommand {
    Execute {
        socket_path: PathBuf,
        command: Command,
    },
    Help,
}

#[derive(Debug, thiserror::Error)]
enum CliError {
    #[error("arguments")]
    Arguments,
    #[error("connection_closed")]
    ConnectionClosed,
    #[error("frame")]
    Frame(#[from] omatype_broker::ipc::FrameError),
    #[error("handshake")]
    Handshake,
    #[error("io")]
    Io(#[from] std::io::Error),
    #[error("no_active_session")]
    NoActiveSession,
    #[error("no_suggestion")]
    NoSuggestion,
    #[error("protocol")]
    Protocol(#[from] omatype_broker::protocol::ProtocolError),
    #[error("rejected")]
    Rejected,
    #[error("serde")]
    Serde(#[from] serde_json::Error),
    #[error("unexpected_response")]
    UnexpectedResponse,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{CliError, Command, ParsedCommand, parse_arguments, redact_health_status};
    use omatype_broker::metrics::MetricsSnapshot;
    use omatype_broker::protocol::{
        ActiveLocator, ControlAction, HealthStatusPayload, ProviderKind, SessionId,
    };
    use serde_json::json;

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_help_without_requiring_a_runtime_directory() {
        assert_eq!(
            parse_arguments(arguments(&["--help"])).expect("help"),
            ParsedCommand::Help
        );
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "-h"]))
                .expect("help after socket"),
            ParsedCommand::Help
        );
    }

    #[test]
    fn parses_commands_with_absolute_socket() {
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "accept-word",]))
                .expect("accept word"),
            ParsedCommand::Execute {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Session(ControlAction::AcceptWord),
            }
        );
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock", "pause", "off"]))
                .expect("pause off"),
            ParsedCommand::Execute {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                command: Command::Global(ControlAction::Resume),
            }
        );
    }

    #[test]
    fn rejects_relative_socket_and_extra_arguments() {
        assert!(matches!(
            parse_arguments(arguments(&["--socket", "broker.sock", "status"])),
            Err(CliError::Arguments)
        ));
        assert!(matches!(
            parse_arguments(arguments(&["--help", "extra"])),
            Err(CliError::Arguments)
        ));
    }

    #[test]
    fn status_json_redacts_all_active_locator_tokens() {
        let session_id = SessionId::new();
        let status = HealthStatusPayload {
            provider: ProviderKind::PhraseV1,
            paused: false,
            sessions: 1,
            socket_mode: "0600".to_owned(),
            max_frame_bytes: 65_536,
            metrics: MetricsSnapshot::default(),
            active: Some(ActiveLocator {
                session_id,
                focus_epoch: 7,
                revision: 11,
                fingerprint: "fingerprint_000000000001".to_owned(),
                suggestion_id: Some("s:secret".to_owned()),
            }),
        };

        let value = serde_json::to_value(redact_health_status(&status)).expect("redacted JSON");
        assert_eq!(
            value["active"],
            json!({ "present": true, "has_suggestion": true })
        );
        let encoded = serde_json::to_string(&value).expect("encoded redacted JSON");
        for secret in [
            &session_id.to_string(),
            "fingerprint_000000000001",
            "s:secret",
        ] {
            assert!(!encoded.contains(secret));
        }

        let mut inactive = status;
        inactive.active = None;
        assert_eq!(
            serde_json::to_value(redact_health_status(&inactive)).expect("inactive JSON")["active"],
            json!({ "present": false, "has_suggestion": false })
        );
    }
}
