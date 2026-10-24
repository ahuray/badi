#![cfg(target_os = "linux")]

use serde_json::{Value, json};
use std::error::Error;
use std::process::Command;

#[test]
fn malformed_explicit_artifacts_fail_before_runtime_activation() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let descriptor = root.path().join("artifact.json");
    for bytes in [
        br#"{"schema":"badi.lab-model-artifact.v1"}"#.to_vec(),
        vec![b' '; 16 * 1024 + 1],
        br#"{"runtime":"private-path-canary"}"#.to_vec(),
    ] {
        std::fs::write(&descriptor, bytes)?;
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg("--model-artifact")
            .arg(&descriptor)
            .arg("--model-directory")
            .arg(root.path())
            .output()?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let record: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            record,
            json!({"type":"error","id":null,"error":"invalid_model_artifact"})
        );
    }
    Ok(())
}

#[test]
fn artifact_flags_are_strict_and_removed_probes_are_refused() -> Result<(), Box<dyn Error>> {
    for args in [
        vec!["--model-artifact"],
        vec!["--model-artifact", "relative.json"],
        vec![
            "--model-artifact",
            "/missing.json",
            "--model-artifact",
            "/other.json",
        ],
        vec!["--model-directory", "/tmp", "--model-directory", "/var/tmp"],
        vec!["--prefill-batch", "32"],
        vec!["--prefill-batch", "064"],
        vec!["--prefill-batch", "64", "--prefill-batch", "16"],
        vec![
            "--model-artifact",
            "/missing.json",
            "--runtime",
            "/bin/true",
        ],
        // The retired probes and spelling engine are no longer dispatched.
        vec!["--prefill-probe"],
        vec!["--paced-probe"],
        vec!["--stop-token-probe", "--model-artifact", "/missing.json"],
        vec!["--spelling-lab"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .args(args)
            .output()?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let record: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            record,
            json!({"type":"error","id":null,"error":"invalid_arguments"})
        );
    }
    Ok(())
}
