use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use serde_json::{Value, json};

use super::*;
use crate::protocol::Coordinates;

fn run_directory(text: Option<bool>, expires_in: i64) -> tempfile::TempDir {
    let directory = tempfile::tempdir().expect("directory");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_secs();
    let mut control = json!({
        "id": "0b6c9a8e-3f43-4f53-9d2a-6d3c1a1b2c3d",
        "expires_at": now.saturating_add_signed(expires_in),
    });
    if let Some(text) = text {
        control["text"] = Value::Bool(text);
    }
    for (name, contents) in [
        (CONTROL_FILE, control.to_string()),
        (TRACE_FILE, String::new()),
    ] {
        let path = directory.path().join(name);
        fs::write(&path, contents).expect("write");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("mode");
    }
    directory
}

fn lines(directory: &tempfile::TempDir) -> Vec<Value> {
    fs::read_to_string(directory.path().join(TRACE_FILE))
        .expect("trace")
        .lines()
        .map(|line| serde_json::from_str(line).expect("json line"))
        .collect()
}

fn context_frame(sensitive: bool) -> WireEnvelope {
    WireEnvelope::session(
        MessageType::ContextChanged,
        Coordinates {
            session_id: SessionId::new(),
            focus_epoch: 1,
            revision: 4,
        },
        0,
        &json!({
            "fingerprint": "f",
            "before": "Dear team, thank you",
            "after": " for everything",
            "explicit": false,
            "language": "en",
            "field": {"purpose": "normal", "sensitive": sensitive, "multiline": true},
        }),
    )
    .expect("frame")
}

fn trace(directory: &tempfile::TempDir, envelope: &WireEnvelope) {
    let tracer = Tracer::new(Some(directory.path().to_owned()));
    tracer.emit("in context.changed", |entry| {
        describe(entry, AdapterKind::Fcitx, envelope);
    });
}

#[test]
fn an_events_only_run_records_lengths_but_no_text() {
    let directory = run_directory(None, 600);
    trace(&directory, &context_frame(false));
    let [line] = lines(&directory).try_into().expect("one line");
    assert_eq!(line["src"], "broker");
    assert_eq!(line["ev"], "in context.changed");
    assert_eq!(line["adapter"], "fcitx");
    assert_eq!(line["rev"], 4);
    assert_eq!(line["before_chars"], 20);
    assert_eq!(line["after_chars"], 15);
    assert!(line.get("before").is_none() && line.get("after").is_none());
}

#[test]
fn a_text_run_keeps_context_except_from_a_sensitive_field() {
    let directory = run_directory(Some(true), 600);
    trace(&directory, &context_frame(false));
    trace(&directory, &context_frame(true));
    let [plain, sensitive] = lines(&directory).try_into().expect("two lines");
    assert_eq!(plain["before"], "Dear team, thank you");
    assert_eq!(plain["after"], " for everything");
    assert!(sensitive.get("before").is_none() && sensitive.get("after").is_none());
    assert_eq!(sensitive["before_chars"], 20);
}

#[test]
fn nothing_is_written_without_a_valid_unexpired_run_or_its_trace_file() {
    for directory in [
        run_directory(Some(true), -1),
        run_directory(Some(true), 3600),
    ] {
        trace(&directory, &context_frame(false));
        assert_eq!(lines(&directory), Vec::<Value>::new());
    }
    let directory = run_directory(Some(true), 600);
    fs::remove_file(directory.path().join(TRACE_FILE)).expect("remove");
    trace(&directory, &context_frame(false));
    assert!(!directory.path().join(TRACE_FILE).exists());
}

#[test]
fn a_trace_readable_by_others_is_left_untouched() {
    let directory = run_directory(None, 600);
    let path = directory.path().join(TRACE_FILE);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("mode");
    trace(&directory, &context_frame(false));
    assert_eq!(lines(&directory), Vec::<Value>::new());
}

#[test]
fn context_keeps_its_last_and_suggestions_their_first_characters() {
    assert_eq!(tail("abcdef", 3), "def");
    assert_eq!(head("abcdef", 3), "abc");
    assert_eq!(tail("ab", 3), "ab");
    assert_eq!(tail("سلام دنیا", 4), "دنیا");
}
