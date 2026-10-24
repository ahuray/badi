//! Private files and directories: owner-only modes, bounded reads, atomic writes.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::Path;

use thiserror::Error;

const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
pub(super) const PRIVATE_FILE_MODE: u32 = 0o600;
const TEMPORARY_NONCE_HEX_LENGTH: usize = 32;

pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), PrivateStorageError> {
    if !path.is_absolute() {
        return Err(PrivateStorageError::PathNotAbsolute);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => return validate_private_directory_metadata(&metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    fs::create_dir_all(parent)?;
    let mut builder = fs::DirBuilder::new();
    builder.mode(PRIVATE_DIRECTORY_MODE);
    match builder.create(path) {
        Ok(()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    validate_private_directory_metadata(&metadata)
}

fn validate_private_directory_metadata(metadata: &fs::Metadata) -> Result<(), PrivateStorageError> {
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o777 != PRIVATE_DIRECTORY_MODE
    {
        return Err(PrivateStorageError::UnsafeDirectory);
    }
    Ok(())
}

pub(crate) fn read_private_limited(
    path: &Path,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, PrivateStorageError> {
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    validate_private_file_metadata(&link_metadata, Some(max_bytes))?;
    let mut file = File::open(path)?;
    validate_open_private_file(path, &file, Some(max_bytes))?;

    let mut bytes = Vec::with_capacity(
        usize::try_from(file.metadata()?.len())
            .unwrap_or(max_bytes)
            .min(max_bytes),
    );
    Read::by_ref(&mut file)
        .take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(PrivateStorageError::FileTooLarge(bytes.len()));
    }
    Ok(Some(bytes))
}

pub(super) fn validate_open_private_file(
    path: &Path,
    file: &File,
    max_bytes: Option<usize>,
) -> Result<(), PrivateStorageError> {
    let opened = file.metadata()?;
    validate_private_file_metadata(&opened, max_bytes)?;
    let current = fs::symlink_metadata(path)?;
    validate_private_file_metadata(&current, max_bytes)?;
    if opened.dev() != current.dev() || opened.ino() != current.ino() {
        return Err(PrivateStorageError::FileChanged);
    }
    Ok(())
}

fn validate_private_file_metadata(
    metadata: &fs::Metadata,
    max_bytes: Option<usize>,
) -> Result<(), PrivateStorageError> {
    if !metadata.is_file()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o777 != PRIVATE_FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(PrivateStorageError::UnsafeFile);
    }
    if max_bytes.is_some_and(|limit| metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX)) {
        return Err(PrivateStorageError::FileTooLarge(
            usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        ));
    }
    Ok(())
}

pub(crate) fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), PrivateStorageError> {
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    ensure_private_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(existing) => validate_private_file_metadata(&existing, None)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PrivateStorageError::InvalidFileName)?;
    let temporary = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().as_simple()
    ));
    let mut temporary_created = false;
    let mut renamed = false;
    let mut directory_synced = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&temporary)?;
        temporary_created = true;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        validate_open_private_file(&temporary, &file, None)?;
        fs::rename(&temporary, path)?;
        renamed = true;
        sync_directory(parent)?;
        directory_synced = true;
        let written = File::open(path)?;
        validate_open_private_file(path, &written, None)
    })();
    if let Err(error) = result {
        if !renamed {
            if temporary_created && remove_private_file(&temporary).is_err() {
                return Err(PrivateStorageError::CommitStateUnknown);
            }
            return Err(error);
        }
        // The rename is the commit point. If a later durability or validation
        // step fails, recover only when the safely re-opened destination is
        // byte-for-byte the intended document. Otherwise callers must assume
        // either version may be authoritative and remain fail-closed.
        return match read_private_limited(path, bytes.len()) {
            Ok(Some(observed))
                if observed == bytes && (directory_synced || sync_directory(parent).is_ok()) =>
            {
                Ok(())
            }
            _ => Err(PrivateStorageError::CommitStateUnknown),
        };
    }
    Ok(())
}

/// Removes only complete-document temporary files created for `path`.
///
/// The caller must hold the storage root's lifetime lock. An exact-looking but
/// unsafe entry is preserved and fails closed rather than being unlinked.
pub(crate) fn remove_private_temporary_files(path: &Path) -> Result<bool, PrivateStorageError> {
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    ensure_private_directory(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PrivateStorageError::InvalidFileName)?;
    let prefix = format!(".{file_name}.");
    let suffix = ".tmp";
    let mut removed = false;
    let result = (|| {
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(nonce) = name
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(suffix))
            else {
                continue;
            };
            if nonce.len() != TEMPORARY_NONCE_HEX_LENGTH
                || !nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                continue;
            }
            let temporary = entry.path();
            let metadata = fs::symlink_metadata(&temporary)?;
            validate_private_file_metadata(&metadata, None)?;
            fs::remove_file(&temporary)?;
            removed = true;
        }
        Ok(())
    })();
    if removed && sync_directory(parent).is_err() {
        return Err(PrivateStorageError::CommitStateUnknown);
    }
    result.map(|()| removed)
}

pub(crate) fn remove_private_file(path: &Path) -> Result<bool, PrivateStorageError> {
    remove_private_file_with_sync(path, sync_directory)
}

pub(super) fn remove_private_file_with_sync<F>(
    path: &Path,
    mut sync_parent: F,
) -> Result<bool, PrivateStorageError>
where
    F: FnMut(&Path) -> Result<(), PrivateStorageError>,
{
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Absence may be the visible result of an earlier unlink whose
            // directory sync failed. Require a successful namespace sync even
            // on retries so clear cannot acknowledge data that may resurrect
            // after a crash.
            let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
            return sync_parent(parent)
                .map(|()| false)
                .map_err(|_| PrivateStorageError::CommitStateUnknown);
        }
        Err(error) => return Err(error.into()),
    };
    validate_private_file_metadata(&metadata, None)?;
    fs::remove_file(path)?;
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    if sync_parent(parent).is_err() {
        return match fs::symlink_metadata(path) {
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && sync_parent(parent).is_ok() =>
            {
                Ok(true)
            }
            _ => Err(PrivateStorageError::CommitStateUnknown),
        };
    }
    Ok(true)
}

pub(super) fn sync_directory(path: &Path) -> Result<(), PrivateStorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[derive(Debug, Error)]
pub enum PrivateStorageError {
    #[error("commit_state_unknown")]
    CommitStateUnknown,
    #[error("file_changed")]
    FileChanged,
    #[error("file_too_large:{0}")]
    FileTooLarge(usize),
    #[error("invalid_file_name")]
    InvalidFileName,
    #[error("io")]
    Io(#[from] io::Error),
    #[error("lock_unavailable")]
    LockUnavailable,
    #[error("missing_home:{0}")]
    MissingHome(&'static str),
    #[error("path_not_absolute")]
    PathNotAbsolute,
    #[error("unsafe_directory")]
    UnsafeDirectory,
    #[error("unsafe_file")]
    UnsafeFile,
}
