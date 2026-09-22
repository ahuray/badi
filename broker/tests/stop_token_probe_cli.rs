#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use serde_json::{Value, json};
use std::error::Error;
use std::process::Command;

#[test]
fn fixed_stop_probe_rejects_variable_inputs_before_activation() -> Result<(), Box<dyn Error>> {
    for args in [
        vec![],
        vec!["--model-artifact"],
        vec!["--model-artifact", "relative.json"],
        vec!["--model-directory", "/tmp"],
        vec!["--prompt", "private-canary"],
        vec!["--model-artifact", "/missing.json", "--prefill-batch", "64"],
        vec![
            "--model-artifact",
            "/missing.json",
            "--model-artifact",
            "/other.json",
        ],
        vec![
            "--model-artifact",
            "/missing.json",
            "--grammar",
            "private-canary",
        ],
        vec!["--model-artifact", "/missing.json", "--budget-ms", "999999"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg("--stop-token-probe")
            .args(args)
            .output()?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            value,
            json!({"type":"error","id":null,"error":"invalid_arguments"})
        );
    }
    Ok(())
}

#[test]
fn wrong_model_descriptor_is_rejected_before_loading() -> Result<(), Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let weights = root.path().join("fixture.gguf");
    std::fs::write(&weights, b"disposable")?;
    let descriptor = root.path().join("artifact.json");
    for (sha, bytes) in [
        ("a".repeat(64), 1_282_439_264u64),
        (
            "d2387ca2dbfee2ffabce7120d3770dadca0b293052bc2f0e138fdc940d9bc7b5".into(),
            10,
        ),
    ] {
        std::fs::write(
            &descriptor,
            serde_json::to_vec(
                &json!({"schema":"badi.lab-model-artifact.v1","weights_path":weights,"sha256":sha,"bytes":bytes,"alias":"fixture"}),
            )?,
        )?;
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg("--stop-token-probe")
            .arg("--model-artifact")
            .arg(&descriptor)
            .output()?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let value: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            value,
            json!({"type":"error","id":null,"error":"stop_probe_model_mismatch"})
        );
    }
    Ok(())
}
