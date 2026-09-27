#![cfg(all(feature = "local-model", target_os = "linux"))]

use std::error::Error;
use std::os::unix::process::CommandExt;
use std::process::Command;

use badi_broker::writing::EXEC_HELPER_FLAG;

#[test]
fn broker_exec_helper_runs_before_tokio_and_refuses_a_foreign_parent() -> Result<(), Box<dyn Error>>
{
    let target = std::fs::canonicalize("/bin/true")?;
    let output = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg(EXEC_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(&target)
        .env_clear()
        .process_group(0)
        .output()?;
    assert!(output.status.success(), "helper failed: {output:?}");
    assert!(output.stdout.is_empty() && output.stderr.is_empty());

    let foreign_parent = rustix::process::getppid().ok_or("missing test parent")?;
    let output = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg(EXEC_HELPER_FLAG)
        .arg(foreign_parent.as_raw_pid().to_string())
        .arg(&target)
        .env_clear()
        .process_group(0)
        .output()?;
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(output.stderr, b"runtime_helper_failed\n");
    Ok(())
}

/// The service unit's `RestartPreventExitStatus=78` relies on this status: a
/// missing installation cannot be fixed by restarting.
#[test]
fn missing_model_exits_with_the_configuration_status() -> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let output = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg("--socket")
        .arg(directory.path().join("private/broker.sock"))
        .arg("--model-directory")
        .arg(directory.path().join("models"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .output()?;
    assert_eq!(output.status.code(), Some(78), "{output:?}");
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)?.starts_with("error_code=local_model: "),
        "startup errors keep the prefix badi doctor classifies"
    );
    assert!(!directory.path().join("private/broker.sock").exists());
    Ok(())
}
