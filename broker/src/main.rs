use std::path::PathBuf;
use std::sync::Arc;

use badi_broker::engine::{Broker, BrokerConfig};
use badi_broker::ipc::default_socket_path;
use badi_broker::provider::DeterministicPhraseProvider;
use badi_broker::{ControlPlane, server};

#[tokio::main]
async fn main() {
    let result = async {
        let command = parse_arguments(std::env::args_os().skip(1))?;
        let BrokerCommand::Run(socket_path) = command else {
            print!("{BROKER_USAGE}");
            return Ok(());
        };
        let provider = Arc::new(DeterministicPhraseProvider::default());
        let control_plane =
            Arc::new(ControlPlane::open_from_environment().map_err(|_| ExitError::ControlPlane)?);
        let broker = Broker::with_control_plane(provider, BrokerConfig::default(), control_plane)
            .map_err(|_| ExitError::ControlPlane)?;
        server::run(&socket_path, broker)
            .await
            .map_err(|_| ExitError::Server)
    }
    .await;

    if let Err(error) = result {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

const BROKER_USAGE: &str = "Usage: badi-broker [--socket ABSOLUTE]\n\
Runs the local Unix-socket suggestion broker.\n\
Options:\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/badi/broker.sock\n  -h, --help         Show this help\n";

fn parse_arguments<I>(arguments: I) -> Result<BrokerCommand, ExitError>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut arguments = arguments.into_iter();
    match arguments.next() {
        None => default_socket_path()
            .map(BrokerCommand::Run)
            .map_err(|_| ExitError::SocketPath),
        Some(flag) if flag == "--help" || flag == "-h" => {
            if arguments.next().is_some() {
                Err(ExitError::Arguments)
            } else {
                Ok(BrokerCommand::Help)
            }
        }
        Some(flag) if flag == "--socket" => {
            let path = arguments.next().ok_or(ExitError::Arguments)?;
            if arguments.next().is_some() {
                return Err(ExitError::Arguments);
            }
            let path = PathBuf::from(path);
            if path.is_absolute() {
                Ok(BrokerCommand::Run(path))
            } else {
                Err(ExitError::SocketPath)
            }
        }
        Some(_) => Err(ExitError::Arguments),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BrokerCommand {
    Help,
    Run(PathBuf),
}

#[derive(Clone, Copy, Debug)]
enum ExitError {
    Arguments,
    ControlPlane,
    Server,
    SocketPath,
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arguments => formatter.write_str("arguments"),
            Self::ControlPlane => formatter.write_str("control_plane"),
            Self::Server => formatter.write_str("server"),
            Self::SocketPath => formatter.write_str("socket_path"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{BrokerCommand, ExitError, parse_arguments};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_help_and_absolute_socket() {
        assert_eq!(
            parse_arguments(arguments(&["--help"])).expect("help"),
            BrokerCommand::Help
        );
        assert_eq!(
            parse_arguments(arguments(&["--socket", "/tmp/broker.sock"])).expect("absolute socket"),
            BrokerCommand::Run(PathBuf::from("/tmp/broker.sock"))
        );
    }

    #[test]
    fn rejects_relative_socket_and_extra_arguments() {
        assert!(matches!(
            parse_arguments(arguments(&["--socket", "broker.sock"])),
            Err(ExitError::SocketPath)
        ));
        assert!(matches!(
            parse_arguments(arguments(&["--help", "extra"])),
            Err(ExitError::Arguments)
        ));
    }
}
