#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use std::error::Error;
use std::os::unix::process::CommandExt;
use std::process::Command;

use badi_broker::writing_lab::process::EXEC_HELPER_FLAG;

#[test]
fn lab_binary_exec_helper_runs_before_model_or_tokio_startup() -> Result<(), Box<dyn Error>> {
    let target = std::fs::canonicalize("/bin/true")?;
    let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
        .arg(EXEC_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(target)
        .env_clear()
        .process_group(0)
        .output()?;
    assert!(
        output.status.success(),
        "helper failed: {:?}",
        output.stderr
    );
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty());
    Ok(())
}

#[test]
fn lab_binary_exec_helper_refuses_parent_mismatch_and_invalid_arguments()
-> Result<(), Box<dyn Error>> {
    let target = std::fs::canonicalize("/bin/true")?;
    let foreign_parent = rustix::process::getppid().ok_or("missing test parent")?;
    for args in [
        vec![],
        vec!["1".into(), target.clone().into_os_string()],
        vec![
            foreign_parent.as_raw_pid().to_string().into(),
            target.clone().into_os_string(),
        ],
        vec![
            std::process::id().to_string().into(),
            "relative-path".into(),
        ],
        vec![
            std::process::id().to_string().into(),
            target.into_os_string(),
            "extra".into(),
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg(EXEC_HELPER_FLAG)
            .args(args)
            .env_clear()
            .process_group(0)
            .output()?;
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"runtime_helper_failed\n");
    }
    Ok(())
}
