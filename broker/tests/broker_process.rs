#![cfg(target_os = "linux")]

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

    let echo = std::fs::canonicalize("/bin/echo")?;
    let masked = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg(EXEC_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(&echo)
        .args(["--cpu-mask", "0xfff"])
        .env_clear()
        .process_group(0)
        .output()?;
    assert!(masked.status.success(), "masked helper failed: {masked:?}");
    assert_eq!(masked.stdout, b"--cpu-mask 0xfff\n");
    assert_eq!(masked.stderr, [] as [u8; 0]);

    for invalid_extra in [
        vec!["--cpu-mask", "0x0"],
        vec!["--cpu-mask", "0xFFF"],
        vec!["--other", "0xfff"],
        vec!["--cpu-mask", "0xfff", "extra"],
    ] {
        let rejected = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
            .arg(EXEC_HELPER_FLAG)
            .arg(std::process::id().to_string())
            .arg(&target)
            .args(invalid_extra)
            .env_clear()
            .process_group(0)
            .output()?;
        assert_eq!(rejected.status.code(), Some(1));
        assert_eq!(rejected.stderr, b"runtime_helper_failed\n");
    }

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
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert!(
        String::from_utf8(output.stderr)?.starts_with("error_code=local_model: "),
        "startup errors keep the prefix badi doctor classifies"
    );
    assert!(!directory.path().join("private/broker.sock").exists());
    Ok(())
}

#[test]
fn unrecognized_settings_schema_on_disk_exits_with_the_configuration_status()
-> Result<(), Box<dyn Error>> {
    use std::os::unix::fs::PermissionsExt as _;

    let directory = tempfile::tempdir()?;
    let config_dir = directory.path().join("config/badi");
    let data_dir = directory.path().join("data/badi");
    for path in [&config_dir, &data_dir] {
        std::fs::create_dir_all(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    let settings_path = config_dir.join("settings.json");
    let retired_v1 = br#"{"schema":"badi.settings.v1","revision":1,"paused":false,"subjects":[]}"#;
    std::fs::write(&settings_path, retired_v1)?;
    std::fs::set_permissions(&settings_path, std::fs::Permissions::from_mode(0o600))?;

    let output = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg("--provider")
        .arg("phrase")
        .arg("--socket")
        .arg(directory.path().join("private/broker.sock"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .output()?;
    assert_eq!(output.status.code(), Some(78), "{output:?}");
    assert_eq!(output.stdout, [] as [u8; 0]);
    assert_eq!(
        String::from_utf8(output.stderr)?,
        "error_code=control_plane: json\n",
        "retired v1 settings emit the configuration error badi doctor classifies"
    );
    assert!(!directory.path().join("private/broker.sock").exists());
    assert_eq!(std::fs::read(&settings_path)?, retired_v1);
    Ok(())
}
