use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::{MAX_SAFE_COUNTER, ProviderKind};
use crate::settings::{
    PermissionDecision, PrivateStorageError, RetentionPermission, SettingsV1,
    SettingsValidationError, StableIdentity, atomic_write_private, ensure_private_directory,
    read_private_limited, remove_private_file,
};

pub const PERSONALIZATION_SCHEMA: &str = "badi.personalization.v1";
pub const MAX_PERSONALIZATION_BYTES: usize = 262_144;
pub const MAX_PERSONALIZATION_RECORDS: usize = 512;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalizationProvider {
    PhraseV1,
    LocalModel,
}

impl From<ProviderKind> for PersonalizationProvider {
    fn from(provider: ProviderKind) -> Self {
        match provider {
            ProviderKind::PhraseV1 => Self::PhraseV1,
            ProviderKind::LocalModel => Self::LocalModel,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersonalizationSignal {
    Shown,
    Dismissed,
    AcceptedWord,
    AcceptedAll,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DailyAggregate {
    pub identity: StableIdentity,
    pub provider: PersonalizationProvider,
    pub day: u64,
    pub shown: u32,
    pub dismissed: u32,
    pub accepted_word: u32,
    pub accepted_all: u32,
}

impl DailyAggregate {
    fn new(identity: StableIdentity, provider: PersonalizationProvider, day: u64) -> Self {
        Self {
            identity,
            provider,
            day,
            shown: 0,
            dismissed: 0,
            accepted_word: 0,
            accepted_all: 0,
        }
    }

    fn key_cmp(
        &self,
        identity: &StableIdentity,
        provider: PersonalizationProvider,
        day: u64,
    ) -> Ordering {
        (&self.identity, self.provider, self.day).cmp(&(identity, provider, day))
    }

    fn outcome_count(&self) -> u64 {
        u64::from(self.dismissed)
            .saturating_add(u64::from(self.accepted_word))
            .saturating_add(u64::from(self.accepted_all))
    }

    fn validate(&self) -> Result<(), PersonalizationValidationError> {
        self.identity.validate()?;
        if self.day > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::DayOutOfRange);
        }
        if self.shown == 0 || self.outcome_count() > u64::from(self.shown) {
            return Err(PersonalizationValidationError::InvalidCounters);
        }
        Ok(())
    }

    fn record(&mut self, signal: PersonalizationSignal) -> Result<bool, PersonalizationStoreError> {
        let before = self.clone();
        match signal {
            PersonalizationSignal::Shown => {
                self.shown = self.shown.saturating_add(1);
            }
            PersonalizationSignal::Dismissed => {
                self.require_unconsumed_show()?;
                self.dismissed = self.dismissed.saturating_add(1);
            }
            PersonalizationSignal::AcceptedWord => {
                self.require_unconsumed_show()?;
                self.accepted_word = self.accepted_word.saturating_add(1);
            }
            PersonalizationSignal::AcceptedAll => {
                self.require_unconsumed_show()?;
                self.accepted_all = self.accepted_all.saturating_add(1);
            }
        }
        Ok(*self != before)
    }

    fn require_unconsumed_show(&self) -> Result<(), PersonalizationStoreError> {
        if self.outcome_count() < u64::from(self.shown) {
            Ok(())
        } else {
            Err(PersonalizationStoreError::SignalWithoutShow)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PersonalizationV1 {
    pub schema: String,
    pub revision: u64,
    pub records: Vec<DailyAggregate>,
}

impl PersonalizationV1 {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema: PERSONALIZATION_SCHEMA.to_owned(),
            revision: 0,
            records: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), PersonalizationValidationError> {
        if self.schema != PERSONALIZATION_SCHEMA {
            return Err(PersonalizationValidationError::UnsupportedSchema);
        }
        if self.revision > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::RevisionOutOfRange);
        }
        if self.records.len() > MAX_PERSONALIZATION_RECORDS {
            return Err(PersonalizationValidationError::TooManyRecords);
        }
        let mut previous: Option<&DailyAggregate> = None;
        for record in &self.records {
            record.validate()?;
            if previous.is_some_and(|candidate| {
                candidate.key_cmp(&record.identity, record.provider, record.day) != Ordering::Less
            }) {
                return Err(PersonalizationValidationError::RecordsNotCanonical);
            }
            previous = Some(record);
        }
        Ok(())
    }

    fn reconcile(&mut self, settings: &SettingsV1, today: u64) {
        self.records.retain(|record| {
            if record.day > today {
                return false;
            }
            let resolution = settings.resolve_identity_validated(&record.identity);
            if !resolution.configured || resolution.permissions.learn != PermissionDecision::Allow {
                return false;
            }
            let days = resolution.permissions.retention.days().map_or(1, u64::from);
            today.saturating_sub(record.day) < days
        });
    }

    fn durable_projection(&self, settings: &SettingsV1, today: u64, durable_revision: u64) -> Self {
        let mut projected = self.clone();
        projected.revision = durable_revision;
        projected
            .records
            .retain(|record| record_is_durable(record, settings, today));
        projected
    }

    fn record_signal(
        &mut self,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        day: u64,
        signal: PersonalizationSignal,
    ) -> Result<bool, PersonalizationStoreError> {
        if day > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::DayOutOfRange.into());
        }
        identity
            .validate()
            .map_err(PersonalizationValidationError::from)?;
        let index = match self
            .records
            .binary_search_by(|record| record.key_cmp(&identity, provider, day))
        {
            Ok(index) => index,
            Err(index) => {
                self.records
                    .insert(index, DailyAggregate::new(identity, provider, day));
                index
            }
        };
        self.records[index].record(signal)
    }

    fn enforce_record_limit(&mut self, settings: &SettingsV1, today: u64) {
        while self.records.len() > MAX_PERSONALIZATION_RECORDS {
            // Ephemeral records may consume only spare in-memory capacity. They
            // must never evict or rewrite another subject's durable history.
            let oldest_ephemeral =
                self.oldest_record_index(|record| !record_is_durable(record, settings, today));
            let oldest = oldest_ephemeral.or_else(|| self.oldest_record_index(|_| true));
            let oldest = oldest.expect("record limit exceeded with at least one record");
            self.records.remove(oldest);
        }
    }

    fn oldest_record_index(&self, include: impl Fn(&DailyAggregate) -> bool) -> Option<usize> {
        self.records
            .iter()
            .enumerate()
            .filter(|(_, record)| include(record))
            .min_by(|(_, left), (_, right)| {
                (left.day, &left.identity, left.provider).cmp(&(
                    right.day,
                    &right.identity,
                    right.provider,
                ))
            })
            .map(|(oldest, _)| oldest)
    }
}

fn record_is_durable(record: &DailyAggregate, settings: &SettingsV1, today: u64) -> bool {
    let resolution = settings.resolve_identity_validated(&record.identity);
    let RetentionPermission::Bounded { days } = resolution.permissions.retention else {
        return false;
    };
    record.day <= today && today.saturating_sub(record.day) < u64::from(days)
}

impl Default for PersonalizationV1 {
    fn default() -> Self {
        Self::empty()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonalizationMutation {
    pub revision: u64,
    pub changed: bool,
    pub signal_recorded: bool,
    pub signal_dropped: bool,
}

#[derive(Debug)]
pub(crate) struct PersonalizationStore {
    path: PathBuf,
    state: Mutex<PersonalizationStoreState>,
}

#[derive(Debug)]
struct PersonalizationStoreState {
    document: PersonalizationV1,
    durable_revision: u64,
    durable_records: Vec<DailyAggregate>,
}

impl PersonalizationStore {
    pub(crate) fn open(
        path: impl Into<PathBuf>,
        settings: &SettingsV1,
        today: u64,
    ) -> Result<Self, PersonalizationStoreError> {
        settings.validate()?;
        if today > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::DayOutOfRange.into());
        }
        let path = path.into();
        if !path.is_absolute() || path.parent().is_none() {
            return Err(PrivateStorageError::PathNotAbsolute.into());
        }
        ensure_private_directory(path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?)?;
        let mut document = match read_private_limited(&path, MAX_PERSONALIZATION_BYTES)? {
            Some(bytes) => decode_personalization(&bytes)?,
            None => PersonalizationV1::empty(),
        };
        let persisted_records = document.records.clone();
        let mut durable_revision = document.revision;
        document.reconcile(settings, today);
        let durable_records = document
            .durable_projection(settings, today, durable_revision)
            .records;
        if persisted_records != durable_records {
            document.revision = next_revision(document.revision)?;
            durable_revision = next_revision(durable_revision)?;
        }
        document.validate()?;
        persist_or_remove(&path, &document, settings, today, durable_revision)?;
        Ok(Self {
            path,
            state: Mutex::new(PersonalizationStoreState {
                document,
                durable_revision,
                durable_records,
            }),
        })
    }

    pub(crate) fn snapshot(&self) -> Result<PersonalizationV1, PersonalizationStoreError> {
        self.state
            .lock()
            .map(|state| state.document.clone())
            .map_err(|_| PersonalizationStoreError::LockPoisoned)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record(
        &self,
        expected_revision: u64,
        settings: &SettingsV1,
        today: u64,
        event_day: u64,
        identity: StableIdentity,
        provider: PersonalizationProvider,
        signal: PersonalizationSignal,
    ) -> Result<PersonalizationMutation, PersonalizationStoreError> {
        settings.validate()?;
        identity
            .validate()
            .map_err(PersonalizationValidationError::from)?;
        if today > MAX_SAFE_COUNTER || event_day > today {
            return Err(PersonalizationValidationError::DayOutOfRange.into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| PersonalizationStoreError::LockPoisoned)?;
        ensure_revision(state.document.revision, expected_revision)?;

        let mut next = state.document.clone();
        next.reconcile(settings, today);
        let resolution = settings.resolve_identity_validated(&identity);
        let retention_days = resolution.permissions.retention.days().map_or(1, u64::from);
        let signal_eligible =
            resolution.allows_learning() && today.saturating_sub(event_day) < retention_days;
        let signal_identity = identity.clone();
        let signal_changed = if signal_eligible {
            next.record_signal(identity, provider, event_day, signal)?
        } else {
            false
        };
        next.enforce_record_limit(settings, today);
        let signal_survived = signal_eligible
            && signal_changed
            && next
                .records
                .binary_search_by(|record| record.key_cmp(&signal_identity, provider, event_day))
                .is_ok();
        let signal_dropped = signal_eligible && signal_changed && !signal_survived;
        let changed = next.records != state.document.records;
        if !changed {
            return Ok(PersonalizationMutation {
                revision: state.document.revision,
                changed: false,
                signal_recorded: false,
                signal_dropped,
            });
        }
        next.revision = next_revision(state.document.revision)?;
        next.validate()?;
        let durable_after = next.durable_projection(settings, today, state.durable_revision);
        let mut durable_revision = state.durable_revision;
        if state.durable_records != durable_after.records {
            durable_revision = next_revision(durable_revision)?;
            persist_or_remove(&self.path, &next, settings, today, durable_revision)?;
        }
        state.document = next;
        state.durable_revision = durable_revision;
        state.durable_records = durable_after.records;
        Ok(PersonalizationMutation {
            revision: state.document.revision,
            changed: true,
            signal_recorded: signal_survived,
            signal_dropped,
        })
    }

    pub(crate) fn reconcile(
        &self,
        expected_revision: u64,
        settings: &SettingsV1,
        today: u64,
    ) -> Result<PersonalizationMutation, PersonalizationStoreError> {
        settings.validate()?;
        if today > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::DayOutOfRange.into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| PersonalizationStoreError::LockPoisoned)?;
        ensure_revision(state.document.revision, expected_revision)?;
        let mut next = state.document.clone();
        next.reconcile(settings, today);
        let memory_changed = next.records != state.document.records;
        let durable_after = next.durable_projection(settings, today, state.durable_revision);
        let durable_changed = state.durable_records != durable_after.records;
        if !memory_changed && !durable_changed {
            return Ok(PersonalizationMutation {
                revision: state.document.revision,
                changed: false,
                signal_recorded: false,
                signal_dropped: false,
            });
        }
        next.revision = next_revision(state.document.revision)?;
        next.validate()?;
        let mut durable_revision = state.durable_revision;
        if durable_changed {
            durable_revision = next_revision(durable_revision)?;
            persist_or_remove(&self.path, &next, settings, today, durable_revision)?;
        }
        state.document = next;
        state.durable_revision = durable_revision;
        state.durable_records = durable_after.records;
        Ok(PersonalizationMutation {
            revision: state.document.revision,
            changed: true,
            signal_recorded: false,
            signal_dropped: false,
        })
    }

    pub(crate) fn clear(
        &self,
        expected_revision: u64,
        settings: &SettingsV1,
        today: u64,
    ) -> Result<PersonalizationMutation, PersonalizationStoreError> {
        settings.validate()?;
        if today > MAX_SAFE_COUNTER {
            return Err(PersonalizationValidationError::DayOutOfRange.into());
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| PersonalizationStoreError::LockPoisoned)?;
        ensure_revision(state.document.revision, expected_revision)?;
        let mut next = state.document.clone();
        next.reconcile(settings, today);
        let before = next.records.len();
        next.records.clear();
        let changed = before != next.records.len() || next.records != state.document.records;
        if !changed {
            return Ok(PersonalizationMutation {
                revision: state.document.revision,
                changed: false,
                signal_recorded: false,
                signal_dropped: false,
            });
        }
        next.revision = next_revision(state.document.revision)?;
        next.validate()?;
        let durable_after = next.durable_projection(settings, today, state.durable_revision);
        let mut durable_revision = state.durable_revision;
        if state.durable_records != durable_after.records {
            durable_revision = next_revision(durable_revision)?;
            persist_or_remove(&self.path, &next, settings, today, durable_revision)?;
        }
        state.document = next;
        state.durable_revision = durable_revision;
        state.durable_records = durable_after.records;
        Ok(PersonalizationMutation {
            revision: state.document.revision,
            changed: true,
            signal_recorded: false,
            signal_dropped: false,
        })
    }

    #[cfg(test)]
    fn export(&self) -> Result<Vec<u8>, PersonalizationStoreError> {
        let state = self
            .state
            .lock()
            .map_err(|_| PersonalizationStoreError::LockPoisoned)?;
        state.document.validate()?;
        encode_personalization(&state.document)
    }
}

fn ensure_revision(actual: u64, expected: u64) -> Result<(), PersonalizationStoreError> {
    if actual == expected {
        Ok(())
    } else {
        Err(PersonalizationStoreError::RevisionConflict { expected, actual })
    }
}

fn next_revision(current: u64) -> Result<u64, PersonalizationValidationError> {
    current
        .checked_add(1)
        .filter(|revision| *revision <= MAX_SAFE_COUNTER)
        .ok_or(PersonalizationValidationError::RevisionOutOfRange)
}

fn decode_personalization(bytes: &[u8]) -> Result<PersonalizationV1, PersonalizationStoreError> {
    let state: PersonalizationV1 = serde_json::from_slice(bytes)?;
    state.validate()?;
    Ok(state)
}

fn encode_personalization(state: &PersonalizationV1) -> Result<Vec<u8>, PersonalizationStoreError> {
    // This is machine-owned state. Compact encoding lets every maximum-shape
    // valid 512-record document fit under the fixed private-file read limit.
    let mut bytes = serde_json::to_vec(state)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_PERSONALIZATION_BYTES {
        return Err(PersonalizationStoreError::EncodedTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn persist_or_remove(
    path: &Path,
    state: &PersonalizationV1,
    settings: &SettingsV1,
    today: u64,
    durable_revision: u64,
) -> Result<(), PersonalizationStoreError> {
    let durable = state.durable_projection(settings, today, durable_revision);
    durable.validate()?;
    if durable.records.is_empty() {
        let _ = remove_private_file(path)?;
    } else {
        atomic_write_private(path, &encode_personalization(&durable)?)?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum PersonalizationValidationError {
    #[error("day_out_of_range")]
    DayOutOfRange,
    #[error("identity")]
    Identity(#[from] crate::settings::IdentityError),
    #[error("invalid_counters")]
    InvalidCounters,
    #[error("records_not_canonical")]
    RecordsNotCanonical,
    #[error("revision_out_of_range")]
    RevisionOutOfRange,
    #[error("too_many_records")]
    TooManyRecords,
    #[error("unsupported_schema")]
    UnsupportedSchema,
}

#[derive(Debug, Error)]
pub enum PersonalizationStoreError {
    #[error("encoded_too_large:{0}")]
    EncodedTooLarge(usize),
    #[error("json")]
    Json(#[from] serde_json::Error),
    #[error("lock_poisoned")]
    LockPoisoned,
    #[error("revision_conflict:{expected}:{actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("settings")]
    Settings(#[from] SettingsValidationError),
    #[error("signal_without_show")]
    SignalWithoutShow,
    #[error("storage")]
    Storage(#[from] PrivateStorageError),
    #[error("validation")]
    Validation(#[from] PersonalizationValidationError),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::tempdir;

    use super::{
        DailyAggregate, MAX_PERSONALIZATION_BYTES, MAX_PERSONALIZATION_RECORDS,
        PERSONALIZATION_SCHEMA, PersonalizationProvider, PersonalizationSignal,
        PersonalizationStore, PersonalizationStoreError, PersonalizationV1,
    };
    use crate::protocol::MAX_SAFE_COUNTER;
    use crate::settings::{
        BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA, SettingsV1,
        StableIdentity, SubjectPermissions, SubjectRule, WebScheme,
    };

    fn identity(host: &str) -> StableIdentity {
        StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
            .expect("identity")
    }

    fn settings(retention: RetentionPermission) -> SettingsV1 {
        SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 1,
            paused: false,
            subjects: vec![SubjectRule {
                identity: identity("example.com"),
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

    #[test]
    fn strict_document_rejects_content_and_unknown_fields() {
        let raw = br#"{
          "schema":"badi.personalization.v1","revision":0,"records":[],
          "before":"private text"
        }"#;
        assert!(serde_json::from_slice::<PersonalizationV1>(raw).is_err());
        assert_eq!(PersonalizationV1::empty().schema, PERSONALIZATION_SCHEMA);
    }

    #[test]
    fn producer_document_matches_the_formal_json_schema() {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../schemas/badi.personalization.v1.schema.json"
        ))
        .expect("personalization schema JSON");
        let validator = jsonschema::validator_for(&schema).expect("personalization validator");
        let document = PersonalizationV1 {
            schema: PERSONALIZATION_SCHEMA.to_owned(),
            revision: 1,
            records: vec![DailyAggregate {
                identity: identity("example.com"),
                provider: PersonalizationProvider::PhraseV1,
                day: 10,
                shown: 2,
                dismissed: 1,
                accepted_word: 1,
                accepted_all: 0,
            }],
        };
        document.validate().expect("Rust document validation");
        let value = serde_json::to_value(document).expect("personalization JSON");
        if let Err(error) = validator.validate(&value) {
            panic!("personalization document failed formal schema: {error}");
        }
    }

    #[test]
    fn learning_requires_a_configured_unpaused_grant_and_prior_show() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let granted = settings(RetentionPermission::None);
        let store = PersonalizationStore::open(&path, &granted, 10).expect("store");
        assert!(matches!(
            store.record(
                0,
                &granted,
                10,
                10,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Dismissed,
            ),
            Err(PersonalizationStoreError::SignalWithoutShow)
        ));
        let shown = store
            .record(
                0,
                &granted,
                10,
                10,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("shown");
        assert!(shown.signal_recorded);
        let dismissed = store
            .record(
                shown.revision,
                &granted,
                10,
                10,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Dismissed,
            )
            .expect("dismissed");
        assert!(dismissed.signal_recorded);
        assert_eq!(store.snapshot().expect("snapshot").records[0].dismissed, 1);

        let mut paused = granted;
        paused.paused = true;
        let ignored = store
            .record(
                dismissed.revision,
                &paused,
                10,
                10,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("paused signal ignored");
        assert!(!ignored.signal_recorded);
    }

    #[test]
    fn retention_none_keeps_memory_only_and_creates_no_file() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let settings = settings(RetentionPermission::None);
        let store = PersonalizationStore::open(&path, &settings, 20).expect("store");
        store
            .record(
                0,
                &settings,
                20,
                20,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record");
        assert!(!path.exists());
        assert_eq!(store.snapshot().expect("snapshot").records.len(), 1);
    }

    #[test]
    fn memory_only_subject_does_not_rewrite_another_subjects_durable_file() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let mut mixed = settings(RetentionPermission::Bounded { days: 30 });
        mixed.subjects.push(SubjectRule {
            identity: identity("memory-only.example"),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention: RetentionPermission::None,
            },
        });
        mixed
            .subjects
            .sort_by(|left, right| left.identity.cmp(&right.identity));
        mixed.validate().expect("mixed settings");
        let store = PersonalizationStore::open(&path, &mixed, 20).expect("store");
        let durable = store
            .record(
                0,
                &mixed,
                20,
                20,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("durable show");
        let durable_bytes = fs::read(&path).expect("durable bytes");

        let ephemeral = store
            .record(
                durable.revision,
                &mixed,
                20,
                20,
                identity("memory-only.example"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("memory-only show");
        assert!(ephemeral.signal_recorded);
        assert_eq!(
            fs::read(&path).expect("unchanged durable bytes"),
            durable_bytes
        );

        let durable_again = store
            .record(
                ephemeral.revision,
                &mixed,
                20,
                20,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("second durable show");
        assert_eq!(durable_again.revision, 3);
        let persisted: PersonalizationV1 =
            serde_json::from_slice(&fs::read(&path).expect("updated durable personalization"))
                .expect("persisted document");
        assert_eq!(persisted.revision, 2);
        assert_eq!(persisted.records.len(), 1);
        assert_eq!(persisted.records[0].shown, 2);
        assert_eq!(persisted.records[0].identity, identity("example.com"));

        drop(store);
        let reopened = PersonalizationStore::open(&path, &mixed, 20).expect("reopened store");
        assert_eq!(reopened.snapshot().expect("reopened snapshot"), persisted);
    }

    #[test]
    fn bounded_retention_is_private_atomic_and_contains_no_document_text() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let settings = settings(RetentionPermission::Bounded { days: 30 });
        let store = PersonalizationStore::open(&path, &settings, 30).expect("store");
        store
            .record(
                0,
                &settings,
                30,
                30,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record");
        let metadata = fs::metadata(&path).expect("metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        let bytes = fs::read(&path).expect("persisted bytes");
        let text = String::from_utf8(bytes).expect("JSON text");
        for forbidden in [
            "before",
            "after",
            "fingerprint",
            "suggestion_id",
            "private phrase",
        ] {
            assert!(!text.contains(forbidden));
        }
    }

    #[test]
    fn expiry_and_retention_revocation_remove_the_file() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let retained = settings(RetentionPermission::Bounded { days: 2 });
        let store = PersonalizationStore::open(&path, &retained, 40).expect("store");
        let mutation = store
            .record(
                0,
                &retained,
                40,
                40,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record");
        assert!(path.exists());
        let expired = store
            .reconcile(mutation.revision, &retained, 42)
            .expect("expire");
        assert!(expired.changed);
        assert!(!path.exists());

        let ephemeral = settings(RetentionPermission::None);
        store
            .reconcile(expired.revision, &ephemeral, 42)
            .expect("retention disabled");
        assert!(!path.exists());
    }

    #[test]
    fn clear_is_scoped_revisioned_and_export_is_strict_json() {
        let temporary = tempdir().expect("temporary directory");
        let path = temporary.path().join("badi/personalization.json");
        let settings = settings(RetentionPermission::Bounded { days: 30 });
        let store = PersonalizationStore::open(&path, &settings, 50).expect("store");
        let recorded = store
            .record(
                0,
                &settings,
                50,
                50,
                identity("example.com"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("record");
        assert!(matches!(
            store.clear(0, &settings, 50),
            Err(PersonalizationStoreError::RevisionConflict { .. })
        ));
        let cleared = store
            .clear(recorded.revision, &settings, 50)
            .expect("clear identity");
        assert!(cleared.changed);
        assert!(!path.exists());
        let exported: PersonalizationV1 =
            serde_json::from_slice(&store.export().expect("export")).expect("strict export");
        assert!(exported.records.is_empty());
        assert_eq!(exported.revision, cleared.revision);
    }

    #[test]
    fn maximum_valid_document_fits_the_private_file_limit() {
        let records = (0..MAX_PERSONALIZATION_RECORDS)
            .map(|index| {
                let host = format!(
                    "h{index:03}.{}.{}.{}.{}",
                    "a".repeat(63),
                    "b".repeat(63),
                    "c".repeat(63),
                    "d".repeat(56)
                );
                assert_eq!(host.len(), 253);
                DailyAggregate {
                    identity: identity(&host),
                    provider: PersonalizationProvider::LocalModel,
                    day: MAX_SAFE_COUNTER,
                    shown: u32::MAX,
                    dismissed: 1_431_655_765,
                    accepted_word: 1_431_655_765,
                    accepted_all: 1_431_655_765,
                }
            })
            .collect();
        let state = PersonalizationV1 {
            schema: PERSONALIZATION_SCHEMA.to_owned(),
            revision: MAX_SAFE_COUNTER,
            records,
        };

        state.validate().expect("maximum-shape document");
        let encoded = super::encode_personalization(&state).expect("bounded encoding");
        assert!(encoded.len() <= MAX_PERSONALIZATION_BYTES);
        assert_eq!(
            super::decode_personalization(&encoded).expect("round trip"),
            state
        );
    }

    #[test]
    fn ephemeral_capacity_cannot_evict_durable_records() {
        let durable_hosts = [
            "durable-a.example",
            "durable-b.example",
            "durable-c.example",
        ];
        let mut subjects: Vec<_> = durable_hosts
            .iter()
            .map(|host| SubjectRule {
                identity: identity(host),
                permissions: SubjectPermissions {
                    suggest: PermissionDecision::Allow,
                    display: PermissionDecision::Allow,
                    context_read: PermissionDecision::Allow,
                    learn: PermissionDecision::Allow,
                    retention: RetentionPermission::Bounded { days: 90 },
                },
            })
            .collect();
        subjects.push(SubjectRule {
            identity: identity("ephemeral.example"),
            permissions: SubjectPermissions {
                suggest: PermissionDecision::Allow,
                display: PermissionDecision::Allow,
                context_read: PermissionDecision::Allow,
                learn: PermissionDecision::Allow,
                retention: RetentionPermission::None,
            },
        });
        subjects.sort_by(|left, right| left.identity.cmp(&right.identity));
        let mixed = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 1,
            paused: false,
            subjects,
        };
        mixed.validate().expect("mixed settings");

        let mut state = PersonalizationV1::empty();
        for host in durable_hosts {
            for provider in [
                PersonalizationProvider::PhraseV1,
                PersonalizationProvider::LocalModel,
            ] {
                for day in 1..=90 {
                    state
                        .record_signal(identity(host), provider, day, PersonalizationSignal::Shown)
                        .expect("durable signal");
                    state.enforce_record_limit(&mixed, 90);
                }
            }
        }
        assert_eq!(state.records.len(), MAX_PERSONALIZATION_RECORDS);
        let durable_before = state.durable_projection(&mixed, 90, 7);

        state
            .record_signal(
                identity("ephemeral.example"),
                PersonalizationProvider::PhraseV1,
                90,
                PersonalizationSignal::Shown,
            )
            .expect("ephemeral signal");
        state.enforce_record_limit(&mixed, 90);
        let durable_after = state.durable_projection(&mixed, 90, 7);

        assert_eq!(state.records.len(), MAX_PERSONALIZATION_RECORDS);
        assert_eq!(durable_after, durable_before);
        assert!(
            state
                .records
                .iter()
                .all(|record| record.identity != identity("ephemeral.example"))
        );

        let temporary = tempdir().expect("temporary directory");
        let directory = temporary.path().join("badi");
        fs::create_dir(&directory).expect("private directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("private directory permissions");
        let path = directory.join("personalization.json");
        fs::write(
            &path,
            super::encode_personalization(&state).expect("encoded full store"),
        )
        .expect("seed full store");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("private file permissions");
        let store = PersonalizationStore::open(&path, &mixed, 90).expect("full store");
        let before = fs::read(&path).expect("full durable bytes");
        let mutation = store
            .record(
                0,
                &mixed,
                90,
                90,
                identity("ephemeral.example"),
                PersonalizationProvider::PhraseV1,
                PersonalizationSignal::Shown,
            )
            .expect("bounded admission result");
        assert!(!mutation.changed);
        assert!(!mutation.signal_recorded);
        assert!(mutation.signal_dropped);
        assert_eq!(fs::read(&path).expect("unchanged durable bytes"), before);
    }
}
