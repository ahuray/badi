#![cfg(target_os = "linux")]

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn broker_commands_print_their_replies_and_refusals() {
    let broker = PhraseBroker::start();

    let default_settings = "{\n  \"paused\": true,\n  \"revision\": 0,\n  \
        \"schema\": \"badi.settings.v2\",\n  \"subjects\": []\n}\n";
    assert_eq!(broker.succeeds(&["settings", "show"]), default_settings);
    assert_eq!(
        broker.succeeds(&["settings", "show", "--json"]),
        default_settings
    );

    let unpaused = r#"{"schema":"badi.settings.v2","revision":1,"paused":false,"subjects":[]}"#;
    let replace = [
        "settings",
        "replace",
        "--if-revision",
        "0",
        "--json",
        unpaused,
    ];
    assert_eq!(
        broker.succeeds(&replace),
        "{\n  \"paused\": false,\n  \"revision\": 1,\n  \
        \"schema\": \"badi.settings.v2\",\n  \"subjects\": []\n}\n"
    );
    broker.fails(
        &replace,
        "error_code=settings_rejected:code=settings_conflict:committed=false:\
        settings_revision=1:degraded=false\n",
    );

    let status = broker.succeeds(&["status"]);
    assert_eq!(status.lines().count(), 1, "status is one compact line");
    let status: Value = serde_json::from_str(&status).expect("status JSON");
    assert_eq!(
        object_keys(&status),
        [
            "active",
            "authority_epoch",
            "control_plane_degraded",
            "max_frame_bytes",
            "metrics",
            "paused",
            "provider",
            "sessions",
            "settings_revision",
            "socket_mode",
        ]
    );
    assert_eq!(status["provider"], "phrase_v1");
    assert_eq!(status["settings_revision"], 1);
    assert_eq!(status["socket_mode"], "0600");
    assert_eq!(
        status["active"],
        json!({"present": false, "has_suggestion": false})
    );

    let overview = broker.succeeds(&["overview", "--json"]);
    assert!(overview.starts_with("{\n  \"schema\": \"badi.overview.v2\",\n"));
    let overview: Value = serde_json::from_str(&overview).expect("overview JSON");
    assert_eq!(overview["broker"]["settings_revision"], 1);
    assert_eq!(overview["settings"]["revision"], 1);

    assert_eq!(
        broker.succeeds(&["memory", "clear"]),
        "{\"revision\":0,\"records\":0,\"bytes\":0,\"changed\":false}\n"
    );

    for (arguments, action, paused) in [
        (&["pause", "on"][..], "pause", true),
        (&["pause", "off"][..], "resume", false),
        (&["pause"][..], "pause_toggle", true),
    ] {
        let reply: Value = serde_json::from_str(&broker.succeeds(arguments)).expect("control JSON");
        assert_eq!(reply["v"], 2);
        assert_eq!(reply["type"], "control.result");
        assert!(
            reply["id"]
                .as_str()
                .is_some_and(|id| id.starts_with("ctl:"))
        );
        assert_eq!(
            reply["payload"],
            json!({"accepted": true, "action": action, "paused": paused, "reason": "accepted"})
        );
    }
    broker.succeeds(&["pause", "toggle"]);

    for command in ["request", "accept-word", "accept-all", "dismiss"] {
        broker.fails(&[command], "error_code=no_active_session\n");
    }

    let probe = broker.succeeds_with_stdin(&["probe", "--explicit", "-"], "Thank you\r\n");
    let probe: Value = serde_json::from_str(&probe).expect("probe JSON");
    assert_eq!(
        object_keys(&probe),
        ["latency_ms", "outcome", "provider", "round_trip_ms", "text"]
    );
    assert_eq!(probe["outcome"], "suggested");
    assert_eq!(probe["provider"], "phrase_v1");
    assert_eq!(probe["text"], " for your time");
    assert!(probe["round_trip_ms"].is_u64());
}

fn object_keys(value: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = value
        .as_object()
        .expect("JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

struct PhraseBroker {
    process: Child,
    socket: PathBuf,
    _directory: tempfile::TempDir,
}

impl PhraseBroker {
    fn start() -> Self {
        let directory = tempfile::tempdir().expect("temporary directory");
        let socket = directory.path().join("private/broker.sock");
        let process = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
            .args(["--provider", "phrase", "--socket"])
            .arg(&socket)
            .env("XDG_CONFIG_HOME", directory.path().join("config"))
            .env("XDG_DATA_HOME", directory.path().join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("broker process");
        let mut broker = Self {
            process,
            socket,
            _directory: directory,
        };
        broker.wait_for_socket();
        broker
    }

    fn wait_for_socket(&mut self) {
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while !self.socket.exists() {
            assert!(
                self.process.try_wait().expect("broker status").is_none(),
                "broker exited before binding"
            );
            assert!(Instant::now() < deadline, "broker did not bind in time");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn badictl(&self, arguments: &[&str], stdin: &str) -> Output {
        let mut child = Command::new(env!("CARGO_BIN_EXE_badictl"))
            .arg("--socket")
            .arg(&self.socket)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("badictl process");
        child
            .stdin
            .take()
            .expect("piped stdin")
            .write_all(stdin.as_bytes())
            .expect("write stdin");
        child.wait_with_output().expect("badictl output")
    }

    fn succeeds(&self, arguments: &[&str]) -> String {
        self.succeeds_with_stdin(arguments, "")
    }

    fn succeeds_with_stdin(&self, arguments: &[&str], stdin: &str) -> String {
        let output = self.badictl(arguments, stdin);
        assert!(output.status.success(), "{arguments:?}: {output:?}");
        assert!(output.stderr.is_empty(), "{arguments:?}: {output:?}");
        String::from_utf8(output.stdout).expect("UTF-8 stdout")
    }

    fn fails(&self, arguments: &[&str], stderr: &str) {
        let output = self.badictl(arguments, "");
        assert_eq!(output.status.code(), Some(1), "{arguments:?}: {output:?}");
        assert!(output.stdout.is_empty(), "{arguments:?}: {output:?}");
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            stderr,
            "{arguments:?}"
        );
    }
}

impl Drop for PhraseBroker {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}
