use std::fs;
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
use std::sync::Arc;
use std::time::Duration;

use tempfile::tempdir;

use super::{ControlPlane, ControlPlaneError, strictly_reduces_authority, unix_day};
use crate::personalization::{DailyAggregate, PersonalizationProvider, PersonalizationSignal};
use crate::settings::{
    ALL_WEB_ORIGINS_PERMISSIONS, BrowserAdapter, PermissionDecision, PrivateStorage,
    PrivateStorageError, RetentionPermission, SETTINGS_SCHEMA, SettingsStoreError, SettingsV2,
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

#[test]
fn unsafe_settings_are_named_and_do_not_restart() {
    let unsafe_file = ControlPlaneError::Storage(PrivateStorageError::UnsafeFile);
    assert_eq!(unsafe_file.startup_name(), "unsafe_file");
    assert!(unsafe_file.restart_cannot_help());
    let locked = ControlPlaneError::Storage(PrivateStorageError::LockUnavailable);
    assert_eq!(locked.startup_name(), "lock_unavailable");
    assert!(!locked.restart_cannot_help());
    let corrupt = ControlPlaneError::Settings(SettingsStoreError::Validation(
        SettingsValidationError::UnsupportedSchema,
    ));
    assert_eq!(corrupt.startup_name(), "validation");
    assert!(corrupt.restart_cannot_help());
}

fn learning_settings(revision: u64, retention: RetentionPermission) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        style_learning: false,
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
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

fn granted_settings(revision: u64) -> SettingsV2 {
    learning_settings(revision, RetentionPermission::Bounded { days: 30 })
}

#[test]
fn the_all_app_default_is_authority_like_the_all_web_default() {
    let current = granted_settings(7);
    let mut all_apps = current.clone();
    all_apps.all_linux_apps = true;
    let mut all_apps_off = all_apps.clone();
    all_apps_off.revision = 8;
    all_apps_off.all_linux_apps = false;
    assert!(strictly_reduces_authority(&all_apps, &all_apps_off));
    let mut paused = current.clone();
    paused.revision = 8;
    paused.paused = true;
    let mut blocked = current.clone();
    blocked.revision = 8;
    blocked.subjects[0].permissions = SubjectPermissions::deny_all();
    for mut grant in [paused, blocked] {
        grant.all_linux_apps = true;
        assert!(
            !strictly_reduces_authority(&current, &grant),
            "turning the app default on is a grant even with a pause or a revoke"
        );
    }
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
    shorter_retention.subjects[0].permissions.retention = RetentionPermission::Bounded { days: 7 };
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
    retention_growth.subjects[0].permissions.retention = RetentionPermission::Bounded { days: 30 };
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

    // `badi site all off` stays possible while the memory store is
    // unavailable; turning the all-web default on never is, not even
    // together with a pause or a revoke.
    let mut all_web = current.clone();
    all_web.all_web_origins = true;
    let mut all_web_off = all_web.clone();
    all_web_off.revision = 8;
    all_web_off.all_web_origins = false;
    assert!(strictly_reduces_authority(&all_web, &all_web_off));
    for mut grant in [paused.clone(), blocked.clone(), no_op.clone()] {
        grant.all_web_origins = true;
        assert!(!strictly_reduces_authority(&current, &grant));
    }

    // Under the all-web default an unlisted origin is allowed, so an
    // explicit origin block is a reduction, removing it is a grant, and a
    // tombstone equal to the default is a no-op.
    let block_other = SubjectRule {
        identity: other_identity(),
        permissions: SubjectPermissions::deny_all(),
    };
    let mut blocked_other = all_web.clone();
    blocked_other.revision = 8;
    blocked_other.subjects.push(block_other);
    assert!(strictly_reduces_authority(&all_web, &blocked_other));
    let mut unblocked_other = blocked_other.clone();
    unblocked_other.revision = 9;
    unblocked_other.subjects.pop();
    assert!(!strictly_reduces_authority(
        &blocked_other,
        &unblocked_other
    ));
    let mut revoke_and_unblock = unblocked_other.clone();
    revoke_and_unblock.subjects[0].permissions = SubjectPermissions::deny_all();
    assert!(!strictly_reduces_authority(
        &blocked_other,
        &revoke_and_unblock
    ));
    let mut default_tombstone = all_web.clone();
    default_tombstone.revision = 8;
    default_tombstone.subjects.push(SubjectRule {
        identity: other_identity(),
        permissions: ALL_WEB_ORIGINS_PERMISSIONS,
    });
    assert!(!strictly_reduces_authority(&all_web, &default_tombstone));
    let mut learning_other = default_tombstone.clone();
    learning_other.subjects[1].permissions.learn = PermissionDecision::Allow;
    assert!(!strictly_reduces_authority(&all_web, &learning_other));
}

#[test]
fn initializes_deny_by_default_and_retains_the_storage_lock() {
    let temporary = tempdir().expect("temporary directory");
    let storage_paths = paths(temporary.path());
    let control = ControlPlane::open(storage_paths.clone()).expect("control plane");
    let snapshot = control.snapshot().expect("snapshot");
    assert_eq!(snapshot.settings, SettingsV2::deny_by_default());
    assert_eq!(snapshot.personalization.records, [] as [DailyAggregate; 0]);
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

    let denied = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 2,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        style_learning: false,
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: Vec::new(),
    };
    control
        .replace_settings(1, denied)
        .expect("revoke settings");
    let snapshot = control.snapshot().expect("reconciled snapshot");
    assert_eq!(snapshot.personalization.records, [] as [DailyAggregate; 0]);
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
    assert_eq!(
        after_grant.personalization.records,
        [] as [DailyAggregate; 0]
    );
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
    let stale_replacement = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 100,
        paused: true,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        style_learning: false,
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
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
    let over_capacity = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 2,
        paused: true,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        style_learning: false,
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
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
    assert_eq!(
        control
            .snapshot()
            .expect("cleared snapshot")
            .personalization
            .records,
        [] as [DailyAggregate; 0]
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
    assert_eq!(
        control
            .snapshot()
            .expect("snapshot")
            .personalization
            .records,
        [] as [DailyAggregate; 0]
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
    assert_eq!(
        durable
            .snapshot()
            .expect("expired durable snapshot")
            .personalization
            .records,
        [] as [DailyAggregate; 0]
    );

    let ephemeral_paths = paths(&temporary.path().join("ephemeral"));
    let ephemeral = ControlPlane::open(ephemeral_paths.clone()).expect("ephemeral control plane");
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
    assert_eq!(
        ephemeral
            .snapshot()
            .expect("expired ephemeral snapshot")
            .personalization
            .records,
        [] as [DailyAggregate; 0]
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
    assert_eq!(
        unavailable.personalization.records,
        [] as [DailyAggregate; 0]
    );
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

    let denied = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 2,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: crate::settings::BatteryMode::Normal,
        style_learning: false,
        language_overrides: std::collections::BTreeMap::new(),
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
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
    let persisted_settings: SettingsV2 = serde_json::from_slice(
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
    assert_eq!(recovered.personalization.records, [] as [DailyAggregate; 0]);
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
