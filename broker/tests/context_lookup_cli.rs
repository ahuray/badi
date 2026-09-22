#![cfg(all(feature = "writing-lab", target_os = "linux"))]

use std::error::Error;
use std::io::Write;
use std::process::{Child, Command, Output, Stdio};

struct Worker(Option<Child>);
impl Worker {
    fn spawn(args: &[&str]) -> std::io::Result<Self> {
        Command::new(env!("CARGO_BIN_EXE_badi-writing-lab"))
            .arg("--context-lookup")
            .args(args)
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map(|child| Self(Some(child)))
    }
    fn finish(mut self) -> std::io::Result<Output> {
        let child = self.0.as_mut().expect("owned child");
        child.stdin.take();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while child.try_wait()?.is_none() {
            if std::time::Instant::now() >= deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "lookup fixture deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        self.0.take().expect("owned child").wait_with_output()
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn frame() -> Vec<u8> {
    format!("{}\n", serde_json::json!({"schema":"badi.context-lookup.request.v1","id":"fixture","before":"Please use the sched","language":"en","context":"The schedule is ready.","style_examples":[]})).into_bytes()
}

#[test]
fn lookup_waits_for_one_snapshot_eof_without_starting_threads_or_children()
-> Result<(), Box<dyn Error>> {
    let mut worker = Worker::spawn(&[])?;
    let child = worker.0.as_mut().ok_or("missing child")?;
    child
        .stdin
        .as_mut()
        .ok_or("missing input")?
        .write_all(&frame())?;
    std::thread::sleep(std::time::Duration::from_millis(30));
    assert!(child.try_wait()?.is_none());
    let pid = child.id();
    assert_eq!(std::fs::read_dir(format!("/proc/{pid}/task"))?.count(), 1);
    assert!(
        std::fs::read_to_string(format!("/proc/{pid}/task/{pid}/children"))?
            .trim()
            .is_empty()
    );
    let output = worker.finish()?;
    assert!(output.status.success() && output.stderr.is_empty());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout)?;
    assert_eq!(result["schema"], "badi.context-lookup.result.v1");
    assert_eq!(result["contract_id"], "badi.context-lookup.exact.v1");
    assert_eq!(result["text"], "ule");
    assert_eq!(result["matched_word"], "schedule");
    assert_eq!(result["sources"], serde_json::json!(["context"]));
    assert!(result.get("identity").is_none() && result.get("replace_before").is_none());
    assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    Ok(())
}

#[test]
fn invalid_lookup_input_exits_nonzero_with_only_a_bounded_structured_error()
-> Result<(), Box<dyn Error>> {
    let extra_field = String::from_utf8(frame())?.replacen('{', "{\"expected\":\"private\",", 1);
    let duplicate = String::from_utf8(frame())?.replacen('{', "{\"id\":\"duplicate\",", 1);
    for (args, input, code) in [
        (vec!["extra"], frame(), "invalid_arguments"),
        (vec![], Vec::new(), "invalid_frame"),
        (vec![], b"{}".to_vec(), "invalid_frame"),
        (vec![], [frame(), frame()].concat(), "invalid_frame"),
        (vec![], vec![b'x'; 64 * 1024 + 1], "invalid_frame"),
        (vec![], b"\xff\n".to_vec(), "invalid_request"),
        (vec![], extra_field.into_bytes(), "invalid_request"),
        (vec![], duplicate.into_bytes(), "invalid_request"),
    ] {
        let mut worker = Worker::spawn(&args)?;
        let child = worker.0.as_mut().ok_or("missing child")?;
        if let Err(error) = child
            .stdin
            .as_mut()
            .ok_or("missing input")?
            .write_all(&input)
        {
            assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
        }
        let output = worker.finish()?;
        assert!(!output.status.success() && output.stderr.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        assert_eq!(
            value,
            serde_json::json!({"schema":"badi.context-lookup.error.v1","error":code})
        );
    }
    Ok(())
}
