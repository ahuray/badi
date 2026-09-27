//! Embeds the source identity of the Rust inputs so installed binaries can be
//! matched to an install receipt. Missing Git metadata yields `unknown`.

use std::path::{Path, PathBuf};
use std::process::Command;

// The dirty check and the rebuild triggers cover the same inputs, and the Git
// watches below cover every HEAD movement, so a cached build cannot keep a
// stale commit or clean/dirty flag for these paths.
const INPUTS: [&str; 4] = ["broker", "evaluation/src", "Cargo.toml", "Cargo.lock"];

fn main() {
    let manifest = PathBuf::from(
        std::env::var_os("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"),
    );
    let root = manifest.parent().unwrap_or(&manifest).to_path_buf();
    println!("cargo:rerun-if-env-changed=BADI_BUILD_COMMIT");
    println!("cargo:rerun-if-env-changed=BADI_BUILD_DIRTY");
    for input in INPUTS {
        rerun_if_exists(&root.join(input));
    }

    let (commit, dirty) = match (
        std::env::var("BADI_BUILD_COMMIT"),
        std::env::var("BADI_BUILD_DIRTY"),
    ) {
        (Ok(commit), Ok(dirty)) => {
            assert!(
                valid_commit(&commit) || commit == "unknown",
                "BADI_BUILD_COMMIT must be a 40-character lowercase hex commit or unknown"
            );
            assert!(
                matches!(dirty.as_str(), "true" | "false" | "unknown")
                    && (commit != "unknown" || dirty == "unknown"),
                "BADI_BUILD_DIRTY must be true, false or unknown (unknown without a commit)"
            );
            (commit, dirty)
        }
        (Err(_), Err(_)) => git_identity(&root),
        _ => panic!("set both BADI_BUILD_COMMIT and BADI_BUILD_DIRTY, or neither"),
    };
    println!("cargo:rustc-env=BADI_BUILD_COMMIT={commit}");
    println!("cargo:rustc-env=BADI_BUILD_DIRTY={dirty}");
}

fn git_identity(root: &Path) -> (String, String) {
    // An exported tree nested inside another repository must not inherit
    // that repository's commit.
    let top = git(root, &["rev-parse", "--show-toplevel"])
        .and_then(|path| std::fs::canonicalize(path.trim()).ok());
    if top.is_none() || top != std::fs::canonicalize(root).ok() {
        return ("unknown".to_owned(), "unknown".to_owned());
    }
    let Some(commit) = git(root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .map(|output| output.trim().to_owned())
        .filter(|commit| valid_commit(commit))
    else {
        return ("unknown".to_owned(), "unknown".to_owned());
    };
    // A branch ref may be packed and reappear as a new loose file on commit,
    // so watch its directory (Cargo scans it recursively) and the HEAD reflog.
    // Reftable repositories rewrite their table list instead.
    for reference in ["HEAD", "packed-refs", "refs/heads", "logs/HEAD", "reftable"] {
        watch_git_path(root, reference);
    }
    if let Some(branch) = git(root, &["symbolic-ref", "-q", "HEAD"]) {
        watch_git_path(root, branch.trim());
    }
    let mut status = vec!["status", "--porcelain", "--untracked-files=normal", "--"];
    status.extend(INPUTS);
    let dirty = git(root, &status).map_or("unknown", |output| {
        if output.trim().is_empty() {
            "false"
        } else {
            "true"
        }
    });
    (commit, dirty.to_owned())
}

fn git(root: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout).ok()
}

fn watch_git_path(root: &Path, reference: &str) {
    if let Some(path) = git(root, &["rev-parse", "--git-path", reference]) {
        rerun_if_exists(&root.join(path.trim()));
    }
}

fn rerun_if_exists(path: &Path) {
    // Cargo reruns on every build for a missing path; watch only real inputs.
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn valid_commit(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
