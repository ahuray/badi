use std::path::PathBuf;
use std::sync::Arc;

use badi_broker::engine::{Broker, BrokerConfig};
use badi_broker::ipc::default_socket_path;
use badi_broker::provider::{CompletionProvider, DeterministicPhraseProvider};
use badi_broker::{ControlPlane, server};

#[tokio::main]
async fn main() {
    let result = async {
        let command = parse_arguments(std::env::args_os().skip(1))?;
        let BrokerCommand::Run {
            socket_path,
            provider,
            model_directory,
        } = command
        else {
            print!("{BROKER_USAGE}");
            return Ok(());
        };
        let control_plane =
            Arc::new(ControlPlane::open_from_environment().map_err(|_| ExitError::ControlPlane)?);
        let provider = start_provider(provider, model_directory).await?;
        let broker = Broker::with_control_plane(provider, BrokerConfig::default(), control_plane)
            .map_err(|_| ExitError::ControlPlane)?;
        server::run(&socket_path, broker)
            .await
            .map_err(|error| match error {
                server::ServerError::ProviderExited => {
                    ExitError::Model("runtime_process_exited".to_owned())
                }
                _ => ExitError::Server,
            })
    }
    .await;

    if let Err(error) = result {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

const BROKER_USAGE: &str = "Usage: badi-broker [--socket ABSOLUTE] [--provider local|phrase] [--model-directory ABSOLUTE]\n\
Runs the local Unix-socket suggestion broker.\n\
Options:\n  --provider local|phrase  Local LLM (default) or deterministic integration fixture\n  --model-directory ABSOLUTE  Override the Badi model/runtime data directory\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/badi/broker.sock\n  -h, --help         Show this help\n";

async fn start_provider(
    kind: ProviderSelection,
    directory: Option<PathBuf>,
) -> Result<Arc<dyn CompletionProvider>, ExitError> {
    match kind {
        ProviderSelection::Phrase => Ok(Arc::new(DeterministicPhraseProvider::default())),
        ProviderSelection::Local => {
            #[cfg(feature = "local-model")]
            {
                let directory = directory
                    .map_or_else(badi_broker::writing::data_directory, Ok)
                    .map_err(|error| ExitError::Model(error.to_string()))?;
                let (runtime, model) = badi_broker::writing::activate(directory)
                    .await
                    .map_err(|error| ExitError::Model(error.to_string()))?;
                eprintln!(
                    "provider=local_model model={} quantization={}",
                    model.filename, model.quantization
                );
                Ok(Arc::new(runtime))
            }
            #[cfg(not(feature = "local-model"))]
            {
                let _ = directory;
                Err(ExitError::Model(
                    "rebuild with the local-model feature".to_owned(),
                ))
            }
        }
    }
}

fn parse_arguments<I>(arguments: I) -> Result<BrokerCommand, ExitError>
where
    I: IntoIterator<Item = std::ffi::OsString>,
{
    let mut arguments = arguments.into_iter().peekable();
    let mut socket = None;
    let mut provider = None;
    let mut model_directory = None;
    while let Some(flag) = arguments.next() {
        if flag == "--help" || flag == "-h" {
            if socket.is_some()
                || provider.is_some()
                || model_directory.is_some()
                || arguments.peek().is_some()
            {
                return Err(ExitError::Arguments);
            }
            return Ok(BrokerCommand::Help);
        }
        let value = arguments.next().ok_or(ExitError::Arguments)?;
        if flag == "--socket" && socket.is_none() {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(ExitError::SocketPath);
            }
            socket = Some(path);
        } else if flag == "--model-directory" && model_directory.is_none() {
            let path = PathBuf::from(value);
            if !path.is_absolute() {
                return Err(ExitError::Arguments);
            }
            model_directory = Some(path);
        } else if flag == "--provider" && provider.is_none() {
            provider = Some(match value.to_str() {
                Some("local") => ProviderSelection::Local,
                Some("phrase") => ProviderSelection::Phrase,
                _ => return Err(ExitError::Arguments),
            });
        } else {
            return Err(ExitError::Arguments);
        }
    }
    let provider = provider.unwrap_or(ProviderSelection::Local);
    if provider == ProviderSelection::Phrase && model_directory.is_some() {
        return Err(ExitError::Arguments);
    }
    Ok(BrokerCommand::Run {
        socket_path: socket
            .map_or_else(default_socket_path, Ok)
            .map_err(|_| ExitError::SocketPath)?,
        provider,
        model_directory,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum BrokerCommand {
    Help,
    Run {
        socket_path: PathBuf,
        provider: ProviderSelection,
        model_directory: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProviderSelection {
    Local,
    Phrase,
}

#[derive(Clone, Debug)]
enum ExitError {
    Arguments,
    ControlPlane,
    Server,
    SocketPath,
    Model(String),
}

impl std::fmt::Display for ExitError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Arguments => formatter.write_str("arguments"),
            Self::ControlPlane => formatter.write_str("control_plane"),
            Self::Server => formatter.write_str("server"),
            Self::SocketPath => formatter.write_str("socket_path"),
            Self::Model(message) => write!(formatter, "local_model: {message}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{BrokerCommand, ExitError, ProviderSelection, parse_arguments};

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
            BrokerCommand::Run {
                socket_path: PathBuf::from("/tmp/broker.sock"),
                provider: ProviderSelection::Local,
                model_directory: None
            }
        );
    }

    #[test]
    fn provider_and_asset_flags_are_explicit_and_unambiguous() {
        assert!(matches!(
            parse_arguments(arguments(&[
                "--provider",
                "phrase",
                "--socket",
                "/tmp/badi-test.sock"
            ])),
            Ok(BrokerCommand::Run {
                provider: ProviderSelection::Phrase,
                ..
            })
        ));
        assert!(matches!(
            parse_arguments(arguments(&[
                "--model-directory",
                "/tmp/badi-models",
                "--socket",
                "/tmp/badi-test.sock"
            ])),
            Ok(BrokerCommand::Run {
                provider: ProviderSelection::Local,
                model_directory: Some(_),
                ..
            })
        ));
        for values in [
            vec!["--provider", "other"],
            vec!["--model-directory", "relative"],
            vec!["--provider", "phrase", "--model-directory", "/tmp/models"],
            vec!["--provider", "local", "--provider", "phrase"],
        ] {
            assert!(parse_arguments(arguments(&values)).is_err());
        }
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
