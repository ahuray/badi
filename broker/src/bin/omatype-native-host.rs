use std::ffi::OsString;
use std::path::PathBuf;

use omatype_broker::ipc::default_socket_path;
use omatype_broker::native_host::{
    NativeHostError, connect_and_bridge, validate_development_caller_origin,
};

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), ExitError> {
    match parse_arguments(std::env::args_os().skip(1))? {
        HostCommand::Help => {
            print!("{HOST_USAGE}");
            Ok(())
        }
        HostCommand::Run { socket_path } => {
            connect_and_bridge(&socket_path, tokio::io::stdin(), tokio::io::stdout()).await?;
            Ok(())
        }
    }
}

fn parse_arguments<I>(arguments: I) -> Result<HostCommand, ExitError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let first = arguments.next().ok_or(ExitError::Arguments)?;
    if first == "--help" || first == "-h" {
        return if arguments.next().is_none() {
            Ok(HostCommand::Help)
        } else {
            Err(ExitError::Arguments)
        };
    }
    let caller_origin = first.into_string().map_err(|_| ExitError::CallerOrigin)?;
    validate_development_caller_origin(&caller_origin).map_err(|_| ExitError::CallerOrigin)?;

    let socket_path = match arguments.next() {
        None => default_socket_path().map_err(|_| ExitError::SocketPath)?,
        Some(flag) if flag == "--socket" => {
            let path = PathBuf::from(arguments.next().ok_or(ExitError::Arguments)?);
            if !path.is_absolute() || arguments.next().is_some() {
                return Err(ExitError::Arguments);
            }
            path
        }
        Some(_) => return Err(ExitError::Arguments),
    };
    Ok(HostCommand::Run { socket_path })
}

const HOST_USAGE: &str = "Usage: omatype-native-host CALLER_ORIGIN [--socket ABSOLUTE]\n\
Bridges the fixed Omatype development Chromium extension to the private broker.\n\
Chrome supplies CALLER_ORIGIN as the first argument.\n\
Options:\n  --socket ABSOLUTE  Override $XDG_RUNTIME_DIR/omatype/broker.sock\n  -h, --help         Show this help\n";

#[derive(Clone, Debug, Eq, PartialEq)]
enum HostCommand {
    Help,
    Run { socket_path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
enum ExitError {
    #[error("arguments")]
    Arguments,
    #[error("caller_origin")]
    CallerOrigin,
    #[error(transparent)]
    NativeHost(#[from] NativeHostError),
    #[error("socket_path")]
    SocketPath,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{ExitError, HostCommand, parse_arguments};

    const ORIGIN: &str = "chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/";

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_fixed_origin_and_absolute_socket() {
        assert_eq!(
            parse_arguments(arguments(&[ORIGIN, "--socket", "/tmp/broker.sock"]))
                .expect("runtime arguments"),
            HostCommand::Run {
                socket_path: PathBuf::from("/tmp/broker.sock")
            }
        );
        assert_eq!(
            parse_arguments(arguments(&["--help"])).expect("help"),
            HostCommand::Help
        );
    }

    #[test]
    fn refuses_arbitrary_or_broad_caller_origins() {
        for origin in [
            "chrome-extension://abcdefghijklmnopabcdefghijklmnop/",
            "chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/*",
            "*",
        ] {
            assert!(matches!(
                parse_arguments(arguments(&[origin, "--socket", "/tmp/broker.sock"])),
                Err(ExitError::CallerOrigin)
            ));
        }
    }

    #[test]
    fn refuses_missing_origin_relative_socket_and_extra_arguments() {
        assert!(matches!(
            parse_arguments(arguments(&[])),
            Err(ExitError::Arguments)
        ));
        assert!(matches!(
            parse_arguments(arguments(&[ORIGIN, "--socket", "broker.sock"])),
            Err(ExitError::Arguments)
        ));
        assert!(matches!(
            parse_arguments(arguments(&[
                ORIGIN,
                "--socket",
                "/tmp/broker.sock",
                "extra"
            ])),
            Err(ExitError::Arguments)
        ));
    }
}
