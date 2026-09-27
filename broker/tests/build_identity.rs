//! Drives the real `build.rs` in a scratch checkout: the embedded identity must
//! follow HEAD even after the branch ref was packed, when a commit writes a
//! loose ref that did not exist while the build script chose what to watch.

use std::error::Error;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, arguments: &[&str]) -> Result<String, Box<dyn Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args([
            "-c",
            "user.name=Badi Test",
            "-c",
            "user.email=test@invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(arguments)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()?;
    if !output.status.success() {
        return Err(format!("git {arguments:?} failed").into());
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

fn embedded_identity(root: &Path, target: &Path) -> Result<String, Box<dyn Error>> {
    let status = Command::new(env!("CARGO"))
        .args(["build", "--quiet", "--offline", "--manifest-path"])
        .arg(root.join("broker/Cargo.toml"))
        .env("CARGO_TARGET_DIR", target)
        .env_remove("BADI_BUILD_COMMIT")
        .env_remove("BADI_BUILD_DIRTY")
        .status()?;
    if !status.success() {
        return Err("fixture build failed".into());
    }
    let output = Command::new(target.join("debug/identity-fixture")).output()?;
    Ok(String::from_utf8(output.stdout)?.trim().to_owned())
}

#[test]
fn embedded_identity_follows_commits_after_refs_are_packed() -> Result<(), Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path().join("checkout");
    let target = temporary.path().join("target");
    let crate_root = root.join("broker");
    fs::create_dir_all(crate_root.join("src"))?;
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("build.rs"),
        crate_root.join("build.rs"),
    )?;
    fs::write(
        crate_root.join("Cargo.toml"),
        "[package]\nname = \"identity-fixture\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n",
    )?;
    fs::write(
        crate_root.join("src/main.rs"),
        "fn main() {\n    println!(\"{} {}\", env!(\"BADI_BUILD_COMMIT\"), env!(\"BADI_BUILD_DIRTY\"));\n}\n",
    )?;
    fs::write(root.join(".gitignore"), "Cargo.lock\n")?;
    git(&root, &["init", "--quiet", "--initial-branch=develop"])?;
    git(&root, &["add", "--all"])?;
    git(&root, &["commit", "--quiet", "-m", "first"])?;
    let first = git(&root, &["rev-parse", "HEAD"])?;
    assert_eq!(embedded_identity(&root, &target)?, format!("{first} false"));

    git(&root, &["pack-refs", "--all", "--prune"])?;
    let loose = git(&root, &["rev-parse", "--git-path", "refs/heads/develop"])?;
    assert!(
        !root.join(loose).exists(),
        "the branch ref must now be packed"
    );
    assert_eq!(embedded_identity(&root, &target)?, format!("{first} false"));

    // Only the ref moves: no watched source file changes.
    git(
        &root,
        &["commit", "--quiet", "--allow-empty", "-m", "second"],
    )?;
    let second = git(&root, &["rev-parse", "HEAD"])?;
    assert_ne!(first, second);
    assert_eq!(
        embedded_identity(&root, &target)?,
        format!("{second} false")
    );

    fs::write(crate_root.join("src/extra.rs"), "")?;
    assert_eq!(embedded_identity(&root, &target)?, format!("{second} true"));
    Ok(())
}
