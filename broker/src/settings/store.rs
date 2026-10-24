//! The settings file: compare-and-swap replacement and canonical encoding.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::Deserialize;
use thiserror::Error;

use crate::protocol::MAX_SAFE_COUNTER;

use super::document::{SettingsV2, SettingsValidationError};
use super::paths::StoreLock;
use super::private_file::{PrivateStorageError, atomic_write_private, read_private_limited};
use super::{MAX_SETTINGS_BYTES, SETTINGS_SCHEMA_V1};

#[derive(Debug)]
pub(crate) struct SettingsStore {
    path: PathBuf,
    _lock: Arc<StoreLock>,
    mutation: Arc<Mutex<()>>,
}

impl SettingsStore {
    pub(super) fn new(
        path: impl Into<PathBuf>,
        lock: Arc<StoreLock>,
        mutation: Arc<Mutex<()>>,
    ) -> Result<Self, PrivateStorageError> {
        let path = path.into();
        if !path.is_absolute() || path.parent().is_none() {
            return Err(PrivateStorageError::PathNotAbsolute);
        }
        Ok(Self {
            path,
            _lock: lock,
            mutation,
        })
    }

    #[must_use]
    #[cfg(test)]
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load_or_initialize(&self) -> Result<SettingsV2, SettingsStoreError> {
        let _guard = self
            .mutation
            .lock()
            .map_err(|_| SettingsStoreError::LockPoisoned)?;
        if let Some(bytes) = read_private_limited(&self.path, MAX_SETTINGS_BYTES)? {
            let (settings, migrated) = decode_settings_with_source(&bytes)?;
            if migrated {
                // The existing lifetime and mutation locks make this a single
                // writer migration. atomic_write_private keeps either the old
                // complete v1 document or the new complete v2 document visible.
                write_settings(&self.path, &settings)?;
            }
            Ok(settings)
        } else {
            let settings = SettingsV2::deny_by_default();
            write_settings(&self.path, &settings)?;
            Ok(settings)
        }
    }

    #[cfg(test)]
    pub(super) fn load(&self) -> Result<Option<SettingsV2>, SettingsStoreError> {
        read_private_limited(&self.path, MAX_SETTINGS_BYTES)?
            .map(|bytes| decode_settings(&bytes))
            .transpose()
    }

    pub(crate) fn compare_and_replace(
        &self,
        expected_revision: u64,
        next: SettingsV2,
    ) -> Result<SettingsV2, SettingsStoreError> {
        self.compare_and_replace_with_writer(expected_revision, &next, write_settings)?;
        Ok(next)
    }

    pub(super) fn compare_and_replace_with_writer<F>(
        &self,
        expected_revision: u64,
        next: &SettingsV2,
        writer: F,
    ) -> Result<SettingsV2, SettingsStoreError>
    where
        F: FnOnce(&Path, &SettingsV2) -> Result<(), SettingsStoreError>,
    {
        next.validate()?;
        let required_revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_SAFE_COUNTER)
            .ok_or(SettingsValidationError::RevisionOutOfRange)?;
        if next.revision != required_revision {
            return Err(SettingsStoreError::NextRevision {
                expected: required_revision,
                actual: next.revision,
            });
        }

        let _guard = self
            .mutation
            .lock()
            .map_err(|_| SettingsStoreError::LockPoisoned)?;
        let current = match read_private_limited(&self.path, MAX_SETTINGS_BYTES)? {
            Some(bytes) => decode_settings(&bytes)?,
            None => SettingsV2::deny_by_default(),
        };
        if current.revision != expected_revision {
            return Err(SettingsStoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        match writer(&self.path, next) {
            Ok(()) => Ok(next.clone()),
            Err(write_error) => {
                if matches!(
                    &write_error,
                    SettingsStoreError::Storage(PrivateStorageError::CommitStateUnknown)
                ) {
                    return Err(SettingsStoreError::CommitStateUnknown);
                }
                // Atomic replacement can fail after rename (for example while
                // syncing the directory). Re-read under the same mutation lock
                // before deciding whether old authority may be restored.
                // Exact new bytes mean the replacement is authoritative now;
                // exact old bytes mean it was rejected. Anything else is
                // commit-unknown and must keep the broker fail-closed.
                let observed = read_private_limited(&self.path, MAX_SETTINGS_BYTES)
                    .map_err(SettingsStoreError::from)
                    .and_then(|bytes| bytes.map(|value| decode_settings(&value)).transpose());
                match observed {
                    Ok(Some(document)) if document == *next => Ok(next.clone()),
                    Ok(Some(document)) if document == current => Err(write_error),
                    Ok(None) if current == SettingsV2::deny_by_default() => Err(write_error),
                    _ => Err(SettingsStoreError::CommitStateUnknown),
                }
            }
        }
    }

    pub(crate) fn preflight_replace(next: &SettingsV2) -> Result<(), SettingsStoreError> {
        let _ = encode_settings(next)?;
        Ok(())
    }
}

fn decode_settings(bytes: &[u8]) -> Result<SettingsV2, SettingsStoreError> {
    decode_settings_with_source(bytes).map(|(settings, _)| settings)
}

fn decode_settings_with_source(bytes: &[u8]) -> Result<(SettingsV2, bool), SettingsStoreError> {
    #[derive(Deserialize)]
    struct SchemaProbe {
        schema: String,
    }

    let source = serde_json::from_slice::<SchemaProbe>(bytes)?;
    let settings: SettingsV2 = serde_json::from_slice(bytes)?;
    settings.validate()?;
    Ok((settings, source.schema == SETTINGS_SCHEMA_V1))
}

pub(super) fn write_settings(path: &Path, settings: &SettingsV2) -> Result<(), SettingsStoreError> {
    let bytes = encode_settings(settings)?;
    atomic_write_private(path, &bytes)?;
    Ok(())
}

pub(super) fn encode_settings(settings: &SettingsV2) -> Result<Vec<u8>, SettingsStoreError> {
    settings.validate()?;
    let mut bytes = serde_json::to_vec_pretty(settings)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsStoreError::EncodedTooLarge(bytes.len()));
    }
    Ok(bytes)
}

#[derive(Debug, Error)]
pub enum SettingsStoreError {
    #[error("commit_state_unknown")]
    CommitStateUnknown,
    #[error("encoded_too_large:{0}")]
    EncodedTooLarge(usize),
    #[error("json")]
    Json(#[from] serde_json::Error),
    #[error("lock_poisoned")]
    LockPoisoned,
    #[error("next_revision:{expected}:{actual}")]
    NextRevision { expected: u64, actual: u64 },
    #[error("revision_conflict:{expected}:{actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("storage")]
    Storage(#[from] PrivateStorageError),
    #[error("validation")]
    Validation(#[from] SettingsValidationError),
}
