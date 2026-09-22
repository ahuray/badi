#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use serde_json::{Value, json};
use std::error::Error;
use std::process::Command;

#[test]
fn malformed_explicit_artifacts_fail_before_worker_or_paced_runtime_activation()
-> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let descriptor = root.path().join("artifact.json");
    for bytes in [
        br#"{"schema":"badi.lab-model-artifact.v1"}"#.to_vec(),
        vec![b' '; 16 * 1024 + 1],
        br#"{"runtime":"private-path-canary"}"#.to_vec(),
    ] {
        std::fs::write(&descriptor, bytes)?;
        for paced in [false, true] {
            let mut command = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"));
            if paced {
                command.arg("--paced-probe");
            }
            let output = command
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
    }
    Ok(())
}

#[test]
fn artifact_flags_are_strict_and_fixed_prefill_probe_cannot_be_reconfigured()
-> Result<(), Box<dyn Error>> {
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
        vec!["--prefill-probe", "--model-artifact", "/missing.json"],
        vec!["--prefill-probe", "--prefill-batch", "64"],
        vec!["--prefill-batch", "32"],
        vec!["--prefill-batch", "064"],
        vec!["--prefill-batch", "64", "--prefill-batch", "16"],
        vec!["--paced-probe", "--prefill-batch", "32"],
        vec![
            "--paced-probe",
            "--prefill-batch",
            "64",
            "--prefill-batch",
            "16",
        ],
        vec![
            "--paced-probe",
            "--model-artifact",
            "/missing.json",
            "--runtime",
            "/bin/true",
        ],
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
