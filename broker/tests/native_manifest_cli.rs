use std::process::Command;

use serde_json::Value;

const ORIGIN: &str = "chrome-extension://ckkiehcjbclcjckkkajohopoikeejkoa/";

#[test]
fn prints_one_deterministic_manifest_without_installing_anything() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let host_path = directory.path().join("omatype-native-host");
    assert!(!host_path.exists());

    let first = Command::new(env!("CARGO_BIN_EXE_omatype-native-manifest"))
        .arg("--host-path")
        .arg(&host_path)
        .output()
        .expect("manifest command");
    let second = Command::new(env!("CARGO_BIN_EXE_omatype-native-manifest"))
        .arg("--host-path")
        .arg(&host_path)
        .output()
        .expect("second manifest command");

    assert!(first.status.success());
    assert!(first.stderr.is_empty());
    assert_eq!(first.stdout, second.stdout);
    assert!(!host_path.exists());

    let manifest: Value = serde_json::from_slice(&first.stdout).expect("manifest JSON");
    let object = manifest.as_object().expect("manifest object");
    assert_eq!(object.len(), 5);
    assert_eq!(manifest["name"], "io.omatype.broker");
    assert_eq!(
        manifest["description"],
        "Omatype private local broker bridge"
    );
    assert_eq!(manifest["path"], host_path.to_str().expect("UTF-8 path"));
    assert_eq!(manifest["type"], "stdio");
    assert_eq!(manifest["allowed_origins"], serde_json::json!([ORIGIN]));
}

#[test]
fn refuses_relative_paths_and_caller_controlled_origins() {
    for arguments in [
        ["--host-path", "relative/host"],
        ["--origin", "*"],
        ["--origin", ORIGIN],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_omatype-native-manifest"))
            .args(arguments)
            .output()
            .expect("manifest command");
        assert!(!output.status.success());
        assert!(output.stdout.is_empty());
        assert_eq!(output.stderr, b"error_code=arguments\n");
    }
}
