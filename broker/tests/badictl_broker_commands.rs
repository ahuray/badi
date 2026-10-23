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
            "desktop",
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
    assert_eq!(
        status["desktop"],
        json!({"observed": false, "session": "active", "low_power": false, "automatic": "normal",
               "screen_share": "none"})
    );

    let overview = broker.succeeds(&["overview", "--json"]);
    assert!(overview.starts_with("{\n  \"schema\": \"badi.overview.v2\",\n"));
    let overview: Value = serde_json::from_str(&overview).expect("overview JSON");
    assert_eq!(overview["broker"]["settings_revision"], 1);
    assert_eq!(overview["broker"]["desktop"], status["desktop"]);
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

#[test]
fn snooze_names_why_nothing_was_snoozed_and_ends_at_once() {
    let broker = PhraseBroker::start();
    // A snooze names the focused field's app or site; with none, it says so.
    broker.fails(&["snooze"], "error_code=rejected:paused\n");
    let unpaused = r#"{"schema":"badi.settings.v2","revision":1,"paused":false,"subjects":[]}"#;
    broker.succeeds(&[
        "settings",
        "replace",
        "--if-revision",
        "0",
        "--json",
        unpaused,
    ]);
    broker.fails(&["snooze"], "error_code=rejected:no_context\n");
    broker.fails(&["snooze", "0"], "error_code=arguments\n");
    let reply: Value =
        serde_json::from_str(&broker.succeeds(&["snooze", "end"])).expect("snooze end JSON");
    assert_eq!(
        reply["payload"],
        json!({"accepted": true, "action": "snooze_end", "paused": false, "reason": "accepted"})
    );
}

#[test]
fn style_commands_keep_the_key_out_of_argv_and_output() {
    let broker = PhraseBroker::start();
    // The key arrives on stdin, never in argv, and is never printed.
    let absent =
        "{\"enabled\":false,\"available\":true,\"key\":\"absent\",\"samples\":0,\"subjects\":0}\n";
    assert_eq!(broker.succeeds(&["style", "status"]), absent);
    // Without a key the export is refused; `style status` names why.
    broker.fails(
        &["style", "export"],
        "error_code=rejected:invalid_message\n",
    );
    let key = format!("{} {}\n", "5a".repeat(16), "c3".repeat(32));
    let ready = broker.succeeds_with_stdin(&["style", "key", "-"], &key);
    assert_eq!(
        ready,
        "{\"enabled\":false,\"available\":true,\"key\":\"ready\",\"samples\":0,\"subjects\":0}\n"
    );
    assert!(!ready.contains(&"c3".repeat(32)));
    assert_eq!(
        broker.succeeds(&["style", "export"]),
        "{\n  \"samples\": []\n}\n"
    );
    assert_eq!(broker.succeeds(&["style", "forget"]), absent);
    broker.fails(&["style", "key", "-"], "error_code=arguments\n");
    broker.fails(
        &["style", "key", &"5a".repeat(16)],
        "error_code=arguments\n",
    );
}

#[test]
fn stats_commands_read_counts_a_previous_broker_kept_and_forget_them() {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    // A previous broker left these counts; the next one reads them at start.
    let directory = tempfile::tempdir().expect("temporary directory");
    let data = directory.path().join("data/badi");
    std::fs::create_dir_all(directory.path().join("data")).expect("data home");
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&data)
        .expect("private data directory");
    let today = badi_broker::stats::local_today();
    let yesterday = today.yesterday().expect("yesterday");
    let ghostty = r#"{"kind":"linux_app","adapter":"fcitx","app_id":"com.mitchellh.ghostty"}"#;
    let site = r#"{"kind":"browser_origin","adapter":"chromium","scheme":"https","host":"example.com","port":443}"#;
    let kept = format!(
        r#"{{"schema":"badi.stats.v1","days":[{{"day":"{yesterday}","subjects":[{{"identity":{site},"shown":9,"accepted":4,"words":7}}]}},{{"day":"{today}","subjects":[{{"identity":{ghostty},"shown":3,"accepted":2,"words":5}}]}}]}}"#
    );
    let path = data.join("stats.json");
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .expect("statistics file")
        .write_all(kept.as_bytes())
        .expect("write");
    let broker = PhraseBroker::start_in(directory);

    let status: Value =
        serde_json::from_str(&broker.succeeds(&["stats", "status"])).expect("stats JSON");
    assert_eq!(status["available"], true);
    let summary = &status["summary"];
    assert_eq!(summary["today"], today.to_string());
    assert_eq!(
        summary["totals"]["today"],
        json!({"shown": 3, "accepted": 2, "words": 5})
    );
    assert_eq!(
        summary["totals"]["last_7_days"],
        json!({"shown": 12, "accepted": 6, "words": 12})
    );
    assert_eq!(summary["days"].as_array().expect("days").len(), 2);
    assert_eq!(summary["subjects"][0]["identity"]["host"], "example.com");
    assert_eq!(
        summary["subjects"][1]["identity"]["app_id"],
        "com.mitchellh.ghostty"
    );

    let forgotten: Value =
        serde_json::from_str(&broker.succeeds(&["stats", "forget"])).expect("stats JSON");
    assert_eq!(forgotten["summary"]["days"], json!([]));
    assert!(!path.exists(), "forget deletes the file");
    broker.fails(&["stats"], "error_code=arguments\n");
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
        Self::start_in(tempfile::tempdir().expect("temporary directory"))
    }

    /// A broker whose XDG config and data homes are `config/` and `data/`
    /// in `directory`.
    fn start_in(directory: tempfile::TempDir) -> Self {
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
