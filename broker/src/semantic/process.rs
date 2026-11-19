//! Linux containment for an owned runtime: the calling executable re-executes
//! itself as a single-threaded helper that arms a parent-death signal and then
//! execs the verified runtime, so the runtime cannot outlive its owner. The
//! helper's arguments never come from the JSON request protocol.

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

pub(crate) fn launch_command(binary: &Path, cpu_mask: Option<&str>) -> io::Result<Command> {
    let mut command = Command::new(std::env::current_exe()?);
    command
        .arg(EXEC_HELPER_FLAG)
        .arg(std::process::id().to_string())
        .arg(binary);
    if let Some(mask) = cpu_mask {
        command.args(["--cpu-mask", mask]);
    }
    Ok(command)
}

/// Replaces this single-threaded helper with the already verified runtime.
/// The caller must dispatch here before constructing a Tokio runtime.
pub fn exec_runtime_helper(args: &[OsString]) -> Result<Infallible, &'static str> {
    let (parent, binary, cpu_mask) = match args {
        [parent, binary] => (parent, binary, None),
        [parent, binary, flag, mask] if flag == "--cpu-mask" => {
            let mask = mask
                .to_str()
                .filter(|value| super::runtime::valid_cpu_mask(value))
                .ok_or(HELPER_ERROR)?;
            (parent, binary, Some(mask))
        }
        _ => return Err(HELPER_ERROR),
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
    let mut runtime = Command::new(binary);
    if let Some(mask) = cpu_mask {
        runtime.args(["--cpu-mask", mask]);
    }
    let _ = runtime.exec();
    Err(HELPER_ERROR)
}

/// Arms `SIGKILL` for when the parent dies and confirms that the parent is
/// still `expected_parent`. Call it in the child before exec.
pub fn arm_parent_death(expected_parent: Pid) -> Result<(), &'static str> {
    set_parent_process_death_signal(Some(Signal::KILL)).map_err(|_| HELPER_ERROR)?;
    // Arming alone misses a parent that died earlier. Checking afterward also
    // covers reparenting before arming; a later death is handled by the kernel.
    if getppid() != Some(expected_parent) {
        return Err(HELPER_ERROR);
    }
    Ok(())
}

/// Accepts only a regular executable file that cannot gain privileges on
/// exec, which would clear the parent-death signal.
pub fn validate_unprivileged_executable(binary: &Path) -> Result<(), &'static str> {
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
mod tests;
