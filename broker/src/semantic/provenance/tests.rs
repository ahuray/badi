use std::error::Error;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use rustix::fs::{CWD, Mode, mkfifoat};
use sha2::Digest as _;
use tempfile::TempDir;

use super::{
    DirectoryManifestExpectation, FileExpectation, ProvenanceError, directory_manifest_sha256,
    verify_directory_manifest, verify_file,
};

struct TestDirectory {
    _temporary: TempDir,
    path: PathBuf,
}

impl TestDirectory {
    fn new() -> Result<Self, Box<dyn Error>> {
        let temporary = tempfile::tempdir()?;
        let path = fs::canonicalize(temporary.path())?;
        Ok(Self {
            _temporary: temporary,
            path,
        })
    }

    fn expectation(&self) -> Result<DirectoryManifestExpectation, Box<dyn Error>> {
        Ok(DirectoryManifestExpectation::new(
            &self.path,
            directory_manifest_sha256(&self.path)?,
        )?)
    }
}

#[test]
fn directory_manifest_is_sorted_and_accepts_safe_link_chains() -> Result<(), Box<dyn Error>> {
    let first = TestDirectory::new()?;
    fs::write(first.path.join("z-file"), b"z")?;
    fs::write(first.path.join("a-file"), b"a")?;
    symlink("z-link", first.path.join("top-link"))?;
    symlink("z-file", first.path.join("z-link"))?;

    let second = TestDirectory::new()?;
    symlink("z-file", second.path.join("z-link"))?;
    symlink("z-link", second.path.join("top-link"))?;
    fs::write(second.path.join("a-file"), b"a")?;
    fs::write(second.path.join("z-file"), b"z")?;

    let first_digest = directory_manifest_sha256(&first.path)?;
    let second_digest = directory_manifest_sha256(&second.path)?;
    assert_eq!(first_digest, second_digest);
    let verified = verify_directory_manifest(&DirectoryManifestExpectation::new(
        &first.path,
        first_digest,
    )?)?;
    assert_eq!(verified.path(), first.path);
    Ok(())
}

#[test]
fn directory_manifest_rejects_modified_extra_and_missing_files() -> Result<(), Box<dyn Error>> {
    for mutation in ["modified", "extra", "missing"] {
        let directory = TestDirectory::new()?;
        let artifact = directory.path.join("artifact.so");
        fs::write(&artifact, b"reviewed")?;
        let expectation = directory.expectation()?;
        match mutation {
            "modified" => fs::write(&artifact, b"tampered")?,
            "extra" => fs::write(directory.path.join("extra.so"), b"extra")?,
            "missing" => fs::remove_file(&artifact)?,
            _ => unreachable!(),
        }
        assert!(matches!(
            verify_directory_manifest(&expectation),
            Err(ProvenanceError::DirectoryManifestDigestMismatch)
        ));
    }
    Ok(())
}

#[test]
fn directory_manifest_rejects_unsafe_missing_and_cyclic_links() -> Result<(), Box<dyn Error>> {
    for (name, target, expected) in [
        ("absolute", "/etc/passwd", "unsafe"),
        ("parent", "../outside", "unsafe"),
        ("missing", "absent.so", "missing"),
        ("cycle-a", "cycle-b", "cycle"),
    ] {
        let directory = TestDirectory::new()?;
        if name == "cycle-a" {
            symlink("cycle-a", directory.path.join("cycle-b"))?;
        }
        symlink(target, directory.path.join(name))?;
        let result = directory_manifest_sha256(&directory.path);
        match expected {
            "unsafe" => assert!(matches!(
                result,
                Err(ProvenanceError::UnsafeSymlinkTarget(_))
            )),
            "missing" => assert!(matches!(
                result,
                Err(ProvenanceError::MissingSymlinkTarget(_))
            )),
            "cycle" => assert!(matches!(result, Err(ProvenanceError::CyclicSymlink(_)))),
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn directory_manifest_rejects_subdirectories_and_fifos() -> Result<(), Box<dyn Error>> {
    let nested = TestDirectory::new()?;
    fs::create_dir(nested.path.join("nested"))?;
    assert!(matches!(
        directory_manifest_sha256(&nested.path),
        Err(ProvenanceError::UnsupportedDirectoryEntry(entry)) if entry == "nested"
    ));

    let fifo = TestDirectory::new()?;
    mkfifoat(CWD, fifo.path.join("runtime.pipe"), Mode::RUSR | Mode::WUSR)?;
    assert!(matches!(
        directory_manifest_sha256(&fifo.path),
        Err(ProvenanceError::UnsupportedDirectoryEntry(entry)) if entry == "runtime.pipe"
    ));
    Ok(())
}

#[test]
fn directory_manifest_reverify_detects_same_content_replacement() -> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new()?;
    let artifact = directory.path.join("artifact.so");
    fs::write(&artifact, b"reviewed")?;
    let verified = verify_directory_manifest(&directory.expectation()?)?;
    let displaced = directory.path.join("displaced.so");
    fs::rename(&artifact, &displaced)?;
    fs::write(&artifact, b"reviewed")?;
    fs::remove_file(displaced)?;
    assert!(matches!(
        verified.reverify(),
        Err(ProvenanceError::IdentityChanged)
    ));
    Ok(())
}

#[test]
fn unchanged_confirmation_detects_replacement_growth_and_links_without_rehashing()
-> Result<(), Box<dyn Error>> {
    let directory = TestDirectory::new()?;
    let path = directory.path.join("model.gguf");
    fs::write(&path, b"weights")?;
    let expectation = FileExpectation::new(&path, sha256_of(b"weights"), 7)?;
    let verified = verify_file(&expectation)?;
    verified.confirm_unchanged()?;

    let replacement = directory.path.join("replacement.gguf");
    fs::write(&replacement, b"weights")?;
    fs::rename(&replacement, &path)?;
    assert!(matches!(
        verified.confirm_unchanged(),
        Err(ProvenanceError::IdentityChanged)
    ));

    let verified = verify_file(&expectation)?;
    fs::OpenOptions::new()
        .append(true)
        .open(&path)?
        .write_all(b"!")?;
    assert!(matches!(
        verified.confirm_unchanged(),
        Err(ProvenanceError::IdentityChanged)
    ));

    fs::write(&path, b"weights")?;
    let verified = verify_file(&expectation)?;
    let target = directory.path.join("target.gguf");
    fs::rename(&path, &target)?;
    symlink(&target, &path)?;
    assert!(matches!(
        verified.confirm_unchanged(),
        Err(ProvenanceError::Symlink)
    ));
    Ok(())
}

#[test]
fn unchanged_bundle_confirmation_detects_entry_replacement_and_additions()
-> Result<(), Box<dyn Error>> {
    for mutation in ["replaced", "extra"] {
        let directory = TestDirectory::new()?;
        let artifact = directory.path.join("artifact.so");
        fs::write(&artifact, b"reviewed")?;
        symlink("artifact.so", directory.path.join("link.so"))?;
        let verified = verify_directory_manifest(&directory.expectation()?)?;
        verified.confirm_unchanged()?;
        if mutation == "replaced" {
            let replacement = directory.path.join("replacement.tmp");
            fs::write(&replacement, b"reviewed")?;
            fs::rename(&replacement, &artifact)?;
        } else {
            fs::write(directory.path.join("extra.so"), b"extra")?;
        }
        assert!(matches!(
            verified.confirm_unchanged(),
            Err(ProvenanceError::IdentityChanged)
        ));
    }
    Ok(())
}

fn sha256_of(bytes: &[u8]) -> String {
    super::encode_lower_hex(sha2::Sha256::digest(bytes))
}
