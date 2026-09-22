#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use std::error::Error;
use std::process::Command;

#[test]
fn spelling_dispatch_rejects_invalid_configuration_without_model_activation()
-> Result<(), Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let manifest = directory.path().join("manifest.json");
    std::fs::write(&manifest, b"{}")?;
    for args in [
        vec!["--spelling-lab".into()],
        vec![
            "--spelling-lab".into(),
            "--spelling-manifest".into(),
            manifest.clone().into_os_string(),
            "--language".into(),
            "de".into(),
        ],
        vec![
            "--spelling-lab".into(),
            "--model-directory".into(),
            directory.path().as_os_str().to_owned(),
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .args(args)
            .env_clear()
            .output()?;
        assert!(output.stderr.is_empty());
        let error: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(error["type"], "error");
        assert!(error["id"].is_null());
        assert!(matches!(
            error["error"].as_str(),
            Some("invalid_arguments" | "invalid_spelling_artifact")
        ));
    }
    Ok(())
}

#[test]
fn spelling_exec_helper_rejects_parent_mismatch_before_artifact_or_tokio_startup()
-> Result<(), Box<dyn Error>> {
    let foreign_parent = rustix::process::getppid().ok_or("missing test parent")?;
    let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
        .arg(badi_broker::writing_lab::spelling::engine::HELPER_FLAG)
        .arg(foreign_parent.as_raw_pid().to_string())
        .args(["/missing/private-manifest", "de"])
        .env_clear()
        .output()?;
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(output.stderr, b"spelling_helper_failed\n");
    Ok(())
}
