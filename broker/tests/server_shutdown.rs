#![cfg(target_os = "linux")]

use std::io::Read as _;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, Signal, kill_process};

const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[test]
fn sigterm_exits_cleanly_and_removes_the_bound_socket() {
    assert_graceful_shutdown(Signal::TERM);
}

#[test]
fn sigint_exits_cleanly_and_removes_the_bound_socket() {
    assert_graceful_shutdown(Signal::INT);
}

fn assert_graceful_shutdown(signal: Signal) {
    let directory = tempfile::tempdir().expect("temporary directory");
    let socket_path = directory.path().join("private").join("broker.sock");
    let child = Command::new(env!("CARGO_BIN_EXE_badi-broker"))
        .arg("--socket")
        .arg(&socket_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("broker process");
    let mut child = ChildGuard::new(child);

    wait_for_socket(&mut child.process, &socket_path);
    let pid = Pid::from_child(&child.process);
    kill_process(pid, signal).expect("send shutdown signal");
    wait_for_socket_removal(&socket_path);
    let status = wait_for_exit(&mut child.process);
    assert!(status.success());

    let mut stdout = Vec::new();
    child
        .process
        .stdout
        .take()
        .expect("captured stdout")
        .read_to_end(&mut stdout)
        .expect("read stdout");
    let mut stderr = Vec::new();
    child
        .process
        .stderr
        .take()
        .expect("captured stderr")
        .read_to_end(&mut stderr)
        .expect("read stderr");
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

fn wait_for_socket(child: &mut Child, socket_path: &Path) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if socket_path.exists() {
            return;
        }
        assert!(
            child.try_wait().expect("broker status").is_none(),
            "broker exited before binding"
        );
        assert!(Instant::now() < deadline, "broker did not bind in time");
        thread::sleep(POLL_INTERVAL);
    }
}

fn wait_for_socket_removal(socket_path: &Path) {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    while socket_path.exists() && Instant::now() < deadline {
        thread::sleep(POLL_INTERVAL);
    }
    assert!(
        !socket_path.exists(),
        "broker socket remained after shutdown signal"
    );
}

fn wait_for_exit(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + PROCESS_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait().expect("broker status") {
            return status;
        }
        assert!(Instant::now() < deadline, "broker did not exit in time");
        thread::sleep(POLL_INTERVAL);
    }
}

struct ChildGuard {
    process: Child,
}

impl ChildGuard {
    const fn new(process: Child) -> Self {
        Self { process }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if matches!(self.process.try_wait(), Ok(None)) {
            let _ = self.process.kill();
            let _ = self.process.wait();
        }
    }
}
