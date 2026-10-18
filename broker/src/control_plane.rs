use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use thiserror::Error;

use crate::personalization::{
    MAX_PERSONALIZATION_BYTES, PersonalizationMutation, PersonalizationProvider,
    PersonalizationSignal, PersonalizationStore, PersonalizationStoreError, PersonalizationV1,
};
use crate::settings::{
    PermissionDecision, PrivateStorage, PrivateStorageError, RetentionPermission, SettingsStore,
    SettingsStoreError, SettingsV2, StableIdentity, StoragePaths, SubjectPermissions, SubjectRule,
    read_private_limited, remove_private_file, remove_private_temporary_files,
};
use crate::style::{STYLE_FILE_NAME, StyleError, StyleKey, StyleSample, StyleStatus, StyleStore};

const SECONDS_PER_DAY: u64 = 86_400;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPlaneSnapshot {
    pub settings: SettingsV2,
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
    settings: SettingsV2,
    personalization: Option<PersonalizationStore>,
    /// `None` when the sealed file is unreadable: it is preserved, nothing
    /// is learned, and only `forget_style` repairs it.
    style: Option<StyleStore>,
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
        let style = match StyleStore::open(storage.paths().data_dir().join(STYLE_FILE_NAME))
            .and_then(|mut store| store.reconcile(&settings, today).map(|_| store))
        {
            Ok(store) => Some(store),
            Err(error) => {
                eprintln!(
                    "badi-broker: style store unavailable and preserved; badi style forget required: {error}"
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
                style,
            }),
        })
    }

    /// Holds the keyring's style key for this broker's lifetime.
    pub fn deliver_style_key(&self, key: StyleKey) -> Result<StyleStatus, ControlPlaneError> {
        let mut state = self.lock_state()?;
        let store = state
            .style
            .as_mut()
            .ok_or(ControlPlaneError::StyleUnavailable)?;
        store.set_key(key);
        Ok(store.status())
    }

    pub fn style_status(&self) -> Result<Option<StyleStatus>, ControlPlaneError> {
        Ok(self.lock_state()?.style.as_ref().map(StyleStore::status))
    }

    /// Seals one authored sample when the settings the event was admitted
    /// under are still current and still allow it.
    pub fn record_style_at_settings_revision(
        &self,
        expected_settings_revision: u64,
        identity: &StableIdentity,
        text: &str,
    ) -> Result<bool, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        if state.settings.revision != expected_settings_revision {
            return Ok(false);
        }
        let ControlPlaneState {
            settings, style, ..
        } = &mut *state;
        let store = style.as_mut().ok_or(ControlPlaneError::StyleUnavailable)?;
        Ok(store.record(settings, today, identity, text)?)
    }

    /// Every decrypted sample, for an explicit export.
    pub fn export_style(&self) -> Result<Vec<StyleSample>, ControlPlaneError> {
        let today = unix_day_now()?;
        let mut state = self.lock_state()?;
        let ControlPlaneState {
            settings, style, ..
        } = &mut *state;
        let store = style.as_mut().ok_or(ControlPlaneError::StyleUnavailable)?;
        store.reconcile(settings, today)?;
        Ok(store.export()?)
    }

    /// Deletes every sample and forgets the key, repairing an unreadable
    /// store. The keyring item belongs to the caller.
    pub fn forget_style(&self) -> Result<StyleStatus, ControlPlaneError> {
        let mut state = self.lock_state()?;
        let path = self.storage.paths().data_dir().join(STYLE_FILE_NAME);
        if let Some(store) = state.style.as_mut() {
            store.forget()?;
            return Ok(store.status());
        }
        remove_private_temporary_files(&path)?;
        remove_private_file(&path)?;
        let store = StyleStore::open(path)?;
        let status = store.status();
        state.style = Some(store);
        Ok(status)
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
        next: SettingsV2,
    ) -> Result<SettingsV2, ControlPlaneError> {
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
            reconcile_style_locked(&mut state, &next, today);
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
        reconcile_style_locked(&mut state, &privacy_floor, today);
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
        reconcile_style_locked(&mut state, &replaced, today);
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

fn strictly_reduces_authority(current: &SettingsV2, next: &SettingsV2) -> bool {
    // `paused` participates in the durable order separately from subject
    // permissions. This rejects latent grants hidden underneath a pause and
    // rejects unpausing even when every currently listed subject is blocked.
    let mut reduced = !current.paused && next.paused;
    if current.paused && !next.paused {
        return false;
    }
    // The all-web and all-app defaults are authority over every unlisted
    // http(s) origin or Linux app; style learning is authority to keep text.
    for (was, is) in [
        (current.all_web_origins, next.all_web_origins),
        (current.all_linux_apps, next.all_linux_apps),
        (current.style_learning, next.style_learning),
    ] {
        match (was, is) {
            (false, true) => return false,
            (true, false) => reduced = true,
            _ => {}
        }
    }

    // Compare effective permissions: an unlisted identity has its document's
    // default, deny or the all-web default for an origin. Removing an explicit
    // origin block under the all-web default is therefore a grant. An explicit
    // tombstone equal to the default is authority-equivalent; any increase
    // invalidates the complete transition, even if another subject is revoked
    // in the same document.
    for identity in current
        .subjects
        .iter()
        .chain(&next.subjects)
        .map(|rule| &rule.identity)
    {
        let Some(rule_reduced) = permissions_do_not_increase(
            current.resolve_identity_validated(identity).permissions,
            next.resolve_identity_validated(identity).permissions,
        ) else {
            return false;
        };
        reduced |= rule_reduced;
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

fn personalization_privacy_floor(current: &SettingsV2, next: &SettingsV2) -> SettingsV2 {
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
    SettingsV2 {
        schema: crate::settings::SETTINGS_SCHEMA.to_owned(),
        revision: current.revision,
        paused: current.paused || next.paused,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: current.mid_line,
        escape_quiet_seconds: current.escape_quiet_seconds,
        battery_mode: current.battery_mode,
        style_learning: current.style_learning && next.style_learning,
        language_overrides: current.language_overrides.clone(),
        spelling_languages: current.spelling_languages.clone(),
        // A degraded control plane accepts only reductions; the model stays.
        writing_model: current.writing_model.clone(),
        writing_languages: current.writing_languages.clone(),
        trailing_space_healing: current.trailing_space_healing.clone(),
        subjects,
    }
}

fn personalization_commit_state_unknown(error: &PersonalizationStoreError) -> bool {
    matches!(
        error,
        PersonalizationStoreError::Storage(PrivateStorageError::CommitStateUnknown)
    )
}

/// Drops style samples the given settings no longer allow. A failure leaves
/// the sealed file preserved and the store unavailable until `forget_style`.
fn reconcile_style_locked(state: &mut ControlPlaneState, settings: &SettingsV2, today: u64) {
    if let Some(store) = state.style.as_mut() {
        if let Err(error) = store.reconcile(settings, today) {
            eprintln!(
                "badi-broker: style reconciliation failed; store preserved and disabled until badi style forget: {error}"
            );
            state.style = None;
        }
    }
}

fn reconcile_personalization_locked(
    state: &mut ControlPlaneState,
    today: u64,
) -> Result<Option<PersonalizationMutation>, PersonalizationStoreError> {
    let settings = state.settings.clone();
    reconcile_style_locked(state, &settings, today);
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
    #[error("style store is unavailable; badi style forget repairs it")]
    StyleUnavailable,
    #[error("style store: {0}")]
    Style(#[from] StyleError),
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

impl ControlPlaneError {
    /// A stable, content-free name for startup diagnostics.
    #[must_use]
    pub fn startup_name(&self) -> String {
        match self {
            Self::ClockBeforeUnixEpoch => "clock_before_unix_epoch".to_owned(),
            Self::LockPoisoned => "lock_poisoned".to_owned(),
            Self::PersonalizationUnavailable => "personalization_unavailable".to_owned(),
            Self::StyleUnavailable => "style_unavailable".to_owned(),
            Self::Style(error) => error.to_string(),
            Self::Personalization(error) => error.to_string(),
            Self::Storage(error) => error.to_string(),
            Self::Settings(error) => error.to_string(),
            Self::SettingsCommittedReconciliation { .. } => {
                "settings_committed_reconciliation".to_owned()
            }
        }
    }

    /// Corrupt or unsafe settings will not improve by restarting the process.
    #[must_use]
    pub fn restart_cannot_help(&self) -> bool {
        match self {
            Self::Settings(SettingsStoreError::Json(_) | SettingsStoreError::Validation(_))
            | Self::Personalization(
                PersonalizationStoreError::Json(_) | PersonalizationStoreError::Validation(_),
            ) => true,
            Self::Settings(SettingsStoreError::Storage(error))
            | Self::Storage(error)
            | Self::Personalization(PersonalizationStoreError::Storage(error)) => {
                storage_restart_cannot_help(error)
            }
            _ => false,
        }
    }
}

fn storage_restart_cannot_help(error: &PrivateStorageError) -> bool {
    matches!(
        error,
        PrivateStorageError::UnsafeDirectory
            | PrivateStorageError::UnsafeFile
            | PrivateStorageError::MissingHome(_)
            | PrivateStorageError::PathNotAbsolute
            | PrivateStorageError::InvalidFileName
    )
}

#[cfg(test)]
mod tests;
