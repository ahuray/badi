#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use serde_json::{Value, json};
use std::error::Error;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn input() -> Value {
    let events = [0,100,250,500].into_iter().enumerate().map(|(index, at_ms)| json!({"at_ms":at_ms,
            "request":{"schema":"badi.prediction-lab.request.v1","id":uuid::Uuid::new_v4(),
                "before":format!("private-disposable-draft-canary {}", &"abc"[..index]),"language":"en","context":"Stable context.",
                "style_examples":[],"config":{"mode":"healed","budget_ms":550,"max_tokens":8,
                    "cache_prompt":true,"temperature":0,"seed":42}}})).collect::<Vec<_>>();
    json!({"schema":"badi.paced-probe.request.v1","arm":"cold","prelude_ms":1500,"events":events})
}

fn run(bytes: &[u8]) -> Result<Output, Box<dyn Error>> {
    let directory = tempfile::tempdir()?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
        .arg("--paced-probe")
        .arg("--model-directory")
        .arg(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Err(error) = child.stdin.take().unwrap().write_all(bytes) {
        if error.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(error.into());
        }
    }
    Ok(child.wait_with_output()?)
}

#[test]
fn paced_probe_rejects_labels_unknown_fields_and_modified_frozen_settings()
-> Result<(), Box<dyn Error>> {
    for modification in 0..4 {
        let mut input = input();
        match modification {
            0 => input["events"][0]["request"]["expected"] = json!(["private-answer-canary"]),
            1 => input["events"][0]["trace"] = json!("hidden-label"),
            2 => input["events"][1]["request"]["config"]["budget_ms"] = json!(551),
            _ => input["events"][1]["at_ms"] = json!(101),
        }
        let output = run(format!("{input}\n").as_bytes())?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let record: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            record,
            json!({"type":"error","id":null,"error":"invalid_paced_request"})
        );
    }
    Ok(())
}

#[test]
fn paced_probe_bounds_input_and_requires_one_complete_frame() -> Result<(), Box<dyn Error>> {
    for bytes in [vec![b'x'; 256 * 1024 + 2], input().to_string().into_bytes()] {
        let output = run(&bytes)?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let record: Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(record["error"], "invalid_frame");
    }
    Ok(())
}

#[test]
fn paced_probe_eof_is_normal_and_startup_failure_preserves_four_private_opportunities()
-> Result<(), Box<dyn Error>> {
    let output = run(format!("{}\n", input()).as_bytes())?;
    assert!(!output.status.success() && output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-disposable-draft-canary"));
    let record: Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(record["type"], "paced_report");
    let report = &record["report"];
    assert_eq!(report["cancelled"], false);
    assert_eq!(report["complete"], false);
    assert_eq!(report["error"], "runtime_start_failed");
    assert!(report["identity"].is_null() && report["cleanup"].is_null());
    let rows = report["events"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|row| row["disposition"] == "unavailable" && row["result"].is_null())
    );
    Ok(())
}
