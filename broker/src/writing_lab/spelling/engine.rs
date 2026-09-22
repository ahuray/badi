use super::artifact::{Artifact, Identity};
use serde::Serialize;
use std::convert::Infallible;
use std::ffi::OsString;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

pub const HELPER_FLAG: &str = "__exec-owned-speller";
const FRAME_BYTES: usize = 64 * 1024;

#[derive(Debug, Eq, PartialEq)]
pub enum Response {
    Accepted,
    Suggestions(Vec<String>),
}

fn bounded_line(reader: &mut impl BufRead, remaining: usize) -> Result<String, &'static str> {
    let mut bytes = Vec::new();
    reader
        .take((remaining + 1) as u64)
        .read_until(b'\n', &mut bytes)
        .map_err(|_| "spelling_pipe_failed")?;
    if bytes.len() > remaining || bytes.last() != Some(&b'\n') {
        return Err("invalid_spelling_frame");
    }
    bytes.pop();
    String::from_utf8(bytes).map_err(|_| "invalid_spelling_frame")
}

pub(super) fn parse_response(line: &str, word: &str) -> Result<Response, &'static str> {
    if line == "*" {
        return Ok(Response::Accepted);
    }
    if line == format!("# {word} 1") {
        return Ok(Response::Suggestions(Vec::new()));
    }
    let (header, body) = line.split_once(": ").ok_or("invalid_spelling_frame")?;
    let fields: Vec<_> = header.split(' ').collect();
    if fields.len() != 4
        || fields[0] != "&"
        || fields[1] != word
        || fields[3] != "1"
        || fields[2].is_empty()
        || !fields[2].bytes().all(|b| b.is_ascii_digit())
    {
        return Err("invalid_spelling_frame");
    }
    let count: usize = fields[2].parse().map_err(|_| "invalid_spelling_frame")?;
    let candidates: Vec<_> = body.split(", ").map(str::to_owned).collect();
    if !(1..=128).contains(&count)
        || count != candidates.len()
        || candidates
            .iter()
            .any(|v| v.is_empty() || v.len() > 512 || v.chars().any(char::is_control))
    {
        return Err("invalid_spelling_frame");
    }
    Ok(Response::Suggestions(candidates))
}

struct Job {
    word: String,
    reply: oneshot::Sender<Result<Response, &'static str>>,
}

#[derive(Debug, Serialize)]
pub struct Cleanup {
    pub process_id: u32,
    pub identity_sha256: String,
    pub reaped: bool,
    pub exit_code: Option<i32>,
    pub forced: bool,
}

pub struct OwnedEngine {
    child: Option<Child>,
    jobs: Option<SyncSender<Job>>,
    artifact: Artifact,
    query_ms: f64,
    poisoned: bool,
}
impl OwnedEngine {
    pub async fn start(
        artifact: Artifact,
        cancellation: CancellationToken,
    ) -> Result<Self, &'static str> {
        // Spawn on the main block_on thread, which outlives the owned engine.
        // PDEATHSIG binds to the creating thread, not an arbitrary Tokio task.
        artifact.reverify()?;
        if cancellation.is_cancelled() {
            return Err("cancelled");
        }
        let mut command =
            Command::new(std::env::current_exe().map_err(|_| "spelling_start_failed")?);
        command
            .arg(HELPER_FLAG)
            .arg(std::process::id().to_string())
            .arg(&artifact.manifest_path)
            .arg(&artifact.identity.language)
            .env_clear()
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        Self::start_command(artifact, command, cancellation, true).await
    }

    async fn start_command(
        artifact: Artifact,
        mut command: Command,
        cancellation: CancellationToken,
        verify_library: bool,
    ) -> Result<Self, &'static str> {
        let mut engine = Self {
            child: Some(command.spawn().map_err(|_| "spelling_start_failed")?),
            jobs: None,
            artifact,
            query_ms: 0.0,
            poisoned: false,
        };
        // Own cleanup immediately after spawn, including missing pipes or a
        // failure to create the reader thread before readiness exists.
        let child = engine.child.as_mut().ok_or("spelling_start_failed")?;
        let input = child.stdin.take().ok_or("spelling_start_failed")?;
        let output = child.stdout.take().ok_or("spelling_start_failed")?;
        let (jobs, receiver) = sync_channel::<Job>(1);
        let (ready, readiness) = oneshot::channel();
        std::thread::Builder::new()
            .name("badi-spelling-pipe".to_owned())
            .spawn(move || {
                let mut input = input;
                let mut output = BufReader::new(output);
                let greeting = bounded_line(&mut output, 1024).and_then(|line| {
                    if line.starts_with("@(#)") && line.contains("Hunspell 1.7.3") {
                        Ok(())
                    } else {
                        Err("invalid_spelling_banner")
                    }
                });
                let valid = greeting.is_ok();
                if ready.send(greeting).is_err() || !valid {
                    return;
                }
                while let Ok(job) = receiver.recv() {
                    let result = (|| {
                        input
                            .write_all(format!("^{}\n", job.word).as_bytes())
                            .map_err(|_| "spelling_pipe_failed")?;
                        input.flush().map_err(|_| "spelling_pipe_failed")?;
                        let line = bounded_line(&mut output, FRAME_BYTES)?;
                        let terminator = bounded_line(&mut output, FRAME_BYTES - line.len() - 1)?;
                        if !terminator.is_empty() {
                            return Err("invalid_spelling_frame");
                        }
                        parse_response(&line, &job.word)
                    })();
                    let failed = result.is_err();
                    if job.reply.send(result).is_err() || failed {
                        break;
                    }
                }
            })
            .map_err(|_| "spelling_start_failed")?;
        engine.jobs = Some(jobs);
        let ready = tokio::select! { biased;
            () = cancellation.cancelled() => Err("cancelled"),
            result = tokio::time::timeout(Duration::from_secs(3),readiness) =>
                result.map_err(|_|"spelling_start_deadline").and_then(|v|v.map_err(|_|"spelling_start_failed")).and_then(|v|v)
        };
        ready?;
        engine.artifact.reverify()?;
        if verify_library {
            super::identity::verify(
                engine.process_id(),
                &engine.artifact.binary,
                &engine.artifact.library,
            )?;
        }
        if engine
            .child
            .as_mut()
            .ok_or("spelling_start_failed")?
            .try_wait()
            .map_err(|_| "spelling_start_failed")?
            .is_some()
        {
            return Err("spelling_start_failed");
        }
        Ok(engine)
    }
    #[must_use]
    pub fn identity(&self) -> &Identity {
        &self.artifact.identity
    }
    #[must_use]
    pub fn process_id(&self) -> u32 {
        self.child.as_ref().map_or(0, Child::id)
    }
    pub fn take_query_ms(&mut self) -> f64 {
        std::mem::take(&mut self.query_ms)
    }
    pub async fn query(
        &mut self,
        word: &str,
        deadline: Instant,
        cancellation: CancellationToken,
    ) -> Result<Response, &'static str> {
        let started = Instant::now();
        if self.poisoned {
            return Err("spelling_stream_retired");
        }
        if cancellation.is_cancelled() {
            return Err("cancelled");
        }
        if !super::policy::word_valid(word, &self.identity().language) || word.chars().count() > 24
        {
            return Err("invalid_request");
        }
        self.artifact.reverify()?;
        if Instant::now() >= deadline {
            self.poisoned = true;
            return Err("query_deadline");
        }
        let (reply, response) = oneshot::channel();
        self.jobs
            .as_ref()
            .ok_or("spelling_stream_retired")?
            .try_send(Job {
                word: word.to_owned(),
                reply,
            })
            .map_err(|_| "spelling_pipe_failed")?;
        let result = tokio::select! { biased;
            () = cancellation.cancelled() => Err("cancelled"),
            () = tokio::time::sleep_until(deadline.into()) => Err("query_deadline"),
            result = response => result.map_err(|_|"spelling_pipe_failed").and_then(|v|v)
        };
        self.query_ms += started.elapsed().as_secs_f64() * 1000.0;
        if result.is_err() {
            self.poisoned = true;
        }
        result
    }
    pub fn shutdown(&mut self) -> Result<Cleanup, &'static str> {
        self.jobs.take();
        let child = self.child.as_mut().ok_or("spelling_cleanup_failed")?;
        let pid = child.id();
        let mut forced = false;
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().map_err(|_| "spelling_cleanup_failed")? {
                self.child.take();
                return Ok(Cleanup {
                    process_id: pid,
                    identity_sha256: self.identity().sha256(),
                    reaped: true,
                    exit_code: status.code(),
                    forced,
                });
            }
            if started.elapsed() >= Duration::from_millis(250) && !forced {
                child.kill().map_err(|_| "spelling_cleanup_failed")?;
                forced = true;
            }
            if started.elapsed() >= Duration::from_millis(1500) {
                return Err("spelling_cleanup_failed");
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
impl Drop for OwnedEngine {
    fn drop(&mut self) {
        if self.child.is_some() {
            let _ = self.shutdown();
        }
    }
}

/// Dispatch before Tokio or any other threads are constructed.
pub fn exec_helper(args: &[OsString]) -> Result<Infallible, &'static str> {
    use rustix::process::Pid;
    let [parent, path, language] = args else {
        return Err("spelling_helper_failed");
    };
    let parent = parent
        .to_str()
        .and_then(|v| v.parse::<i32>().ok())
        .filter(|p| *p > 1)
        .and_then(Pid::from_raw)
        .ok_or("spelling_helper_failed")?;
    crate::writing_lab::process::arm_parent_death(parent).map_err(|_| "spelling_helper_failed")?;
    let artifact = Artifact::read(
        std::path::Path::new(path),
        language.to_str().ok_or("spelling_helper_failed")?,
    )?;
    crate::writing_lab::process::validate_unprivileged_executable(artifact.binary.path())
        .map_err(|_| "spelling_helper_failed")?;
    let _ = Command::new(artifact.binary.path())
        .env_clear()
        .env("LC_ALL", "C.UTF-8")
        .args(["-a", "-i", "UTF-8", "-d"])
        .arg(artifact.aff.path().with_extension(""))
        .args(["-p", "/dev/null"])
        .exec();
    Err("spelling_helper_failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn fixture(script: &str) -> (OwnedEngine, tempfile::TempDir) {
        let directory = tempfile::tempdir().unwrap();
        let artifact = Artifact::fixture(&directory.path().join("artifact"));
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", script])
            .env_clear()
            .env("LC_ALL", "C.UTF-8")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let engine = OwnedEngine::start_command(artifact, command, CancellationToken::new(), false)
            .await
            .unwrap();
        (engine, directory)
    }
    const NORMAL: &str = "test -z \"${HOME+x}\" || exit 1\nprintf '%s\\n' '@(#) Hunspell 1.7.3 fixture'\nwhile IFS= read -r badi_word; do\ncase \"$badi_word\" in\n'^Fahrad') printf '%s\\n\\n' '& Fahrad 1 1: Fahrrad';;\n'^Fahrrad') printf '*\\n\\n';;\n'^Hase') printf '%s\\n\\n' '& Hase 2 1: Hose, Haxe';;\n*) printf '%s\\n\\n' '# unknown 1';;\nesac\ndone";
    fn request(before: &str) -> super::super::Request {
        super::super::Request {
            schema: super::super::REQUEST_SCHEMA.to_owned(),
            id: "fixture".to_owned(),
            before: before.to_owned(),
            language: "de".to_owned(),
            protected_words: Vec::new(),
        }
    }
    #[tokio::test]
    async fn persistent_native_pipe_proposes_only_confirmed_unambiguous_corrections_and_reaps_on_eof()
     {
        let (mut engine, _directory) = fixture(NORMAL).await;
        let pid = engine.process_id();
        let result = super::super::check(
            &mut engine,
            request("Das Fahrad "),
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(result.outcome, "suggestion");
        assert_eq!(result.text.as_deref(), Some("Fahrrad "));
        assert_eq!(result.replace_before.as_deref(), Some("Fahrad "));
        assert_eq!(
            (result.query_count, result.filtered_candidate_count),
            (2, 1)
        );
        assert_eq!(engine.process_id(), pid);
        assert!(result.latency_ms >= result.query_ms);
        for (before, reason, queries) in [
            ("Das Fahrrad ", "dictionary_accepts_original", 1),
            ("Der Hase ", "ambiguous_candidates", 1),
            ("Das Fahrad", "no_completed_word", 0),
        ] {
            let result =
                super::super::check(&mut engine, request(before), CancellationToken::new())
                    .await
                    .unwrap();
            assert_eq!(
                (result.outcome, result.reason, result.query_count),
                ("abstention", reason, queries)
            );
            assert!(result.text.is_none() && result.replace_before.is_none());
        }
        let mut protected = request("Das Fahrad ");
        protected.protected_words.push("Fahrad".to_owned());
        let result = super::super::check(&mut engine, protected, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!((result.reason, result.query_count), ("protected_word", 0));
        let receipt = engine.shutdown().unwrap();
        assert!(receipt.reaped && !receipt.forced);
        assert_eq!(receipt.exit_code, Some(0));
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    #[tokio::test]
    async fn deadline_and_cancellation_retire_the_pipe_and_reap_the_owned_child() {
        let script = "printf '%s\\n' '@(#) Hunspell 1.7.3 fixture'\nIFS= read -r badi_word\nIFS= read -r badi_stall";
        for cancel in [false, true] {
            let (mut engine, _directory) = fixture(script).await;
            let pid = engine.process_id();
            let cancellation = CancellationToken::new();
            let trigger = cancellation.clone();
            let query = engine.query(
                "Fahrad",
                Instant::now() + Duration::from_millis(40),
                cancellation,
            );
            let interrupt = async move {
                if cancel {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    trigger.cancel();
                }
            };
            let (result, ()) = tokio::join!(query, interrupt);
            assert_eq!(
                result,
                Err(if cancel {
                    "cancelled"
                } else {
                    "query_deadline"
                })
            );
            assert_eq!(
                engine
                    .query(
                        "Fahrad",
                        Instant::now() + Duration::from_secs(1),
                        CancellationToken::new()
                    )
                    .await,
                Err("spelling_stream_retired")
            );
            let receipt = engine.shutdown().unwrap();
            assert!(receipt.reaped);
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        }
    }
    #[tokio::test]
    async fn malformed_multiword_and_early_eof_responses_never_leave_a_reusable_engine() {
        for reply in ["printf '*\\n*\\n\\n'", "exit 0"] {
            let script = format!(
                "printf '%s\\n' '@(#) Hunspell 1.7.3 fixture'\nIFS= read -r badi_word\n{reply}"
            );
            let (mut engine, _directory) = fixture(&script).await;
            let pid = engine.process_id();
            assert!(
                engine
                    .query(
                        "Fahrad",
                        Instant::now() + Duration::from_millis(100),
                        CancellationToken::new()
                    )
                    .await
                    .is_err()
            );
            drop(engine);
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        }
    }
    #[tokio::test]
    async fn a_failed_banner_reaps_before_start_returns() {
        let directory = tempfile::tempdir().unwrap();
        let artifact = Artifact::fixture(&directory.path().join("artifact"));
        let pid_file = directory.path().join("pid");
        let mut command = Command::new("/bin/sh");
        command.args(["-c","printf '%s' \"$$\" > \"$BADI_SPELLING_PID_PATH\"; printf 'bad banner\\n'; IFS= read -r badi_stall"])
            .env_clear().env("BADI_SPELLING_PID_PATH",&pid_file)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
        assert!(
            OwnedEngine::start_command(artifact, command, CancellationToken::new(), false)
                .await
                .is_err()
        );
        let pid = std::fs::read_to_string(pid_file).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }
    #[tokio::test]
    async fn cancellation_and_identity_failure_before_ready_reap_the_child() {
        for cancel in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let artifact = Artifact::fixture(&directory.path().join("artifact"));
            let pid_file = directory.path().join("pid");
            let mut command = Command::new("/bin/sh");
            command.args(["-c", if cancel {
                "printf '%s' \"$$\" > \"$BADI_SPELLING_PID_PATH\"; IFS= read -r badi_stall"
            } else {
                "printf '%s' \"$$\" > \"$BADI_SPELLING_PID_PATH\"; printf '%s\\n' '@(#) Hunspell 1.7.3 fixture'; IFS= read -r badi_stall"
            }]).env_clear().env("BADI_SPELLING_PID_PATH", &pid_file)
                .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
            let cancellation = CancellationToken::new();
            let trigger = cancellation.clone();
            let interrupt = async {
                if cancel {
                    let deadline = Instant::now() + Duration::from_secs(2);
                    while !pid_file.exists() {
                        assert!(Instant::now() < deadline);
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                    trigger.cancel();
                }
            };
            let start = OwnedEngine::start_command(artifact, command, cancellation, true);
            let (result, ()) = tokio::join!(start, interrupt);
            assert!(matches!(
                result,
                Err("cancelled" | "spelling_identity_unverified")
            ));
            let pid = std::fs::read_to_string(pid_file).unwrap();
            assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        }
    }
    #[tokio::test]
    async fn protecting_originals_never_removes_a_competing_correction() {
        let (mut engine, _directory) = fixture(NORMAL).await;
        let mut ambiguous = request("Der Hase ");
        ambiguous.protected_words.push("Hose".to_owned());
        let result = super::super::check(&mut engine, ambiguous, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.reason, "ambiguous_candidates");
        assert_eq!(result.filtered_candidate_count, 2);
        let mut unique = request("Das Fahrad ");
        unique.protected_words.push("Fahrrad".to_owned());
        let result = super::super::check(&mut engine, unique, CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.text.as_deref(), Some("Fahrrad "));
        engine.shutdown().unwrap();
    }
    #[test]
    fn strict_pipe_frames_preserve_identity_counts_and_offsets() {
        assert_eq!(parse_response("*", "Fahrad"), Ok(Response::Accepted));
        assert_eq!(
            parse_response("# Fahrad 1", "Fahrad"),
            Ok(Response::Suggestions(vec![]))
        );
        assert_eq!(
            parse_response("& Fahrad 2 1: Fahrrad, Fahrrat", "Fahrad"),
            Ok(Response::Suggestions(vec![
                "Fahrrad".into(),
                "Fahrrat".into()
            ]))
        );
        for line in [
            "& other 1 1: Fahrrad",
            "& Fahrad 1 0: Fahrrad",
            "& Fahrad 2 1: Fahrrad",
            "& Fahrad 1 1: ",
            "* extra",
            "# Fahrad 0",
            "+ root",
            "-",
        ] {
            assert!(parse_response(line, "Fahrad").is_err());
        }
    }
    #[test]
    fn bounded_reader_refuses_unterminated_oversized_or_invalid_utf8() {
        for bytes in [
            b"word".as_slice(),
            b"abcdef\n".as_slice(),
            b"\xff\n".as_slice(),
        ] {
            assert!(bounded_line(&mut std::io::Cursor::new(bytes), 5).is_err());
        }
        assert_eq!(
            bounded_line(&mut std::io::Cursor::new(b"\n"), 1),
            Ok(String::new())
        );
    }
}
