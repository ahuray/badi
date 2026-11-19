use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

const SHA256_HEX_BYTES: usize = 64;
const HASH_BUFFER_BYTES: usize = 1024 * 1024;
const DIRECTORY_MANIFEST_HEADER: &str = "badi.runtime-directory-manifest.v1\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
    pub size: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileExpectation {
    path: PathBuf,
    sha256: String,
    size: u64,
}

impl FileExpectation {
    pub fn new(
        path: impl Into<PathBuf>,
        sha256: impl Into<String>,
        size: u64,
    ) -> Result<Self, ProvenanceError> {
        let expectation = Self {
            path: path.into(),
            sha256: sha256.into(),
            size,
        };
        expectation.validate()?;
        Ok(expectation)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    fn validate(&self) -> Result<(), ProvenanceError> {
        validate_absolute_digest(&self.path, &self.sha256)?;
        if self.size == 0 {
            return Err(ProvenanceError::InvalidSize);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryManifestExpectation {
    path: PathBuf,
    sha256: String,
}

impl DirectoryManifestExpectation {
    pub fn new(
        path: impl Into<PathBuf>,
        sha256: impl Into<String>,
    ) -> Result<Self, ProvenanceError> {
        let expectation = Self {
            path: path.into(),
            sha256: sha256.into(),
        };
        expectation.validate()?;
        Ok(expectation)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn validate(&self) -> Result<(), ProvenanceError> {
        validate_absolute_digest(&self.path, &self.sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct VerifiedFile {
    path: PathBuf,
    sha256: String,
    identity: FileIdentity,
    #[serde(skip)]
    hashed: FilesystemIdentity,
}

impl VerifiedFile {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    #[must_use]
    pub const fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn reverify(&self) -> Result<(), ProvenanceError> {
        let current = verify_file(&FileExpectation {
            path: self.path.clone(),
            sha256: self.sha256.clone(),
            size: self.identity.size,
        })?;
        if current.identity != self.identity {
            return Err(ProvenanceError::IdentityChanged);
        }
        Ok(())
    }

    /// Confirms the path still names the exact object whose bytes were hashed,
    /// without reading them again. Replacement changes the inode and a write
    /// changes the modification and change times.
    pub fn confirm_unchanged(&self) -> Result<(), ProvenanceError> {
        let current = metadata(&self.path, "unchanged-file metadata")?;
        validate_regular_leaf(&current)?;
        if filesystem_identity(&current) != self.hashed {
            return Err(ProvenanceError::IdentityChanged);
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct VerifiedDirectoryManifest {
    path: PathBuf,
    sha256: String,
    directory_identity: FilesystemIdentity,
    entries: Vec<ObservedDirectoryEntry>,
}

impl VerifiedDirectoryManifest {
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    pub fn reverify(&self) -> Result<(), ProvenanceError> {
        let current = verify_directory_manifest(&DirectoryManifestExpectation {
            path: self.path.clone(),
            sha256: self.sha256.clone(),
        })?;
        if current.directory_identity != self.directory_identity || current.entries != self.entries
        {
            return Err(ProvenanceError::IdentityChanged);
        }
        Ok(())
    }

    /// Like [`VerifiedFile::confirm_unchanged`] for the directory and every
    /// entry. An added or removed entry changes the directory's own times.
    pub fn confirm_unchanged(&self) -> Result<(), ProvenanceError> {
        let directory = metadata(&self.path, "unchanged runtime bundle metadata")?;
        validate_directory_leaf(&directory)?;
        let names = directory_entry_names(&self.path)?;
        if filesystem_identity(&directory) != self.directory_identity
            || names.len() != self.entries.len()
        {
            return Err(ProvenanceError::IdentityChanged);
        }
        for (name, entry) in names.iter().zip(&self.entries) {
            let current = metadata(&self.path.join(name), "unchanged runtime bundle entry")?;
            if *name != entry.name || filesystem_identity(&current) != entry.identity {
                return Err(ProvenanceError::IdentityChanged);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("artifact path must be absolute")]
    RelativePath,
    #[error("artifact path must be canonical and contain no symbolic-link component")]
    NonCanonicalPath,
    #[error("artifact path is a symbolic link")]
    Symlink,
    #[error("artifact path is not a regular file")]
    NotRegularFile,
    #[error("runtime bundle path is not a directory")]
    NotDirectory,
    #[error("artifact size expectation must be nonzero")]
    InvalidSize,
    #[error("artifact SHA-256 expectation must be 64 lowercase hexadecimal characters")]
    InvalidDigest,
    #[error("artifact size does not match the expected bytes")]
    SizeMismatch,
    #[error("artifact SHA-256 does not match the expected bytes")]
    DigestMismatch,
    #[error("runtime bundle manifest SHA-256 does not match the expected directory")]
    DirectoryManifestDigestMismatch,
    #[error("artifact identity changed during verification")]
    IdentityChanged,
    #[error("runtime bundle contains an unsafe manifest name: {0:?}")]
    UnsafeManifestName(String),
    #[error("runtime bundle contains an unsupported entry type: {0}")]
    UnsupportedDirectoryEntry(String),
    #[error("runtime bundle symbolic link has an unsafe target: {0}")]
    UnsafeSymlinkTarget(String),
    #[error("runtime bundle symbolic link target is missing: {0}")]
    MissingSymlinkTarget(String),
    #[error("runtime bundle symbolic link cycle begins at: {0}")]
    CyclicSymlink(String),
    #[error("artifact {operation} failed")]
    Io {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
}

pub fn verify_file(expectation: &FileExpectation) -> Result<VerifiedFile, ProvenanceError> {
    expectation.validate()?;
    let canonical = canonical_path(expectation.path())?;

    let initial = metadata(expectation.path(), "metadata")?;
    validate_regular_leaf(&initial)?;
    let initial_identity = file_identity(&initial);
    if initial_identity.size != expectation.size() {
        return Err(ProvenanceError::SizeMismatch);
    }

    let mut file = File::open(expectation.path()).map_err(|source| ProvenanceError::Io {
        operation: "open",
        source,
    })?;
    let opened = file.metadata().map_err(|source| ProvenanceError::Io {
        operation: "opened-file metadata",
        source,
    })?;
    validate_regular_leaf(&opened)?;
    if file_identity(&opened) != initial_identity {
        return Err(ProvenanceError::IdentityChanged);
    }

    let digest = hash_reader(&mut file)?;
    if digest != expectation.sha256() {
        return Err(ProvenanceError::DigestMismatch);
    }

    let opened_after = file.metadata().map_err(|source| ProvenanceError::Io {
        operation: "post-hash opened-file metadata",
        source,
    })?;
    let path_after = metadata(expectation.path(), "post-hash metadata")?;
    validate_regular_leaf(&opened_after)?;
    validate_regular_leaf(&path_after)?;
    if file_identity(&opened_after) != initial_identity
        || file_identity(&path_after) != initial_identity
    {
        return Err(ProvenanceError::IdentityChanged);
    }

    Ok(VerifiedFile {
        path: canonical,
        sha256: digest,
        identity: initial_identity,
        hashed: filesystem_identity(&initial),
    })
}

/// Hashes an exact, flat runtime directory after validating every entry.
///
/// The canonical manifest commits to sorted UTF-8 entry names, regular-file
/// sizes and hashes, and literal safe same-directory symbolic-link targets.
/// Permission bits are intentionally excluded so extraction umasks do not
/// change the content provenance of an otherwise identical release archive.
pub fn directory_manifest_sha256(path: &Path) -> Result<String, ProvenanceError> {
    Ok(observe_directory(path)?.sha256)
}

pub fn verify_directory_manifest(
    expectation: &DirectoryManifestExpectation,
) -> Result<VerifiedDirectoryManifest, ProvenanceError> {
    expectation.validate()?;
    let observed = observe_directory(expectation.path())?;
    if observed.sha256 != expectation.sha256() {
        return Err(ProvenanceError::DirectoryManifestDigestMismatch);
    }
    Ok(VerifiedDirectoryManifest {
        path: observed.path,
        sha256: observed.sha256,
        directory_identity: observed.directory_identity,
        entries: observed.entries,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct FilesystemIdentity {
    device: u64,
    inode: u64,
    size: u64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ObservedDirectoryEntry {
    name: String,
    identity: FilesystemIdentity,
    kind: ObservedDirectoryEntryKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ObservedDirectoryEntryKind {
    RegularFile { size: u64, sha256: String },
    Symlink { target: String },
}

struct ObservedDirectory {
    path: PathBuf,
    sha256: String,
    directory_identity: FilesystemIdentity,
    entries: Vec<ObservedDirectoryEntry>,
}

fn observe_directory(path: &Path) -> Result<ObservedDirectory, ProvenanceError> {
    if !path.is_absolute() {
        return Err(ProvenanceError::RelativePath);
    }
    let canonical = canonical_path(path)?;
    let initial_metadata = metadata(path, "runtime bundle metadata")?;
    validate_directory_leaf(&initial_metadata)?;
    let directory_identity = filesystem_identity(&initial_metadata);
    let names = directory_entry_names(path)?;
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        entries.push(observe_directory_entry(path, name)?);
    }
    validate_symlink_graph(&entries)?;

    let post_scan = snapshot_directory(path)?;
    if post_scan.directory_identity != directory_identity || post_scan.entries != entries {
        return Err(ProvenanceError::IdentityChanged);
    }

    let mut manifest = String::from(DIRECTORY_MANIFEST_HEADER);
    for entry in &entries {
        match &entry.kind {
            ObservedDirectoryEntryKind::RegularFile { size, sha256 } => {
                use std::fmt::Write as _;
                writeln!(&mut manifest, "file\t{size}\t{sha256}\t{}", entry.name)
                    .expect("writing to a String cannot fail");
            }
            ObservedDirectoryEntryKind::Symlink { target } => {
                use std::fmt::Write as _;
                writeln!(&mut manifest, "symlink\t{target}\t{}", entry.name)
                    .expect("writing to a String cannot fail");
            }
        }
    }

    Ok(ObservedDirectory {
        path: canonical,
        sha256: encode_lower_hex(Sha256::digest(manifest.as_bytes())),
        directory_identity,
        entries,
    })
}

fn snapshot_directory(path: &Path) -> Result<ObservedDirectory, ProvenanceError> {
    let initial = metadata(path, "post-scan runtime bundle metadata")?;
    validate_directory_leaf(&initial)?;
    let directory_identity = filesystem_identity(&initial);
    let names = directory_entry_names(path)?;
    let mut entries = Vec::with_capacity(names.len());
    for name in names {
        entries.push(observe_directory_entry(path, name)?);
    }
    validate_symlink_graph(&entries)?;
    let final_metadata = metadata(path, "final runtime bundle metadata")?;
    validate_directory_leaf(&final_metadata)?;
    if filesystem_identity(&final_metadata) != directory_identity {
        return Err(ProvenanceError::IdentityChanged);
    }
    Ok(ObservedDirectory {
        path: path.to_owned(),
        sha256: String::new(),
        directory_identity,
        entries,
    })
}

fn observe_directory_entry(
    directory: &Path,
    name: String,
) -> Result<ObservedDirectoryEntry, ProvenanceError> {
    let path = directory.join(&name);
    let initial = metadata(&path, "runtime bundle entry metadata")?;
    let initial_identity = filesystem_identity(&initial);
    let kind = if initial.file_type().is_file() {
        let mut file = File::open(&path).map_err(|source| ProvenanceError::Io {
            operation: "runtime bundle file open",
            source,
        })?;
        let opened = file.metadata().map_err(|source| ProvenanceError::Io {
            operation: "runtime bundle opened-file metadata",
            source,
        })?;
        validate_regular_leaf(&opened)?;
        if filesystem_identity(&opened) != initial_identity {
            return Err(ProvenanceError::IdentityChanged);
        }
        let sha256 = hash_reader(&mut file)?;
        let opened_after = file.metadata().map_err(|source| ProvenanceError::Io {
            operation: "runtime bundle post-hash opened-file metadata",
            source,
        })?;
        let path_after = metadata(&path, "runtime bundle post-hash metadata")?;
        validate_regular_leaf(&opened_after)?;
        validate_regular_leaf(&path_after)?;
        if filesystem_identity(&opened_after) != initial_identity
            || filesystem_identity(&path_after) != initial_identity
        {
            return Err(ProvenanceError::IdentityChanged);
        }
        ObservedDirectoryEntryKind::RegularFile {
            size: initial.len(),
            sha256,
        }
    } else if initial.file_type().is_symlink() {
        let target = read_safe_symlink_target(&path)?;
        let path_after = metadata(&path, "runtime bundle post-link metadata")?;
        if filesystem_identity(&path_after) != initial_identity
            || read_safe_symlink_target(&path)? != target
        {
            return Err(ProvenanceError::IdentityChanged);
        }
        ObservedDirectoryEntryKind::Symlink { target }
    } else {
        return Err(ProvenanceError::UnsupportedDirectoryEntry(name));
    };
    Ok(ObservedDirectoryEntry {
        name,
        identity: initial_identity,
        kind,
    })
}

fn directory_entry_names(path: &Path) -> Result<Vec<String>, ProvenanceError> {
    let directory = fs::read_dir(path).map_err(|source| ProvenanceError::Io {
        operation: "runtime bundle directory read",
        source,
    })?;
    let mut names = Vec::new();
    for entry in directory {
        let entry = entry.map_err(|source| ProvenanceError::Io {
            operation: "runtime bundle directory entry read",
            source,
        })?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| ProvenanceError::UnsafeManifestName(name.to_string_lossy().into()))?;
        validate_manifest_component(&name)?;
        names.push(name);
    }
    names.sort_unstable();
    Ok(names)
}

fn validate_manifest_component(value: &str) -> Result<(), ProvenanceError> {
    if value.is_empty()
        || value.chars().any(char::is_control)
        || !matches!(
            Path::new(value).components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        )
    {
        return Err(ProvenanceError::UnsafeManifestName(value.to_owned()));
    }
    Ok(())
}

fn read_safe_symlink_target(path: &Path) -> Result<String, ProvenanceError> {
    let target = fs::read_link(path).map_err(|source| ProvenanceError::Io {
        operation: "runtime bundle symbolic-link read",
        source,
    })?;
    let target = target
        .into_os_string()
        .into_string()
        .map_err(|target| ProvenanceError::UnsafeSymlinkTarget(target.to_string_lossy().into()))?;
    if validate_manifest_component(&target).is_err() {
        return Err(ProvenanceError::UnsafeSymlinkTarget(target));
    }
    Ok(target)
}

fn validate_symlink_graph(entries: &[ObservedDirectoryEntry]) -> Result<(), ProvenanceError> {
    let by_name = entries
        .iter()
        .map(|entry| (entry.name.as_str(), &entry.kind))
        .collect::<BTreeMap<_, _>>();
    for entry in entries {
        if !matches!(entry.kind, ObservedDirectoryEntryKind::Symlink { .. }) {
            continue;
        }
        let mut current = entry.name.as_str();
        let mut visited = HashSet::new();
        loop {
            if !visited.insert(current) {
                return Err(ProvenanceError::CyclicSymlink(entry.name.clone()));
            }
            match by_name.get(current) {
                Some(ObservedDirectoryEntryKind::RegularFile { .. }) => break,
                Some(ObservedDirectoryEntryKind::Symlink { target }) => {
                    current = target;
                }
                None => {
                    return Err(ProvenanceError::MissingSymlinkTarget(current.to_owned()));
                }
            }
        }
    }
    Ok(())
}

fn validate_absolute_digest(path: &Path, sha256: &str) -> Result<(), ProvenanceError> {
    if !path.is_absolute() {
        return Err(ProvenanceError::RelativePath);
    }
    if !is_lower_hex(sha256, SHA256_HEX_BYTES) {
        return Err(ProvenanceError::InvalidDigest);
    }
    Ok(())
}

fn canonical_path(path: &Path) -> Result<PathBuf, ProvenanceError> {
    let canonical = fs::canonicalize(path).map_err(|source| ProvenanceError::Io {
        operation: "canonicalization",
        source,
    })?;
    if canonical != path {
        return Err(ProvenanceError::NonCanonicalPath);
    }
    Ok(canonical)
}

fn metadata(path: &Path, operation: &'static str) -> Result<Metadata, ProvenanceError> {
    fs::symlink_metadata(path).map_err(|source| ProvenanceError::Io { operation, source })
}

fn validate_regular_leaf(metadata: &Metadata) -> Result<(), ProvenanceError> {
    if metadata.file_type().is_symlink() {
        return Err(ProvenanceError::Symlink);
    }
    if !metadata.file_type().is_file() {
        return Err(ProvenanceError::NotRegularFile);
    }
    Ok(())
}

fn validate_directory_leaf(metadata: &Metadata) -> Result<(), ProvenanceError> {
    if metadata.file_type().is_symlink() {
        return Err(ProvenanceError::Symlink);
    }
    if !metadata.file_type().is_dir() {
        return Err(ProvenanceError::NotDirectory);
    }
    Ok(())
}

fn file_identity(metadata: &Metadata) -> FileIdentity {
    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
    }
}

fn filesystem_identity(metadata: &Metadata) -> FilesystemIdentity {
    FilesystemIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
        size: metadata.len(),
        modified_seconds: metadata.mtime(),
        modified_nanoseconds: metadata.mtime_nsec(),
        changed_seconds: metadata.ctime(),
        changed_nanoseconds: metadata.ctime_nsec(),
    }
}

fn hash_reader(reader: &mut impl Read) -> Result<String, ProvenanceError> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; HASH_BUFFER_BYTES];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|source| ProvenanceError::Io {
                operation: "read",
                source,
            })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(encode_lower_hex(hasher.finalize()))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn encode_lower_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write as _;

    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests;
