use std::ffi::OsString;
use std::path::PathBuf;

use omatype_broker::native_host::{ManifestError, render_native_manifest};

fn main() {
    if let Err(error) = run() {
        eprintln!("error_code={error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ExitError> {
    match parse_arguments(std::env::args_os().skip(1))? {
        ManifestCommand::Help => print!("{MANIFEST_USAGE}"),
        ManifestCommand::Print { host_path } => {
            print!("{}", render_native_manifest(&host_path)?);
        }
    }
    Ok(())
}

fn parse_arguments<I>(arguments: I) -> Result<ManifestCommand, ExitError>
where
    I: IntoIterator<Item = OsString>,
{
    let mut arguments = arguments.into_iter();
    let first = arguments.next().ok_or(ExitError::Arguments)?;
    if first == "--help" || first == "-h" {
        return if arguments.next().is_none() {
            Ok(ManifestCommand::Help)
        } else {
            Err(ExitError::Arguments)
        };
    }
    if first != "--host-path" {
        return Err(ExitError::Arguments);
    }
    let host_path = PathBuf::from(arguments.next().ok_or(ExitError::Arguments)?);
    if !host_path.is_absolute() || arguments.next().is_some() {
        return Err(ExitError::Arguments);
    }
    Ok(ManifestCommand::Print { host_path })
}

const MANIFEST_USAGE: &str = "Usage: omatype-native-manifest --host-path ABSOLUTE\n\
Prints the deterministic Chromium NativeMessagingHosts manifest to stdout.\n\
It never installs or writes the manifest.\n\
Options:\n  --host-path ABSOLUTE  Exact omatype-native-host executable path\n  -h, --help            Show this help\n";

#[derive(Clone, Debug, Eq, PartialEq)]
enum ManifestCommand {
    Help,
    Print { host_path: PathBuf },
}

#[derive(Debug, thiserror::Error)]
enum ExitError {
    #[error("arguments")]
    Arguments,
    #[error(transparent)]
    Manifest(#[from] ManifestError),
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{ExitError, ManifestCommand, parse_arguments};

    fn arguments(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn parses_print_only_manifest_command() {
        assert_eq!(
            parse_arguments(arguments(&[
                "--host-path",
                "/opt/omatype/omatype-native-host"
            ]))
            .expect("manifest arguments"),
            ManifestCommand::Print {
                host_path: PathBuf::from("/opt/omatype/omatype-native-host")
            }
        );
        assert_eq!(
            parse_arguments(arguments(&["--help"])).expect("help"),
            ManifestCommand::Help
        );
    }

    #[test]
    fn rejects_relative_paths_origins_and_extra_arguments() {
        for case in [
            vec!["--host-path", "relative/host"],
            vec!["--origin", "*"],
            vec!["--host-path", "/opt/host", "extra"],
        ] {
            assert!(matches!(
                parse_arguments(arguments(&case)),
                Err(ExitError::Arguments)
            ));
        }
    }
}
