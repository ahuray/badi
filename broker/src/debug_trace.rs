//! The opt-in event trace behind `badi debug on`.
//!
//! While the controller's `debug-control.json` is valid and unexpired, each
//! event appends one JSON line to `debug-trace.jsonl` in the private runtime
//! directory, beside the Fcitx addon's, the observer's and the editors'
//! lines. The broker never creates the trace: `badi debug on` does, and
//! `badi debug off` deletes it, which stops every writer at once. Events are
//! content-free unless the control also says `"text": true` (`badi debug on
//! --text`); then context tails and suggestion text are included, except from
//! a field that reports itself sensitive.

use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use rustix::fs::{CWD, Mode, OFlags};
use serde_json::{Map, Value};

use crate::protocol::{AdapterKind, MessageType, SessionId, WireEnvelope};

const CONTROL_FILE: &str = "debug-control.json";
const TRACE_FILE: &str = "debug-trace.jsonl";
const MAX_CONTROL_BYTES: u64 = 512;
/// Writers stop appending once the trace reaches this size.
const MAX_TRACE_BYTES: u64 = 16 * 1024 * 1024;
/// Matches the controller: a debug run never lasts longer than this.
const MAX_RUN_SECONDS: u64 = 900;
const RECHECK: Duration = Duration::from_secs(1);
/// Text kept from the end of the context before the caret.
const BEFORE_CHARS: usize = 160;
/// Text kept from the start of the context after the caret.
const AFTER_CHARS: usize = 80;
/// Suggestion and commit text kept from the start.
const SUGGESTION_CHARS: usize = 400;

/// One trace line under construction. Text fields are dropped unless the run
/// recorded text.
pub struct Entry {
    fields: Map<String, Value>,
    text: bool,
}

impl Entry {
    pub fn put(&mut self, key: &str, value: impl Into<Value>) {
        self.fields.insert(key.to_owned(), value.into());
    }

    /// The first `SUGGESTION_CHARS` of suggestion or commit text.
    pub fn text(&mut self, key: &str, value: &str) {
        if self.text {
            self.put(key, head(value, SUGGESTION_CHARS));
        }
    }

    fn copy(&mut self, payload: &Value, fields: &[(&str, &str)]) {
        for (key, pointer) in fields {
            if let Some(value) = payload.pointer(pointer) {
                self.put(key, value.clone());
            }
        }
    }

    /// Top-level string fields of `payload`, as `text` keeps them.
    fn copy_text(&mut self, payload: &Value, keys: &[&str]) {
        for key in keys {
            if let Some(value) = payload.get(key).and_then(Value::as_str) {
                self.text(key, value);
            }
        }
    }

    fn chars(&mut self, key: &str, payload: &Value, pointer: &str) {
        if let Some(text) = payload.pointer(pointer).and_then(Value::as_str) {
            self.put(key, text.chars().count());
        }
    }
}

struct Run {
    checked: Instant,
    /// `Some(text)` while a valid run is active.
    active: Option<bool>,
}

/// A trace writer bound to one runtime directory.
pub struct Tracer {
    directory: Option<PathBuf>,
    run: Mutex<Option<Run>>,
}

impl Tracer {
    #[must_use]
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            run: Mutex::new(None),
        }
    }

    /// Appends `event` when a debug run is active. `build` runs only then.
    pub fn emit(&self, event: &str, build: impl FnOnce(&mut Entry)) {
        let Some(directory) = self.directory.as_deref() else {
            return;
        };
        let Some(text) = self.active(directory) else {
            return;
        };
        let mut entry = Entry {
            fields: Map::new(),
            text,
        };
        let at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        entry.put("t", u64::try_from(at.as_millis()).unwrap_or(u64::MAX));
        entry.put("src", "broker");
        entry.put("ev", event);
        build(&mut entry);
        let mut line = Value::Object(entry.fields).to_string();
        line.push('\n');
        append(directory, line.as_bytes());
    }

    fn active(&self, directory: &Path) -> Option<bool> {
        let Ok(mut run) = self.run.lock() else {
            return None;
        };
        let now = Instant::now();
        if let Some(run) = run.as_ref().filter(|run| now < run.checked + RECHECK) {
            return run.active;
        }
        let active = read_control(directory);
        *run = Some(Run {
            checked: now,
            active,
        });
        active
    }
}

/// `Some(text)` when the control names a valid, unexpired run.
fn read_control(directory: &Path) -> Option<bool> {
    let file = open_private(
        &directory.join(CONTROL_FILE),
        OFlags::RDONLY,
        MAX_CONTROL_BYTES,
    )?;
    let control: Value = serde_json::from_reader(file).ok()?;
    let expires_at = control.get("expires_at")?.as_u64()?;
    let id = control.get("id")?.as_str()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if expires_at <= now || expires_at > now + MAX_RUN_SECONDS || uuid::Uuid::parse_str(id).is_err()
    {
        return None;
    }
    Some(control.get("text").and_then(Value::as_bool) == Some(true))
}

/// Appends to an existing private trace below its size limit; never creates it.
fn append(directory: &Path, line: &[u8]) {
    if let Some(mut file) = open_private(
        &directory.join(TRACE_FILE),
        OFlags::WRONLY | OFlags::APPEND,
        MAX_TRACE_BYTES,
    ) {
        // One write per line: O_APPEND keeps concurrent writers' lines whole.
        let _ = file.write(line);
    }
}

/// An owned, private regular file no larger than `limit`, opened without
/// following a symlink.
fn open_private(path: &Path, flags: OFlags, limit: u64) -> Option<File> {
    let descriptor = rustix::fs::openat(
        CWD,
        path,
        flags | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .ok()?;
    let info = rustix::fs::fstat(&descriptor).ok()?;
    let mode = rustix::fs::FileType::from_raw_mode(info.st_mode);
    let owner = rustix::process::getuid().as_raw();
    let private = !Mode::from_raw_mode(info.st_mode).intersects(Mode::RWXG | Mode::RWXO);
    let size = u64::try_from(info.st_size).ok()?;
    (mode == rustix::fs::FileType::RegularFile && info.st_uid == owner && private && size <= limit)
        .then(|| File::from(descriptor))
}

#[cfg(not(test))]
static TRACER: LazyLock<Tracer> = LazyLock::new(|| {
    Tracer::new(
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|runtime| runtime.is_absolute())
            .map(|runtime| runtime.join("badi")),
    )
});

/// Unit tests never write into a live debug run on the developer's desktop.
#[cfg(test)]
static TRACER: LazyLock<Tracer> = LazyLock::new(|| Tracer::new(None));

/// Appends `event` to the desktop trace when a debug run is active.
pub fn emit(event: &str, build: impl FnOnce(&mut Entry)) {
    TRACER.emit(event, build);
}

/// The first eight characters of a session id: enough to follow one field.
#[must_use]
pub fn session(id: SessionId) -> String {
    id.to_string().chars().take(8).collect()
}

/// One wire frame, by allowlisted field. Health and statistics polls, which
/// the open panel sends every few seconds, are left out.
pub fn wire(direction: &str, adapter: AdapterKind, envelope: &WireEnvelope) {
    if matches!(
        envelope.message_type,
        MessageType::HealthRequest
            | MessageType::HealthStatus
            | MessageType::StatsQuery
            | MessageType::StatsStatus
    ) {
        return;
    }
    let name = serde_json::to_value(envelope.message_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    emit(&format!("{direction} {name}"), |entry| {
        describe(entry, adapter, envelope);
    });
}

fn describe(entry: &mut Entry, adapter: AdapterKind, envelope: &WireEnvelope) {
    if let Ok(adapter) = serde_json::to_value(adapter) {
        entry.put("adapter", adapter);
    }
    if let Some(session_id) = envelope.session_id {
        entry.put("sid", session(session_id));
    }
    if let Some(revision) = envelope.revision {
        entry.put("rev", revision);
    }
    let payload = &envelope.payload;
    match envelope.message_type {
        MessageType::ContextChanged => describe_context(entry, payload),
        MessageType::SuggestRequest => entry.copy(payload, &[("explicit", "/explicit")]),
        MessageType::SuggestCancel | MessageType::SessionClose => {
            entry.copy(payload, &[("reason", "/reason")]);
        }
        MessageType::SuggestionShow => {
            entry.copy(
                payload,
                &[
                    ("id", "/suggestion_id"),
                    ("ttl_ms", "/ttl_ms"),
                    ("provider", "/provider"),
                ],
            );
            entry.chars("chars", payload, "/text");
            entry.chars("word_chars", payload, "/accept_word");
            entry.copy_text(payload, &["text", "accept_word", "replace_before"]);
        }
        MessageType::SuggestionClear => {
            entry.copy(payload, &[("reason", "/reason"), ("id", "/suggestion_id")]);
        }
        MessageType::ControlRequest => {
            entry.copy(payload, &[("action", "/action"), ("id", "/suggestion_id")]);
        }
        MessageType::ControlResult => entry.copy(
            payload,
            &[
                ("action", "/action"),
                ("accepted", "/accepted"),
                ("reason", "/reason"),
                ("paused", "/paused"),
            ],
        ),
        MessageType::CommitPrepare => {
            entry.copy(
                payload,
                &[("id", "/suggestion_id"), ("acceptance", "/acceptance")],
            );
            entry.chars("chars", payload, "/text");
            entry.copy_text(payload, &["text", "replace_before"]);
        }
        MessageType::CommitResult => {
            entry.copy(payload, &[("id", "/suggestion_id"), ("status", "/status")]);
        }
        MessageType::SessionOpen => entry.copy(
            payload,
            &[
                ("kind", "/target/kind"),
                ("app", "/target/app_id"),
                ("host", "/target/origin/host"),
                ("activation", "/activation"),
            ],
        ),
        MessageType::Error => entry.copy(payload, &[("code", "/code")]),
        // Content-free desktop signals: session state, power and whether
        // the screen is shared, with the hold that results.
        MessageType::DesktopState | MessageType::DesktopStatus => {
            entry.put("state", payload.clone());
        }
        // Content-free editor focus: one boolean each way.
        MessageType::EditorFocus => entry.copy(payload, &[("focused", "/focused")]),
        MessageType::EditorStatus => {
            entry.copy(payload, &[("native_withheld", "/native_withheld")]);
        }
        // Settings, style keys and samples, memory, statistics and probes:
        // the type only.
        _ => {}
    }
}

/// Lengths always; the context itself only in a text run and never from a
/// field that is sensitive or not plain prose.
fn describe_context(entry: &mut Entry, payload: &Value) {
    entry.copy(
        payload,
        &[
            ("explicit", "/explicit"),
            ("language", "/language"),
            ("purpose", "/field/purpose"),
            ("multiline", "/field/multiline"),
        ],
    );
    entry.chars("before_chars", payload, "/before");
    entry.chars("after_chars", payload, "/after");
    // Only its length: the window's text stays out of every trace (ADR 0010).
    entry.chars("window_chars", payload, "/window_text");
    let sensitive = payload.pointer("/field/sensitive") != Some(&Value::Bool(false))
        || payload.pointer("/field/purpose").and_then(Value::as_str) != Some("normal");
    if entry.text && !sensitive {
        let text = |pointer| payload.pointer(pointer).and_then(Value::as_str);
        if let Some(before) = text("/before") {
            entry.put("before", tail(before, BEFORE_CHARS));
        }
        if let Some(after) = text("/after") {
            entry.put("after", head(after, AFTER_CHARS));
        }
    }
}

fn head(text: &str, limit: usize) -> String {
    text.chars().take(limit).collect()
}

fn tail(text: &str, limit: usize) -> String {
    let count = text.chars().count();
    text.chars().skip(count.saturating_sub(limit)).collect()
}

#[cfg(test)]
mod tests;
