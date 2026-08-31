use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::personalization::{
    MAX_PERSONALIZATION_BYTES, PersonalizationMutation, PersonalizationProvider,
    PersonalizationSignal, PersonalizationStore, PersonalizationStoreError, PersonalizationV1,
};
use crate::settings::{
    PermissionDecision, PrivateStorage, PrivateStorageError, RetentionPermission, SettingsStore,
    SettingsStoreError, SettingsV1, StableIdentity, StoragePaths, SubjectPermissions, SubjectRule,
    read_private_limited, remove_private_file, remove_private_temporary_files,
};

const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPlaneSnapshot {
    pub settings: SettingsV1,
    pub personalization: PersonalizationV1,
    pub persisted_personalization_bytes: usize,
    pub personalization_store_available: bool,
}

#[derive(Debug)]
pub struct ControlPlane {
    // Retaining PrivateStorage retains exclusive advisory locks for both XDG
    // roots for this instance's lifetime.
    storage: PrivateStorage,
    state: Mutex<ControlPlaneState>,
}

#[derive(Debug)]
struct ControlPlaneState {
    settings_store: SettingsStore,
    settings: SettingsV1,
    personalization: Option<PersonalizationStore>,
}

impl ControlPlane {
    pub fn open_from_environment() -> Result<Self, ControlPlaneError> {
        Self::from_storage(PrivateStorage::open_from_environment()?)
    }

    pub fn open(paths: StoragePaths) -> Result<Self, ControlPlaneError> {
        Self::from_storage(PrivateStorage::open(paths)?)
    }

    fn from_storage(storage: PrivateStorage) -> Result<Self, ControlPlaneError> {
        let today = unix_day_now()?;
        let settings_store = storage.settings_store()?;
        let settings = settings_store.load_or_initialize()?;
        let personalization = match PersonalizationStore::open(
            storage.paths().personalization_path(),
            &settings,
            today,
        ) {
            Ok(store) => Some(store),
            Err(error) => {
                eprintln!(
                    "badi-broker: personalization store unavailable and preserved; explicit clear required: {error}"
                );
                None
            }
        };
        Ok(Self {
            storage,
            state: Mutex::new(ControlPlaneState {
                settings_store,
                settings,
                personalization,
            }),
        })
    }

    pub fn snapshot(&self) -> Result<ControlPlaneSnapshot, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        if let Err(error) = reconcile_personalization_locked(&mut state, today) {
            eprintln!(
                "badi-broker: personalization retention reconciliation failed; store preserved and disabled until explicit clear: {error}"
            );
        }
        let (personalization, persisted_personalization_bytes, personalization_store_available) =
            match state.personalization.as_ref() {
                Some(store) => {
                    let inspected = (|| -> Result<_, ControlPlaneError> {
                        let personalization = store.snapshot()?;
                        let persisted = read_private_limited(
                            &self.storage.paths().personalization_path(),
                            MAX_PERSONALIZATION_BYTES,
                        )?;
                        let persisted_personalization_bytes = match persisted {
                            Some(bytes) => {
                                let document: PersonalizationV1 = serde_json::from_slice(&bytes)
                                    .map_err(PersonalizationStoreError::from)?;
                                document
                                    .validate()
                                    .map_err(PersonalizationStoreError::from)?;
                                bytes.len()
                            }
                            None => 0,
                        };
                        Ok((personalization, persisted_personalization_bytes, true))
                    })();
                    match inspected {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            eprintln!(
                                "badi-broker: personalization store became unavailable and was preserved; explicit clear required: {error}"
                            );
                            state.personalization = None;
                            (PersonalizationV1::empty(), 0, false)
                        }
                    }
                }
                None => (PersonalizationV1::empty(), 0, false),
            };
        Ok(ControlPlaneSnapshot {
            settings: state.settings.clone(),
            personalization,
            persisted_personalization_bytes,
            personalization_store_available,
        })
    }

    pub fn replace_settings(
        &self,
        expected_revision: u64,
        next: SettingsV1,
    ) -> Result<SettingsV1, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;

        next.validate().map_err(SettingsStoreError::from)?;
        let required_revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= crate::protocol::MAX_SAFE_COUNTER)
            .ok_or_else(|| {
                SettingsStoreError::from(
                    crate::settings::SettingsValidationError::RevisionOutOfRange,
                )
            })?;
        if next.revision != required_revision {
            return Err(SettingsStoreError::NextRevision {
                expected: required_revision,
                actual: next.revision,
            }
            .into());
        }
        if state.settings.revision != expected_revision {
            return Err(SettingsStoreError::RevisionConflict {
                expected: expected_revision,
                actual: state.settings.revision,
            }
            .into());
        }
        SettingsStore::preflight_replace(&next)?;
        if state.personalization.is_none() {
            // Corrupt aggregate bytes must stay unavailable and untouched
            // until the explicit clear path repairs them. The settings store
            // is an independent durable authority record, though, so permit a
            // document that is strictly lower in the authority partial order.
            // Persisting that deny before returning lets the broker revoke its
            // live authority without waiting for aggregate repair. Any grant,
            // mixed grant/revoke, retention growth, or no-op is rejected.
            if !strictly_reduces_authority(&state.settings, &next) {
                return Err(ControlPlaneError::PersonalizationUnavailable);
            }
            let replaced = state
                .settings_store
                .compare_and_replace(expected_revision, next)?;
            state.settings = replaced.clone();
            return Ok(replaced);
        }

        // Reconcile the privacy intersection before settings commit. A crash
        // can therefore lose aggregates, but can never leave data on disk
        // after the committed settings revoke or shorten its retention.
        let personalization_revision = state
            .personalization
            .as_ref()
            .expect("availability checked")
            .snapshot()?
            .revision;
        let privacy_floor = personalization_privacy_floor(&state.settings, &next);
        let privacy_result = state
            .personalization
            .as_ref()
            .expect("availability checked")
            .reconcile(personalization_revision, &privacy_floor, today);
        if let Err(source) = privacy_result {
            if personalization_commit_state_unknown(&source) {
                state.personalization = None;
            }
            return Err(source.into());
        }
        let replaced = state
            .settings_store
            .compare_and_replace(expected_revision, next)?;
        state.settings = replaced.clone();
        let personalization_revision = state
            .personalization
            .as_ref()
            .expect("availability checked")
            .snapshot()?
            .revision;
        if let Err(source) = state
            .personalization
            .as_ref()
            .expect("availability checked")
            .reconcile(personalization_revision, &replaced, today)
        {
            if personalization_commit_state_unknown(&source) {
                state.personalization = None;
            }
            return Err(ControlPlaneError::SettingsCommittedReconciliation {
                settings_revision: replaced.revision,
                source,
            });
        }
        Ok(replaced)
    }

    pub fn clear_personalization(&self) -> Result<PersonalizationMutation, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        let path = self.storage.paths().personalization_path();
        let removed_temporaries = remove_private_temporary_files(&path)?;
        if let Some(personalization) = state.personalization.as_ref() {
            let revision = personalization.snapshot()?.revision;
            let result = personalization.clear(revision, &state.settings, today);
            return match result {
                Ok(mut mutation) => {
                    mutation.changed |= removed_temporaries;
                    Ok(mutation)
                }
                Err(error) => {
                    if personalization_commit_state_unknown(&error) {
                        state.personalization = None;
                    }
                    Err(error.into())
                }
            };
        }

        let changed = remove_private_file(&path)? || removed_temporaries;
        let personalization = PersonalizationStore::open(&path, &state.settings, today)?;
        let revision = personalization.snapshot()?.revision;
        state.personalization = Some(personalization);
        Ok(PersonalizationMutation {
            revision,
            changed,
            signal_recorded: false,
            signal_dropped: false,
        })
    }

    pub(crate) fn reconcile_personalization_now(
        &self,
    ) -> Result<Option<PersonalizationMutation>, ControlPlaneError> {
        self.reconcile_personalization_at(unix_day_now()?)
    }

    fn reconcile_personalization_at(
        &self,
        today: u64,
    ) -> Result<Option<PersonalizationMutation>, ControlPlaneError> {
        let mut state = self.lock_state()?;
        reconcile_personalization_locked(&mut state, today).map_err(ControlPlaneError::from)
    }

    pub fn record_signal(
        &self,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        signal: PersonalizationSignal,
    ) -> Result<PersonalizationMutation, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        let personalization = state
            .personalization
            .as_ref()
            .ok_or(ControlPlaneError::PersonalizationUnavailable)?;
        let revision = personalization.snapshot()?.revision;
        let result = personalization.record(
            revision,
            &state.settings,
            today,
            today,
            identity,
            provider,
            signal,
        );
        match result {
            Ok(mutation) => Ok(mutation),
            Err(error) => {
                if personalization_commit_state_unknown(&error) {
                    state.personalization = None;
                }
                Err(error.into())
            }
        }
    }

    pub fn record_signal_at_settings_revision(
        &self,
        expected_settings_revision: u64,
        event_day: u64,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        signal: PersonalizationSignal,
    ) -> Result<PersonalizationMutation, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        let personalization = state
            .personalization
            .as_ref()
            .ok_or(ControlPlaneError::PersonalizationUnavailable)?;
        if state.settings.revision != expected_settings_revision {
            return Ok(PersonalizationMutation {
                revision: personalization.snapshot()?.revision,
                changed: false,
                signal_recorded: false,
                signal_dropped: true,
            });
        }
        let revision = personalization.snapshot()?.revision;
        let result = personalization.record(
            revision,
            &state.settings,
            today,
            event_day,
            identity,
            provider,
            signal,
        );
        match result {
            Ok(mutation) => Ok(mutation),
            Err(error) => {
                if personalization_commit_state_unknown(&error) {
                    state.personalization = None;
                }
                Err(error.into())
            }
        }
    }

    fn lock_state(
        &self,
    ) -> Result<std::sync::MutexGuard<'_, ControlPlaneState>, ControlPlaneError> {
        self.state
            .lock()
            .map_err(|_| ControlPlaneError::LockPoisoned)
    }
}

fn strictly_reduces_authority(current: &SettingsV1, next: &SettingsV1) -> bool {
    // `paused` participates in the durable order separately from subject
    // permissions. This rejects latent grants hidden underneath a pause and
    // rejects unpausing even when every currently listed subject is blocked.
    let mut reduced = !current.paused && next.paused;
    if current.paused && !next.paused {
        return false;
    }

    for current_rule in &current.subjects {
        let next_permissions = next
            .subjects
            .binary_search_by(|rule| rule.identity.cmp(&current_rule.identity))
            .ok()
            .map_or_else(SubjectPermissions::deny_all, |index| {
                next.subjects[index].permissions
            });
        let Some(rule_reduced) =
            permissions_do_not_increase(current_rule.permissions, next_permissions)
        else {
            return false;
        };
        reduced |= rule_reduced;
    }

    // A newly listed subject starts from the effective deny-by-default rule.
    // An explicit all-deny tombstone is authority-equivalent; any allowed bit
    // or bounded retention would be an increase and invalidates the complete
    // transition, even if another subject is revoked in the same document.
    for next_rule in &next.subjects {
        if current
            .subjects
            .binary_search_by(|rule| rule.identity.cmp(&next_rule.identity))
            .is_err()
            && permissions_do_not_increase(SubjectPermissions::deny_all(), next_rule.permissions)
                .is_none()
        {
            return false;
        }
    }

    reduced
}

fn permissions_do_not_increase(
    current: SubjectPermissions,
    next: SubjectPermissions,
) -> Option<bool> {
    let decisions = [
        (current.suggest, next.suggest),
        (current.display, next.display),
        (current.context_read, next.context_read),
        (current.learn, next.learn),
    ];
    let mut reduced = false;
    for (current_decision, next_decision) in decisions {
        match (current_decision, next_decision) {
            (PermissionDecision::Block, PermissionDecision::Allow) => return None,
            (PermissionDecision::Allow, PermissionDecision::Block) => reduced = true,
            _ => {}
        }
    }

    let retention_reduced = match (current.retention, next.retention) {
        (RetentionPermission::None, RetentionPermission::Bounded { .. }) => return None,
        (RetentionPermission::Bounded { .. }, RetentionPermission::None) => true,
        (
            RetentionPermission::Bounded { days: current_days },
            RetentionPermission::Bounded { days: next_days },
        ) if next_days > current_days => return None,
        (
            RetentionPermission::Bounded { days: current_days },
            RetentionPermission::Bounded { days: next_days },
        ) => next_days < current_days,
        (RetentionPermission::None, RetentionPermission::None) => false,
    };
    Some(reduced || retention_reduced)
}

fn personalization_privacy_floor(current: &SettingsV1, next: &SettingsV1) -> SettingsV1 {
    let mut subjects = Vec::new();
    for current_rule in &current.subjects {
        let Ok(index) = next
            .subjects
            .binary_search_by(|rule| rule.identity.cmp(&current_rule.identity))
        else {
            continue;
        };
        let next_rule = &next.subjects[index];
        if current_rule.permissions.learn != PermissionDecision::Allow
            || next_rule.permissions.learn != PermissionDecision::Allow
        {
            continue;
        }
        let retention = match (
            current_rule.permissions.retention,
            next_rule.permissions.retention,
        ) {
            (
                RetentionPermission::Bounded { days: current_days },
                RetentionPermission::Bounded { days: next_days },
            ) => RetentionPermission::Bounded {
                days: current_days.min(next_days),
            },
            (RetentionPermission::None, RetentionPermission::Bounded { .. }) => {
                // Granting persistence is not consent to persist aggregates
                // observed before that grant. Omit the subject so the
                // pre-commit reconciliation scrubs its ephemeral history.
                continue;
            }
            _ => RetentionPermission::None,
        };
        subjects.push(SubjectRule {
            identity: current_rule.identity.clone(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention,
            },
        });
    }
    SettingsV1 {
        schema: crate::settings::SETTINGS_SCHEMA.to_owned(),
        revision: current.revision,
        paused: current.paused || next.paused,
        subjects,
    }
}

fn personalization_commit_state_unknown(error: &PersonalizationStoreError) -> bool {
    matches!(
        error,
        PersonalizationStoreError::Storage(PrivateStorageError::CommitStateUnknown)
    )
}

fn reconcile_personalization_locked(
    state: &mut ControlPlaneState,
    today: u64,
) -> Result<Option<PersonalizationMutation>, PersonalizationStoreError> {
    let Some(store) = state.personalization.as_ref() else {
        return Ok(None);
    };
    let result = store
        .snapshot()
        .and_then(|snapshot| store.reconcile(snapshot.revision, &state.settings, today));
    match result {
        Ok(mutation) => Ok(Some(mutation)),
        Err(error) => {
            // Failure to enforce expiry is a privacy-state failure even when
            // the write did not reach rename. Preserve the file, stop further
            // learning, and require the explicit validated clear path.
            state.personalization = None;
            Err(error)
        }
    }
}

pub fn unix_day_now() -> Result<u64, ControlPlaneError> {
    unix_day(SystemTime::now())
}

fn unix_day(time: SystemTime) -> Result<u64, ControlPlaneError> {
    let elapsed = time
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ControlPlaneError::ClockBeforeUnixEpoch)?;
    Ok(elapsed.as_secs() / SECONDS_PER_DAY)
}

#[derive(Debug, Error)]
pub enum ControlPlaneError {
    #[error("system clock is before the Unix epoch")]
    ClockBeforeUnixEpoch,
    #[error("control-plane state lock is poisoned")]
    LockPoisoned,
    #[error(
        "personalization store is unavailable; only a strict authority reduction or explicit clear is allowed"
    )]
    PersonalizationUnavailable,
    #[error("personalization store: {0}")]
    Personalization(#[from] PersonalizationStoreError),
    #[error("private storage: {0}")]
    Storage(#[from] PrivateStorageError),
    #[error("settings store: {0}")]
    Settings(#[from] SettingsStoreError),
    #[error(
        "settings revision {settings_revision} was committed, but personalization reconciliation failed: {source}"
    )]
    SettingsCommittedReconciliation {
        settings_revision: u64,
        #[source]
        source: PersonalizationStoreError,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
    use std::sync::Arc;
    use std::time::Duration;

    use tempfile::tempdir;

    use super::{ControlPlane, ControlPlaneError, strictly_reduces_authority, unix_day};
    use crate::personalization::{PersonalizationProvider, PersonalizationSignal};
    use crate::settings::{
        BrowserAdapter, PermissionDecision, PrivateStorage, PrivateStorageError,
        RetentionPermission, SETTINGS_SCHEMA, SettingsStoreError, SettingsV1,
        SettingsValidationError, StableIdentity, StoragePaths, SubjectPermissions, SubjectRule,
        WebScheme,
    };

    fn paths(root: &std::path::Path) -> StoragePaths {
        StoragePaths::new(root.join("config/badi"), root.join("data/badi")).expect("paths")
    }

    fn identity() -> StableIdentity {
        StableIdentity::browser_origin(
            BrowserAdapter::Chromium,
            WebScheme::Https,
            "example.com",
            None,
        )
        .expect("identity")
    }

    fn other_identity() -> StableIdentity {
        StableIdentity::browser_origin(
            BrowserAdapter::Chromium,
            WebScheme::Https,
            "other.example",
            None,
        )
        .expect("other identity")
    }

    fn learning_settings(revision: u64, retention: RetentionPermission) -> SettingsV1 {
        SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision,
            paused: false,
            subjects: vec![SubjectRule {
                identity: identity(),
                permissions: SubjectPermissions {
                    suggest: PermissionDecision::Allow,
                    display: PermissionDecision::Allow,
                    context_read: PermissionDecision::Allow,
                    learn: PermissionDecision::Allow,
                    retention,
                },
            }],
        }
    }

    fn granted_settings(revision: u64) -> SettingsV1 {
        learning_settings(revision, RetentionPermission::Bounded { days: 30 })
    }

    #[test]
    fn unavailable_store_authority_order_rejects_grants_noops_and_mixed_changes() {
        let current = granted_settings(7);

        let mut blocked = current.clone();
        blocked.revision = 8;
        blocked.subjects[0].permissions = SubjectPermissions::deny_all();
        assert!(strictly_reduces_authority(&current, &blocked));

        let mut shorter_retention = current.clone();
        shorter_retention.revision = 8;
        shorter_retention.subjects[0].permissions.retention =
            RetentionPermission::Bounded { days: 7 };
        assert!(strictly_reduces_authority(&current, &shorter_retention));

        let mut paused = current.clone();
        paused.revision = 8;
        paused.paused = true;
        assert!(strictly_reduces_authority(&current, &paused));

        let mut no_op = current.clone();
        no_op.revision = 8;
        assert!(!strictly_reduces_authority(&current, &no_op));

        let mut retention_growth = shorter_retention.clone();
        retention_growth.revision = 9;
        retention_growth.subjects[0].permissions.retention =
            RetentionPermission::Bounded { days: 30 };
        assert!(!strictly_reduces_authority(
            &shorter_retention,
            &retention_growth
        ));

        let mut latent_grant = blocked.clone();
        latent_grant.paused = true;
        latent_grant.subjects.push(SubjectRule {
            identity: other_identity(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Block,
                retention: RetentionPermission::None,
            },
        });
        assert!(!strictly_reduces_authority(&blocked, &latent_grant));

        let mut mixed = current.clone();
        mixed.revision = 8;
        mixed.subjects[0].permissions = SubjectPermissions::deny_all();
        mixed.subjects.push(SubjectRule {
            identity: other_identity(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Block,
                retention: RetentionPermission::None,
            },
        });
        assert!(!strictly_reduces_authority(&current, &mixed));
    }

    #[test]
    fn initializes_deny_by_default_and_retains_the_storage_lock() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        let snapshot = control.snapshot().expect("snapshot");
        assert_eq!(snapshot.settings, SettingsV1::deny_by_default());
        assert!(snapshot.personalization.records.is_empty());
        assert_eq!(snapshot.persisted_personalization_bytes, 0);
        assert!(matches!(
            PrivateStorage::open(storage_paths),
            Err(PrivateStorageError::LockUnavailable)
        ));
    }

    #[test]
    fn settings_cas_immediately_reconciles_revoked_personalization() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");
        control
            .record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record signal");
        assert!(
            control
                .snapshot()
                .expect("snapshot")
                .persisted_personalization_bytes
                > 0
        );

        let denied = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 2,
            paused: false,
            subjects: Vec::new(),
        };
        control
            .replace_settings(1, denied)
            .expect("revoke settings");
        let snapshot = control.snapshot().expect("reconciled snapshot");
        assert!(snapshot.personalization.records.is_empty());
        assert_eq!(snapshot.persisted_personalization_bytes, 0);
        assert!(!storage_paths.personalization_path().exists());
    }

    #[test]
    fn retention_grant_does_not_promote_pre_consent_ephemeral_history() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        control
            .replace_settings(0, learning_settings(1, RetentionPermission::None))
            .expect("grant memory-only learning");
        control
            .record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record ephemeral signal");
        assert_eq!(
            control
                .snapshot()
                .expect("ephemeral snapshot")
                .personalization
                .records[0]
                .shown,
            1
        );
        assert!(!storage_paths.personalization_path().exists());

        control
            .replace_settings(
                1,
                learning_settings(2, RetentionPermission::Bounded { days: 30 }),
            )
            .expect("grant bounded retention");
        let after_grant = control.snapshot().expect("post-grant snapshot");
        assert!(after_grant.personalization.records.is_empty());
        assert_eq!(after_grant.persisted_personalization_bytes, 0);
        assert!(!storage_paths.personalization_path().exists());

        control
            .record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record post-consent signal");
        let after_signal = control.snapshot().expect("post-consent snapshot");
        assert_eq!(after_signal.personalization.records[0].shown, 1);
        assert!(after_signal.persisted_personalization_bytes > 0);
        assert!(storage_paths.personalization_path().exists());
    }

    #[test]
    fn stale_settings_cas_does_not_reconcile_personalization() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");
        control
            .record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record signal");

        let before = control.snapshot().expect("snapshot before stale CAS");
        let persisted_before =
            fs::read(storage_paths.personalization_path()).expect("persisted personalization");
        let stale_replacement = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 100,
            paused: true,
            subjects: Vec::new(),
        };
        assert!(matches!(
            control.replace_settings(99, stale_replacement),
            Err(ControlPlaneError::Settings(
                SettingsStoreError::RevisionConflict {
                    expected: 99,
                    actual: 1
                }
            ))
        ));

        assert_eq!(
            control.snapshot().expect("snapshot after stale CAS"),
            before
        );
        assert_eq!(
            fs::read(storage_paths.personalization_path()).expect("unchanged personalization"),
            persisted_before
        );
    }

    #[test]
    fn over_capacity_settings_are_rejected_before_privacy_reconciliation() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");
        control
            .record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record signal");
        let before = control.snapshot().expect("snapshot before oversized CAS");
        let settings_before = fs::read(storage_paths.settings_path()).expect("settings bytes");
        let personalization_before =
            fs::read(storage_paths.personalization_path()).expect("personalization bytes");

        let subjects = (0..65)
            .map(|index| {
                let host = format!(
                    "h{index:03}.{}.{}.{}.{}",
                    "a".repeat(63),
                    "b".repeat(63),
                    "c".repeat(63),
                    "d".repeat(56)
                );
                SubjectRule {
                    identity: StableIdentity::browser_origin(
                        BrowserAdapter::Chromium,
                        WebScheme::Https,
                        &host,
                        None,
                    )
                    .expect("long identity"),
                    permissions: SubjectPermissions::deny_all(),
                }
            })
            .collect();
        let over_capacity = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 2,
            paused: true,
            subjects,
        };
        assert!(matches!(
            control.replace_settings(1, over_capacity),
            Err(ControlPlaneError::Settings(SettingsStoreError::Validation(
                SettingsValidationError::TooManySubjects
            )))
        ));

        assert_eq!(
            control.snapshot().expect("snapshot after rejection"),
            before
        );
        assert_eq!(
            fs::read(storage_paths.settings_path()).expect("unchanged settings"),
            settings_before
        );
        assert_eq!(
            fs::read(storage_paths.personalization_path()).expect("unchanged personalization"),
            personalization_before
        );
    }

    #[test]
    fn clear_and_concurrent_signals_use_the_internal_revision() {
        let temporary = tempdir().expect("temporary directory");
        let control = Arc::new(ControlPlane::open(paths(temporary.path())).expect("control plane"));
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");

        let workers: Vec<_> = (0..8)
            .map(|_| {
                let control = Arc::clone(&control);
                std::thread::spawn(move || {
                    control
                        .record_signal(
                            identity(),
                            PersonalizationProvider::PhraseV1,
                            PersonalizationSignal::Shown,
                        )
                        .expect("record signal");
                })
            })
            .collect();
        for worker in workers {
            worker.join().expect("worker");
        }
        assert_eq!(
            control
                .snapshot()
                .expect("snapshot")
                .personalization
                .records[0]
                .shown,
            8
        );

        let cleared = control.clear_personalization().expect("clear");
        assert!(cleared.changed);
        assert!(
            control
                .snapshot()
                .expect("cleared snapshot")
                .personalization
                .records
                .is_empty()
        );
    }

    #[test]
    fn corrupt_settings_are_reported_without_reinitialization() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        fs::create_dir_all(storage_paths.config_dir()).expect("config directory");
        fs::create_dir_all(storage_paths.data_dir()).expect("data directory");
        fs::set_permissions(
            storage_paths.config_dir(),
            fs::Permissions::from_mode(0o700),
        )
        .expect("config permissions");
        fs::set_permissions(storage_paths.data_dir(), fs::Permissions::from_mode(0o700))
            .expect("data permissions");
        let settings_path = storage_paths.settings_path();
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true).mode(0o600);
        std::io::Write::write_all(
            &mut options.open(&settings_path).expect("settings file"),
            b"not valid JSON\n",
        )
        .expect("settings contents");

        assert!(matches!(
            ControlPlane::open(storage_paths),
            Err(ControlPlaneError::Settings(_))
        ));
        assert_eq!(
            fs::read(&settings_path).expect("unchanged settings"),
            b"not valid JSON\n"
        );
    }

    #[test]
    fn persisted_aggregate_is_content_free_and_snapshot_size_is_exact() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");
        control
            .record_signal(
                identity(),
                PersonalizationProvider::LocalModel,
                PersonalizationSignal::Shown,
            )
            .expect("record signal");
        let bytes = fs::read(storage_paths.personalization_path()).expect("personalization file");
        let snapshot = control.snapshot().expect("snapshot");
        assert_eq!(snapshot.persisted_personalization_bytes, bytes.len());
        let encoded = String::from_utf8(bytes).expect("JSON");
        for forbidden in [
            "before",
            "after",
            "context",
            "suggestion_id",
            "target_id",
            "private text",
        ] {
            assert!(!encoded.contains(forbidden));
        }
    }

    #[test]
    fn event_time_settings_revision_prevents_reinterpreting_old_signals() {
        let temporary = tempdir().expect("temporary directory");
        let control = ControlPlane::open(paths(temporary.path())).expect("control plane");
        control
            .replace_settings(0, granted_settings(1))
            .expect("grant settings");
        let today = super::unix_day_now().expect("current UTC day");
        let stale = control
            .record_signal_at_settings_revision(
                0,
                today,
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("stale signal is ignored");
        assert!(!stale.signal_recorded);
        assert!(stale.signal_dropped);
        assert!(
            control
                .snapshot()
                .expect("snapshot")
                .personalization
                .records
                .is_empty()
        );

        let current = control
            .record_signal_at_settings_revision(
                1,
                today,
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("current signal");
        assert!(current.signal_recorded);
    }

    #[test]
    fn idle_reconciliation_expires_durable_and_memory_only_records() {
        let temporary = tempdir().expect("temporary directory");
        let today = super::unix_day_now().expect("current UTC day");

        let durable_paths = paths(&temporary.path().join("durable"));
        let durable = ControlPlane::open(durable_paths.clone()).expect("durable control plane");
        durable
            .replace_settings(
                0,
                learning_settings(1, RetentionPermission::Bounded { days: 1 }),
            )
            .expect("grant one-day retention");
        durable
            .record_signal_at_settings_revision(
                1,
                today,
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record durable signal");
        assert!(durable_paths.personalization_path().exists());
        let expired = durable
            .reconcile_personalization_at(today + 1)
            .expect("advance durable retention")
            .expect("store available");
        assert!(expired.changed);
        assert!(!durable_paths.personalization_path().exists());
        assert!(
            durable
                .snapshot()
                .expect("expired durable snapshot")
                .personalization
                .records
                .is_empty()
        );

        let ephemeral_paths = paths(&temporary.path().join("ephemeral"));
        let ephemeral =
            ControlPlane::open(ephemeral_paths.clone()).expect("ephemeral control plane");
        ephemeral
            .replace_settings(0, learning_settings(1, RetentionPermission::None))
            .expect("grant memory-only learning");
        ephemeral
            .record_signal_at_settings_revision(
                1,
                today,
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record memory-only signal");
        let expired = ephemeral
            .reconcile_personalization_at(today + 1)
            .expect("advance ephemeral retention")
            .expect("store available");
        assert!(expired.changed);
        assert!(!ephemeral_paths.personalization_path().exists());
        assert!(
            ephemeral
                .snapshot()
                .expect("expired ephemeral snapshot")
                .personalization
                .records
                .is_empty()
        );
    }

    #[allow(clippy::too_many_lines)]
    #[test]
    fn corrupt_optional_personalization_accepts_durable_deny_before_explicit_clear() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        {
            let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
            control
                .replace_settings(0, granted_settings(1))
                .expect("grant settings");
            control
                .record_signal(
                    identity(),
                    PersonalizationProvider::PhraseV1,
                    PersonalizationSignal::Shown,
                )
                .expect("seed personalization");
        }
        let personalization_path = storage_paths.personalization_path();
        fs::write(&personalization_path, b"not valid aggregate JSON\n")
            .expect("corrupt optional store");
        let corrupt_bytes = fs::read(&personalization_path).expect("corrupt bytes");

        let control = ControlPlane::open(storage_paths.clone()).expect("degraded control plane");
        let unavailable = control.snapshot().expect("unavailable snapshot");
        assert!(!unavailable.personalization_store_available);
        assert!(unavailable.personalization.records.is_empty());
        assert_eq!(unavailable.persisted_personalization_bytes, 0);
        assert_eq!(
            fs::read(&personalization_path).expect("preserved corrupt store"),
            corrupt_bytes
        );
        assert!(matches!(
            control.record_signal(
                identity(),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            ),
            Err(ControlPlaneError::PersonalizationUnavailable)
        ));

        let denied = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 2,
            paused: false,
            subjects: vec![SubjectRule {
                identity: identity(),
                permissions: SubjectPermissions::deny_all(),
            }],
        };
        let replaced = control
            .replace_settings(1, denied.clone())
            .expect("strict deny remains available");
        assert_eq!(replaced, denied);

        // Returning from replace_settings is the acknowledgement boundary:
        // the complete deny document and its directory entry are durable by
        // this point, while corrupt aggregate evidence remains untouched.
        let persisted_settings: SettingsV1 = serde_json::from_slice(
            &fs::read(storage_paths.settings_path()).expect("persisted deny settings"),
        )
        .expect("valid persisted settings");
        assert_eq!(persisted_settings, denied);
        assert_eq!(
            fs::read(&personalization_path).expect("preserved corrupt store after deny"),
            corrupt_bytes
        );

        let unavailable_after_deny = control.snapshot().expect("unavailable after deny");
        assert!(!unavailable_after_deny.personalization_store_available);
        assert_eq!(unavailable_after_deny.settings, denied);

        let grant = granted_settings(3);
        assert!(matches!(
            control.replace_settings(2, grant.clone()),
            Err(ControlPlaneError::PersonalizationUnavailable)
        ));
        assert_eq!(
            control
                .snapshot()
                .expect("still unavailable")
                .settings
                .revision,
            2
        );

        drop(control);
        let control = ControlPlane::open(storage_paths.clone())
            .expect("restart with durable deny and preserved corruption");
        let restarted = control.snapshot().expect("restarted snapshot");
        assert_eq!(restarted.settings, denied);
        assert!(!restarted.personalization_store_available);
        assert_eq!(
            fs::read(&personalization_path).expect("corrupt store survives restart"),
            corrupt_bytes
        );

        let orphaned_temporary = storage_paths
            .data_dir()
            .join(".personalization.json.00000000000000000000000000000004.tmp");
        fs::write(&orphaned_temporary, b"private aggregate copy\n")
            .expect("orphaned complete document");
        fs::set_permissions(&orphaned_temporary, fs::Permissions::from_mode(0o600))
            .expect("private orphan mode");

        let cleared = control
            .clear_personalization()
            .expect("explicit recovery clear");
        assert!(cleared.changed);
        let recovered = control.snapshot().expect("recovered snapshot");
        assert!(recovered.personalization_store_available);
        assert!(recovered.personalization.records.is_empty());
        assert_eq!(recovered.persisted_personalization_bytes, 0);
        assert!(!personalization_path.exists());
        assert!(!orphaned_temporary.exists());

        let granted = control
            .replace_settings(2, grant)
            .expect("grant is available only after explicit repair");
        assert_eq!(granted.revision, 3);
    }

    #[test]
    fn corrupt_store_rejects_retention_growth_and_mixed_authority_change() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = paths(temporary.path());
        {
            let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
            control
                .replace_settings(
                    0,
                    learning_settings(1, RetentionPermission::Bounded { days: 7 }),
                )
                .expect("grant seven-day settings");
            control
                .record_signal(
                    identity(),
                    PersonalizationProvider::PhraseV1,
                    PersonalizationSignal::Shown,
                )
                .expect("seed personalization");
        }
        let personalization_path = storage_paths.personalization_path();
        fs::write(&personalization_path, b"corrupt aggregate evidence\n")
            .expect("corrupt optional store");
        let corrupt_bytes = fs::read(&personalization_path).expect("corrupt bytes");
        let control = ControlPlane::open(storage_paths.clone()).expect("degraded control plane");

        let retention_growth = learning_settings(2, RetentionPermission::Bounded { days: 30 });
        assert!(matches!(
            control.replace_settings(1, retention_growth),
            Err(ControlPlaneError::PersonalizationUnavailable)
        ));

        let mut mixed = learning_settings(2, RetentionPermission::Bounded { days: 7 });
        mixed.subjects[0].permissions = SubjectPermissions::deny_all();
        mixed.subjects.push(SubjectRule {
            identity: other_identity(),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Block,
                retention: RetentionPermission::None,
            },
        });
        assert!(matches!(
            control.replace_settings(1, mixed),
            Err(ControlPlaneError::PersonalizationUnavailable)
        ));
        assert_eq!(
            control
                .snapshot()
                .expect("unchanged snapshot")
                .settings
                .revision,
            1
        );
        assert_eq!(
            fs::read(storage_paths.settings_path()).expect("unchanged settings bytes"),
            serde_json::to_vec_pretty(&learning_settings(
                1,
                RetentionPermission::Bounded { days: 7 }
            ))
            .map(|mut bytes| {
                bytes.push(b'\n');
                bytes
            })
            .expect("settings JSON")
        );
        assert_eq!(
            fs::read(personalization_path).expect("preserved corrupt evidence"),
            corrupt_bytes
        );
    }

    #[test]
    fn unix_day_uses_complete_utc_days_and_rejects_pre_epoch_time() {
        assert_eq!(
            unix_day(std::time::UNIX_EPOCH + Duration::from_secs(3 * 86_400 + 42)).expect("day"),
            3
        );
        assert!(matches!(
            unix_day(std::time::UNIX_EPOCH - Duration::from_secs(1)),
            Err(ControlPlaneError::ClockBeforeUnixEpoch)
        ));
    }
}
