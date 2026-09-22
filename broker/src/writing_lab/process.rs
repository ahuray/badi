//! Linux containment for the private Lab runtime. No normal broker launch uses
//! this helper, and its arguments never come from the JSON request protocol.

use std::convert::Infallible;
use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use rustix::io::Errno;
use rustix::process::{Pid, Signal, getppid, set_parent_process_death_signal};

pub const EXEC_HELPER_FLAG: &str = "__exec-owned-runtime";
const HELPER_ERROR: &str = "runtime_helper_failed";

pub(crate) fn launch_command(binary: &Path) -> io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(EXEC_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(binary);
    Ok(command)
}

/// Replaces this single-threaded helper with the already verified runtime.
/// The caller must dispatch here before constructing a Tokio runtime.
pub fn exec_runtime_helper(args: &[OsString]) -> Result<Infallible, &'static str> {
    let [parent, binary] = args else {
        return Err(HELPER_ERROR);
    };
    let parent = parent
        .to_str()
        .and_then(|value| value.parse::<i32>().ok())
        .filter(|value| *value > 1)
        .and_then(Pid::from_raw)
        .ok_or(HELPER_ERROR)?;
    let binary = Path::new(binary);
    if !binary.is_absolute() {
        return Err(HELPER_ERROR);
    }
    arm_parent_death(parent)?;
    validate_unprivileged_executable(binary)?;
    // exec preserves the process ID, group, cleared runtime environment and
    // death signal. Never fork another child or change credentials here.
    let _ = Command::new(binary).exec();
    Err(HELPER_ERROR)
}

pub(crate) fn arm_parent_death(expected_parent: Pid) -> Result<(), &'static str> {
    set_parent_process_death_signal(Some(Signal::KILL)).map_err(|_| HELPER_ERROR)?;
    // Arming alone misses a parent that died earlier. Checking afterward also
    // covers reparenting before arming; a later death is handled by the kernel.
    if getppid() != Some(expected_parent) {
        return Err(HELPER_ERROR);
    }
    Ok(())
}

pub(crate) fn validate_unprivileged_executable(binary: &Path) -> Result<(), &'static str> {
    let leaf = std::fs::symlink_metadata(binary).map_err(|_| HELPER_ERROR)?;
    if !leaf.is_file() {
        return Err(HELPER_ERROR);
    }
    let file = File::open(binary).map_err(|_| HELPER_ERROR)?;
    let metadata = file.metadata().map_err(|_| HELPER_ERROR)?;
    if !metadata.is_file()
        || metadata.mode() & 0o6000 != 0
        || metadata.mode() & 0o111 == 0
        || metadata.dev() != leaf.dev()
        || metadata.ino() != leaf.ino()
    {
        return Err(HELPER_ERROR);
    }
    // Privileged exec can clear PDEATHSIG even when the file digest is still
    // valid. Only a confirmed missing capability xattr authorizes this launch;
    // inaccessible/unsupported inspection is not proof of absence.
    let mut capabilities = [0_u8; 64];
    require_no_file_capabilities(rustix::fs::fgetxattr(
        &file,
        "security.capability",
        &mut capabilities,
    ))
}

fn require_no_file_capabilities(result: rustix::io::Result<usize>) -> Result<(), &'static str> {
    if matches!(result, Err(Errno::NODATA)) {
        Ok(())
    } else {
        Err(HELPER_ERROR)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::CommandExt;
    use std::path::{Path, PathBuf};
    use std::process::{Child, Command, Stdio};
    use std::thread;
    use std::time::{Duration, Instant};

    use rustix::io::Errno;
    use rustix::process::{Pid, Signal, kill_process_group};

    use super::{arm_parent_death, exec_runtime_helper, require_no_file_capabilities};

    const FIXTURE: &str = "writing_lab::process::tests::lifecycle_fixture";
    const DIRECTORY: &str = "BADI_LAB_LIFECYCLE_DIRECTORY";
    const ROLE: &str = "BADI_LAB_LIFECYCLE_ROLE";
    const STAGE: &str = "BADI_LAB_LIFECYCLE_STAGE";
    const PARENT: &str = "BADI_LAB_LIFECYCLE_PARENT";
    const LIMIT: Duration = Duration::from_secs(5);

    fn wait_until(mut condition: impl FnMut() -> bool) {
        let deadline = Instant::now() + LIMIT;
        while !condition() {
            assert!(Instant::now() < deadline, "process fixture timed out");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn command(directory: &Path, role: &str, stage: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--ignored", "--exact", FIXTURE])
            .env(DIRECTORY, directory)
            .env(ROLE, role)
            .env(STAGE, stage)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0);
        command
    }

    fn process_state(pid: u32) -> std::io::Result<Option<(String, String)>> {
        let stat = match fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => stat,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let invalid = || std::io::Error::other("invalid process fixture identity");
        let (_, tail) = stat.rsplit_once(')').ok_or_else(invalid)?;
        let fields: Vec<_> = tail.split_whitespace().collect();
        Ok(Some((
            fields.get(19).ok_or_else(invalid)?.to_string(),
            fields.first().ok_or_else(invalid)?.to_string(),
        )))
    }

    struct ProcessTree {
        parent: Child,
        child: Option<(u32, String)>,
    }

    impl Drop for ProcessTree {
        fn drop(&mut self) {
            if let Some((pid, start)) = &self.child {
                if process_state(*pid)
                    .ok()
                    .flatten()
                    .is_some_and(|(current, _)| current == *start)
                {
                    let pid = Pid::from_raw(i32::try_from(*pid).unwrap()).unwrap();
                    let _ = kill_process_group(pid, Signal::KILL);
                }
            }
            let _ = self.parent.kill();
            let _ = self.parent.wait();
        }
    }

    // The fixture is selected explicitly in disposable child processes. It is
    // never part of the production helper's arguments or environment contract.
    #[test]
    #[ignore = "subprocess fixture selected by the lifecycle regression"]
    fn lifecycle_fixture() -> Result<(), Box<dyn Error>> {
        let directory = PathBuf::from(std::env::var(DIRECTORY)?);
        let stage = std::env::var(STAGE)?;
        if std::env::var(ROLE)? == "parent" {
            let mut child = command(&directory, "child", &stage)
                .env(PARENT, std::process::id().to_string())
                .spawn()?;
            fs::write(directory.join("spawned"), child.id().to_string())?;
            // A failing test cannot leave an unbounded supervisor behind.
            let deadline = Instant::now() + LIMIT + LIMIT;
            while Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        let expected = std::env::var(PARENT)?.parse::<i32>()?;
        let parent = Pid::from_raw(expected).unwrap();
        match stage.as_str() {
            "before_arm" => {
                fs::write(directory.join("ready"), b"before_arm")?;
                wait_until(|| directory.join("continue").exists());
                assert_eq!(arm_parent_death(parent), Err(super::HELPER_ERROR));
                fs::write(directory.join("refused"), b"parent_gone")?;
            }
            "before_exec" => {
                arm_parent_death(parent)?;
                fs::write(directory.join("ready"), b"before_exec")?;
                wait_until(|| directory.join("continue").exists());
                panic!("armed child survived parent death");
            }
            "after_exec" => {
                let result = exec_runtime_helper(&[
                    expected.to_string().into(),
                    directory.join("llama-server").into_os_string(),
                ]);
                panic!("exec fixture returned: {result:?}");
            }
            _ => panic!("unknown fixture stage"),
        }
        Ok(())
    }

    #[test]
    fn runtime_cannot_survive_parent_death_before_arming_before_exec_or_after_exec()
    -> Result<(), Box<dyn Error>> {
        for stage in ["before_arm", "before_exec", "after_exec"] {
            let temporary = tempfile::tempdir()?;
            let directory = temporary.path();
            let target = directory.join("llama-server");
            // POSIX shell builtins only: no descendant process escapes the
            // fixture. SIGSTOP keeps the exec'd process observable until kill.
            fs::write(
                &target,
                b"#!/bin/sh\nprintf after_exec > \"$BADI_LAB_LIFECYCLE_DIRECTORY/ready\"\nkill -STOP \"$$\"\nexit 0\n",
            )?;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o700))?;
            let mut tree = ProcessTree {
                parent: command(directory, "parent", stage).spawn()?,
                child: None,
            };
            wait_until(|| {
                fs::read_to_string(directory.join("spawned"))
                    .is_ok_and(|text| text.parse::<u32>().is_ok())
            });
            let pid: u32 = fs::read_to_string(directory.join("spawned"))?.parse()?;
            let (start, _) = process_state(pid)?.ok_or("missing spawned child")?;
            tree.child = Some((pid, start.clone()));
            wait_until(|| directory.join("ready").exists());
            tree.parent.kill()?;
            tree.parent.wait()?;
            if stage == "before_arm" {
                fs::write(directory.join("continue"), b"parent_is_dead")?;
                wait_until(|| directory.join("refused").exists());
            }
            wait_until(|| {
                process_state(pid).unwrap().is_none_or(|(current, state)| {
                    current != start || matches!(state.as_str(), "Z" | "X")
                })
            });
        }
        Ok(())
    }

    #[test]
    fn privileged_or_uninspectable_runtime_cannot_clear_containment() -> Result<(), Box<dyn Error>>
    {
        let temporary = tempfile::tempdir()?;
        let target = temporary.path().join("llama-server");
        fs::write(&target, b"disposable fixture")?;
        for mode in [0o4700, 0o2700, 0o600] {
            fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
            assert_eq!(
                super::validate_unprivileged_executable(&target),
                Err(super::HELPER_ERROR)
            );
        }
        assert!(require_no_file_capabilities(Err(Errno::NODATA)).is_ok());
        for observed in [Ok(0), Ok(24), Err(Errno::ACCESS), Err(Errno::NOTSUP)] {
            assert_eq!(
                require_no_file_capabilities(observed),
                Err(super::HELPER_ERROR)
            );
        }
        Ok(())
    }
}
