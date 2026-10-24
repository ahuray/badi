use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use tempfile::tempdir;

use super::{
    DailyAggregate, MAX_PERSONALIZATION_BYTES, MAX_PERSONALIZATION_RECORDS, PERSONALIZATION_SCHEMA,
    PersonalizationProvider, PersonalizationSignal, PersonalizationStore,
    PersonalizationStoreError, PersonalizationV1,
};
use crate::protocol::MAX_SAFE_COUNTER;
use crate::settings::{
    BrowserAdapter, LinuxAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA,
    SettingsV2, StableIdentity, SubjectPermissions, SubjectRule, WebScheme,
};

fn identity(host: &str) -> StableIdentity {
    StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
        .expect("identity")
}

fn settings(retention: RetentionPermission) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
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
        "../../schemas/badi.personalization.v1.schema.json"
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
fn personalization_v1_rejects_linux_application_identities() {
    let document = PersonalizationV1 {
        schema: PERSONALIZATION_SCHEMA.to_owned(),
        revision: 1,
        records: vec![DailyAggregate {
            identity: StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite")
                .expect("Linux identity"),
            provider: PersonalizationProvider::PhraseV1,
            day: 10,
            shown: 1,
            dismissed: 0,
            accepted_word: 0,
            accepted_all: 0,
        }],
    };

    assert!(document.validate().is_err());
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
    assert_eq!(exported.records, [] as [DailyAggregate; 0]);
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
    let mixed = SettingsV2 {
        revision: 1,
        paused: false,
        subjects,
        ..SettingsV2::deny_by_default()
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
