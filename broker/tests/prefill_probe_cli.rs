#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use std::error::Error;
use std::process::Command;

#[test]
fn prefill_probe_rejects_variable_inputs_before_runtime_activation() -> Result<(), Box<dyn Error>> {
    for args in [
        vec!["--prompt", "private-input-canary"],
        vec!["--n-predict", "1"],
        vec!["--budget-ms", "999999"],
        vec!["--model-directory", "relative"],
        vec!["--model-directory", "/tmp/fixed-probe", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg("--prefill-probe")
            .args(args)
            .output()?;
        assert!(!output.status.success());
        assert!(output.stderr.is_empty());
        let response: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            response,
            serde_json::json!({"type":"error","id":null,"error":"invalid_arguments"})
        );
    }
    Ok(())
}
