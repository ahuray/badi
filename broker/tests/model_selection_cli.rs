use std::process::Command;

use serde_json::Value;

#[test]
fn hardware_report_is_local_json_without_a_runtime_directory() {
    let output = Command::new(env!("CARGO_BIN_EXE_badictl"))
        .args(["hardware", "--json"])
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("hardware command");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("hardware JSON");
    assert_eq!(report["schema"], "badi.hardware.v1");
    assert!(report["architecture"].is_string());
    assert!(
        report["logical_cpus"]
            .as_u64()
            .is_some_and(|count| count >= 1)
    );
    assert!(report["memory"].is_object());
    assert!(report["gpu"].is_object());
}

#[test]
fn model_report_is_pinned_and_never_claims_runtime_readiness() {
    let output = Command::new(env!("CARGO_BIN_EXE_badictl"))
        .args(["models", "code", "--json"])
        .env_remove("XDG_RUNTIME_DIR")
        .output()
        .expect("models command");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let report: Value = serde_json::from_slice(&output.stdout).expect("models JSON");
    assert_eq!(report["schema"], "badi.model-advice.v1");
    assert_eq!(report["hardware"]["schema"], "badi.hardware.v1");
    assert_eq!(report["use_case"], "code");
    assert_eq!(report["runtime_ready"], false);
    assert_eq!(report["recommended"]["license"], "Apache-2.0");
    assert_eq!(report["recommended"]["runtime"], "llama.cpp");
    assert_eq!(report["download"]["tool"], "hf");
    assert_eq!(
        report["recommended"]["revision"]
            .as_str()
            .expect("revision")
            .len(),
        40
    );
    assert_eq!(
        report["download"]["expected_sha256"]
            .as_str()
            .expect("digest")
            .len(),
        64
    );
}
