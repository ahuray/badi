use std::fs;
use std::os::unix::fs::PermissionsExt as _;

use tempfile::tempdir;

use super::private_file::{PRIVATE_FILE_MODE, remove_private_file_with_sync};
use super::store::write_settings;
use super::{
    ALL_LINUX_APPS_PERMISSIONS, BatteryMode, BrowserAdapter, LinuxAdapter,
    MAX_ESCAPE_QUIET_SECONDS, MAX_SETTINGS_BYTES, MAX_SUBJECTS, PermissionDecision,
    PredictionLanguage, PrivateStorage, PrivateStorageError, RetentionPermission, SETTINGS_SCHEMA,
    SETTINGS_SCHEMA_V1, ScreenShareMode, SettingsStoreError, SettingsV2, SettingsValidationError,
    StableIdentity, StoragePaths, SubjectPermissions, SubjectRule, SuggestionLength, WebScheme,
};
use crate::protocol::{
    MAX_FRAME_BYTES, Origin, OriginScheme, TargetDescriptor, TargetKind, WireEnvelope,
};

fn allowed(days: Option<u16>) -> SubjectPermissions {
    SubjectPermissions {
        suggest: PermissionDecision::Allow,
        display: PermissionDecision::Allow,
        context_read: PermissionDecision::Allow,
        learn: PermissionDecision::Allow,
        retention: days.map_or(RetentionPermission::None, |days| {
            RetentionPermission::Bounded { days }
        }),
    }
}

fn identity(host: &str) -> StableIdentity {
    StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
        .expect("identity")
}

fn settings_with(rule: SubjectRule) -> SettingsV2 {
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: vec![rule],
    }
}

#[test]
fn missing_settings_are_paused_and_deny_everything() {
    let settings = SettingsV2::deny_by_default();
    settings.validate().expect("safe settings");
    let resolution = settings.resolve_identity(&identity("example.com"));
    assert!(resolution.identity_known);
    assert!(!resolution.configured());
    assert!(!resolution.allows_context_read());
    assert!(!resolution.allows_suggestion());
    assert!(!resolution.allows_display());
    assert!(!resolution.allows_learning());
}

#[test]
fn all_linux_apps_allows_unlisted_apps_by_default_but_exact_rules_win() {
    let app = |id: &str| StableIdentity::linux_app(LinuxAdapter::Fcitx, id).expect("app");
    let mut settings = settings_with(SubjectRule {
        identity: app("discord"),
        permissions: SubjectPermissions::deny_all(),
    });
    settings.subjects.push(SubjectRule {
        identity: app("omawrite"),
        permissions: ALL_LINUX_APPS_PERMISSIONS,
    });
    settings.validate().expect("valid settings");
    for all_linux_apps in [false, true] {
        settings.all_linux_apps = all_linux_apps;
        let unlisted = settings.resolve_identity(&app("libreoffice"));
        assert_eq!(
            (unlisted.configured(), unlisted.by_default()),
            (all_linux_apps, all_linux_apps)
        );
        assert_eq!(unlisted.allows_suggestion(), all_linux_apps);
        assert!(!unlisted.allows_learning(), "never learns by default");
        let blocked = settings.resolve_identity(&app("discord"));
        assert!(blocked.configured() && !blocked.by_default() && !blocked.allows_context_read());
        let listed = settings.resolve_identity(&app("omawrite"));
        assert!(
            listed.allows_suggestion() && !listed.by_default(),
            "an exact allow is never a default"
        );
        let site = settings.resolve_identity(&identity("mail.example"));
        assert!(!site.configured(), "websites keep their own mode");
    }
}

#[test]
fn all_web_origins_allows_unlisted_web_origins_but_exact_rules_win() {
    let blocked = SubjectRule {
        identity: identity("bank.example"),
        permissions: SubjectPermissions::deny_all(),
    };
    let mut settings = settings_with(blocked);
    settings.subjects.push(SubjectRule {
        identity: StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite").expect("app"),
        permissions: SubjectPermissions::deny_all(),
    });
    settings.validate().expect("valid settings");
    let http = StableIdentity::browser_origin(
        BrowserAdapter::Chromium,
        WebScheme::Http,
        "localhost",
        Some(8080),
    )
    .expect("http origin");
    let unlisted_app =
        StableIdentity::linux_app(LinuxAdapter::Fcitx, "brave-browser").expect("app");
    for all_web_origins in [false, true] {
        settings.all_web_origins = all_web_origins;
        for origin in [identity("mail.example"), http.clone()] {
            let resolution = settings.resolve_identity(&origin);
            assert_eq!(resolution.configured(), all_web_origins);
            assert_eq!(resolution.allows_context_read(), all_web_origins);
            assert_eq!(resolution.allows_display(), all_web_origins);
            assert_eq!(resolution.allows_suggestion(), all_web_origins);
            assert!(!resolution.allows_learning(), "never learns by default");
            assert_eq!(resolution.permissions.retention, RetentionPermission::None);
        }
        // An exact block and every native app keep their own resolution.
        assert!(
            !settings
                .resolve_identity(&identity("bank.example"))
                .allows_context_read()
        );
        for app in [
            StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite").expect("app"),
            unlisted_app.clone(),
        ] {
            assert!(!settings.resolve_identity(&app).allows_context_read());
        }
    }
    settings.paused = true;
    assert!(
        !settings
            .resolve_identity(&identity("mail.example"))
            .allows_context_read()
    );
}

#[test]
fn legacy_browser_clients_neither_see_nor_erase_the_all_web_default() {
    let mut current = settings_with(SubjectRule {
        identity: StableIdentity::linux_app(LinuxAdapter::Fcitx, "omawrite").expect("app"),
        permissions: super::ALL_WEB_ORIGINS_PERMISSIONS,
    });
    current.all_web_origins = true;
    current.mid_line = true;
    current.escape_quiet_seconds = 60;
    current
        .language_overrides
        .insert("omawrite".to_owned(), super::PredictionLanguage::German);
    let legacy = current.wire_document(1).expect("v1 wire document");
    assert!(legacy.get("all_web_origins").is_none());
    assert_eq!(legacy["subjects"], serde_json::json!([]));
    let next: SettingsV2 = serde_json::from_value(legacy).expect("legacy replacement");
    assert!(!next.all_web_origins);
    let merged = next.preserving_v2_policy_from(&current);
    assert!(merged.all_web_origins);
    assert_eq!(merged.subjects, current.subjects);
    assert_eq!(merged.language_overrides, current.language_overrides);
    assert_eq!(merged.mid_line, current.mid_line);
    assert_eq!(merged.escape_quiet_seconds, current.escape_quiet_seconds);
    assert_eq!(
        current.wire_document(2).expect("v2 wire document")["all_web_origins"],
        true
    );
}

#[test]
fn escape_quiet_seconds_round_trips_bounds_and_rejects_v1() {
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(
        encoded.get("escape_quiet_seconds").is_none(),
        "zero is omitted"
    );

    settings.escape_quiet_seconds = 90;
    settings.validate().expect("90 seconds is in range");
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert_eq!(encoded["escape_quiet_seconds"], 90);
    let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
    assert_eq!(decoded, settings);

    settings.escape_quiet_seconds = MAX_ESCAPE_QUIET_SECONDS + 1;
    assert_eq!(
        settings.validate(),
        Err(SettingsValidationError::EscapeQuietOutOfRange)
    );

    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "escape_quiet_seconds": 90, "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
}

#[test]
fn spelling_languages_omit_english_alone_and_must_be_canonical() {
    use crate::spelling::SpellingLanguage::{English, French, German};
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(
        encoded.get("spelling_languages").is_none(),
        "English alone is omitted"
    );
    settings.spelling_languages = vec![English, German, French];
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert_eq!(
        encoded["spelling_languages"],
        serde_json::json!(["en", "de", "fr"])
    );
    assert_eq!(
        serde_json::from_value::<SettingsV2>(encoded).expect("decode"),
        settings
    );
    settings.spelling_languages = vec![French, German];
    assert_eq!(
        settings.validate(),
        Err(SettingsValidationError::SpellingLanguagesNotCanonical)
    );
    settings.spelling_languages = Vec::new();
    assert_eq!(settings.validate(), Ok(()), "spelling can be off");
    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "spelling_languages": ["en"], "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
}

#[test]
fn battery_mode_round_trips_omits_quiet_and_rejects_v1() {
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    settings.battery_mode = BatteryMode::default();
    assert_eq!(settings.battery_mode, BatteryMode::Quiet);
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(encoded.get("battery_mode").is_none(), "quiet is omitted");
    let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
    assert_eq!(decoded.battery_mode, BatteryMode::Quiet);
    for (mode, wire) in [
        (BatteryMode::Normal, "normal"),
        (BatteryMode::ExplicitOnly, "explicit"),
    ] {
        settings.battery_mode = mode;
        let encoded = serde_json::to_value(&settings).expect("encode settings");
        assert_eq!(encoded["battery_mode"], wire);
        let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
        assert_eq!(decoded, settings);
    }
    let mut explicit_quiet = serde_json::to_value(&settings).expect("encode settings");
    explicit_quiet["battery_mode"] = serde_json::json!("quiet");
    let decoded: SettingsV2 = serde_json::from_value(explicit_quiet).expect("quiet");
    assert_eq!(decoded.battery_mode, BatteryMode::Quiet);

    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "battery_mode": "quiet", "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
    let unknown = serde_json::json!({
        "schema": SETTINGS_SCHEMA, "revision": 1, "paused": false,
        "battery_mode": "off", "subjects": [],
    });
    assert!(serde_json::from_value::<SettingsV2>(unknown).is_err());
}

#[test]
fn screen_share_round_trips_omits_hold_and_survives_a_v1_write() {
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    assert_eq!(settings.screen_share, ScreenShareMode::Hold);
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(encoded.get("screen_share").is_none(), "hold is omitted");
    settings.screen_share = ScreenShareMode::Suggest;
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert_eq!(encoded["screen_share"], "suggest");
    let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
    assert_eq!(decoded, settings);
    let mut explicit_hold = serde_json::to_value(&settings).expect("encode settings");
    explicit_hold["screen_share"] = serde_json::json!("hold");
    let decoded: SettingsV2 = serde_json::from_value(explicit_hold).expect("hold");
    assert_eq!(decoded.screen_share, ScreenShareMode::Hold);
    assert_eq!(
        SettingsV2::deny_by_default().screen_share,
        ScreenShareMode::Hold
    );

    // A protocol v1 client manages browser origins only and keeps the choice.
    let v1_write = SettingsV2::deny_by_default().preserving_v2_policy_from(&settings);
    assert_eq!(v1_write.screen_share, ScreenShareMode::Suggest);

    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "screen_share": "hold", "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
    for value in [serde_json::json!("off"), serde_json::json!(false)] {
        let unknown = serde_json::json!({
            "schema": SETTINGS_SCHEMA, "revision": 1, "paused": false,
            "screen_share": value, "subjects": [],
        });
        assert!(serde_json::from_value::<SettingsV2>(unknown).is_err());
    }
}

#[test]
fn suggestion_length_round_trips_omits_normal_and_survives_a_v1_write() {
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    assert_eq!(settings.suggestion_length, SuggestionLength::Normal);
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(
        encoded.get("suggestion_length").is_none(),
        "normal is omitted"
    );
    settings.validate().expect("normal");
    settings.suggestion_length = SuggestionLength::Short;
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert_eq!(encoded["suggestion_length"], "short");
    let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
    assert_eq!(decoded, settings);
    decoded.validate().expect("short");
    assert_eq!(SuggestionLength::Short.max_words(), 2);
    let mut explicit_normal = serde_json::to_value(&settings).expect("encode settings");
    explicit_normal["suggestion_length"] = serde_json::json!("normal");
    let decoded: SettingsV2 = serde_json::from_value(explicit_normal).expect("normal");
    assert_eq!(decoded.suggestion_length, SuggestionLength::Normal);
    assert_eq!(SuggestionLength::Normal.max_words(), 4);

    // A protocol v1 client manages browser origins only and keeps the length.
    let v1_write = SettingsV2::deny_by_default().preserving_v2_policy_from(&settings);
    assert_eq!(v1_write.suggestion_length, SuggestionLength::Short);

    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "suggestion_length": "short", "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
    // Longer suggestions doubled harmful additions in the Lab; none decode.
    for value in [
        serde_json::json!("long"),
        serde_json::json!("medium"),
        serde_json::json!(4),
    ] {
        let unknown = serde_json::json!({
            "schema": SETTINGS_SCHEMA, "revision": 1, "paused": false,
            "suggestion_length": value, "subjects": [],
        });
        assert!(serde_json::from_value::<SettingsV2>(unknown).is_err());
    }
}

#[test]
fn the_language_answer_round_trips_omits_false_and_survives_a_v1_write() {
    let mut settings = settings_with(SubjectRule {
        identity: identity("mail.example"),
        permissions: allowed(None),
    });
    assert!(!settings.languages_chosen);
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert!(
        encoded.get("languages_chosen").is_none(),
        "false is omitted"
    );
    settings.languages_chosen = true;
    let encoded = serde_json::to_value(&settings).expect("encode settings");
    assert_eq!(encoded["languages_chosen"], true);
    let decoded: SettingsV2 = serde_json::from_value(encoded).expect("decode settings");
    assert_eq!(decoded, settings);
    let v1_write = SettingsV2::deny_by_default().preserving_v2_policy_from(&settings);
    assert!(
        v1_write.languages_chosen,
        "a protocol v1 client keeps the answer"
    );
    let legacy = serde_json::json!({
        "schema": SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "languages_chosen": true, "subjects": [],
    });
    assert!(
        serde_json::from_value::<SettingsV2>(legacy).is_err(),
        "v1 cannot name it"
    );
    let unknown = serde_json::json!({
        "schema": SETTINGS_SCHEMA, "revision": 1, "paused": false,
        "languages_chosen": "yes", "subjects": [],
    });
    assert!(serde_json::from_value::<SettingsV2>(unknown).is_err());
}

#[test]
fn english_and_german_are_the_default_writing_languages_and_persian_is_opt_in() {
    let defaults = SettingsV2::deny_by_default();
    assert_eq!(
        defaults.writing_languages,
        [PredictionLanguage::English, PredictionLanguage::German]
    );
    let preferences = defaults.language_preferences();
    assert_eq!(preferences, crate::writing::LanguagePreferences::default());
    assert!(!preferences.enables(crate::writing::WritingLanguage::Persian));
    let mut persian = defaults.clone();
    persian.writing_languages = vec![
        PredictionLanguage::English,
        PredictionLanguage::German,
        PredictionLanguage::Persian,
    ];
    let encoded = serde_json::to_value(&persian).expect("encode settings");
    assert_eq!(
        encoded["writing_languages"],
        serde_json::json!(["en", "de", "fa"])
    );
    assert!(
        serde_json::to_value(&defaults)
            .expect("encode settings")
            .get("writing_languages")
            .is_none(),
        "English and German are omitted"
    );
}

#[test]
fn browser_target_resolution_is_exact_and_canonical() {
    let target = TargetDescriptor {
        kind: TargetKind::Browser,
        app_id: "chromium".to_owned(),
        target_id: "opaque-session".to_owned(),
        origin: Some(Origin {
            scheme: OriginScheme::Https,
            host: "EXAMPLE.COM".to_owned(),
            port: None,
        }),
    };
    assert_eq!(
        StableIdentity::from_target(&target).expect("known browser origin"),
        identity("example.com")
    );
    for app in ["chromium-browser", "brave-origin", "zen"] {
        assert_eq!(
            StableIdentity::from_target(&TargetDescriptor {
                app_id: app.to_owned(),
                ..target.clone()
            })
            .expect("shared site policy"),
            identity("example.com")
        );
    }

    for rejected in [
        TargetDescriptor {
            origin: None,
            ..target.clone()
        },
        TargetDescriptor {
            kind: TargetKind::Terminal,
            ..target.clone()
        },
        TargetDescriptor {
            app_id: "claimed-app".to_owned(),
            ..target.clone()
        },
    ] {
        assert!(StableIdentity::from_target(&rejected).is_err());
    }
}

#[test]
fn desktop_target_resolution_uses_exact_canonical_fcitx_program_identity() {
    let target = TargetDescriptor {
        kind: TargetKind::DesktopApplication,
        app_id: "com.github.xournalpp.xournalpp".to_owned(),
        target_id: "ic:42".to_owned(),
        origin: None,
    };
    assert_eq!(
        StableIdentity::from_target(&target).expect("known Linux app"),
        StableIdentity::linux_app(LinuxAdapter::Fcitx, &target.app_id).expect("canonical identity")
    );

    for app_id in [
        "",
        "Omawrite",
        "omawrite window",
        ".omawrite",
        "omawrite.",
        "omawrite.1editor",
        "omawrite._editor",
        "omawrite.-editor",
    ] {
        assert!(StableIdentity::linux_app(LinuxAdapter::Fcitx, app_id).is_err());
    }
    for app_id in [
        "omawrite",
        "com.github.xournalpp.xournalpp",
        "omawrite.editor_",
    ] {
        StableIdentity::linux_app(LinuxAdapter::Fcitx, app_id)
            .expect("schema-compatible Linux app identity");
    }
    let mut with_origin = target;
    with_origin.origin = Some(Origin {
        scheme: OriginScheme::Https,
        host: "example.com".to_owned(),
        port: None,
    });
    assert!(StableIdentity::from_target(&with_origin).is_err());
}

#[test]
fn editor_targets_require_their_exact_adapter_identity() {
    for (kind, adapter, app) in [
        (TargetKind::Obsidian, LinuxAdapter::Obsidian, "obsidian"),
        (TargetKind::Terminal, LinuxAdapter::Shell, "bash"),
        (TargetKind::Neovim, LinuxAdapter::Neovim, "nvim"),
        (
            TargetKind::Libreoffice,
            LinuxAdapter::Libreoffice,
            "libreoffice-writer",
        ),
    ] {
        let mut target = TargetDescriptor {
            kind,
            app_id: app.to_owned(),
            target_id: "buffer".to_owned(),
            origin: None,
        };
        assert_eq!(
            StableIdentity::from_target(&target).unwrap(),
            StableIdentity::linux_app(adapter, app).unwrap()
        );
        target.app_id = "other".to_owned();
        assert!(StableIdentity::from_target(&target).is_err());
        target.app_id = app.to_owned();
        target.origin = Some(Origin {
            scheme: OriginScheme::Https,
            host: "example.com".to_owned(),
            port: None,
        });
        assert!(StableIdentity::from_target(&target).is_err());
    }
}

#[test]
fn identity_rejects_ambiguous_hosts_and_normalizes_ip_literals() {
    for host in [
        "",
        ".example.com",
        "example.com.",
        "bad_label",
        "-bad.example",
    ] {
        assert!(
            StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
                .is_err()
        );
    }
    assert_eq!(
        StableIdentity::browser_origin(
            BrowserAdapter::Chromium,
            WebScheme::Http,
            "[2001:0db8::1]",
            None,
        )
        .expect("IPv6"),
        StableIdentity::BrowserOrigin {
            adapter: BrowserAdapter::Chromium,
            scheme: WebScheme::Http,
            host: "2001:db8::1".to_owned(),
            port: 80,
        }
    );
}

#[test]
fn permission_dependencies_and_retention_bounds_are_strict() {
    let mut permissions = SubjectPermissions::deny_all();
    permissions.suggest = PermissionDecision::Allow;
    assert!(permissions.validate().is_err());

    let mut permissions = allowed(None);
    permissions.suggest = PermissionDecision::Block;
    assert!(permissions.validate().is_err());

    let mut permissions = allowed(Some(1));
    assert!(permissions.validate().is_ok());
    permissions.retention = RetentionPermission::Bounded { days: 91 };
    assert!(permissions.validate().is_err());
}

#[test]
fn linux_adapters_sort_by_wire_name_like_the_desktop_clients() {
    // The desktop CLI and panel sort subjects by adapter string. A newer
    // adapter must not make their canonical documents unacceptable.
    let adapters = [
        LinuxAdapter::Fcitx,
        LinuxAdapter::Neovim,
        LinuxAdapter::Obsidian,
        LinuxAdapter::Shell,
        LinuxAdapter::Libreoffice,
    ];
    for adapter in adapters {
        assert_eq!(
            serde_json::to_value(adapter).unwrap(),
            serde_json::Value::String(adapter.as_str().to_owned())
        );
    }
    let mut sorted = adapters;
    sorted.sort();
    let names = sorted.map(LinuxAdapter::as_str);
    assert_eq!(
        names,
        ["fcitx", "libreoffice", "neovim", "obsidian", "shell"]
    );

    let rule = |adapter, app_id: &str| SubjectRule {
        identity: StableIdentity::linux_app(adapter, app_id).unwrap(),
        permissions: SubjectPermissions {
            learn: PermissionDecision::Block,
            retention: RetentionPermission::None,
            ..allowed(None)
        },
    };
    let document = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: vec![
            rule(LinuxAdapter::Fcitx, "libreoffice"),
            rule(LinuxAdapter::Libreoffice, "libreoffice-writer"),
            rule(LinuxAdapter::Obsidian, "obsidian"),
        ],
    };
    document
        .validate()
        .expect("alphabetical adapter order is canonical");
}

#[test]
fn settings_require_sorted_unique_subjects() {
    let first = SubjectRule {
        identity: identity("a.example"),
        permissions: allowed(None),
    };
    let second = SubjectRule {
        identity: identity("b.example"),
        permissions: allowed(None),
    };
    let valid = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: std::collections::BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects: vec![first.clone(), second.clone()],
    };
    valid.validate().expect("canonical settings");
    let mut reversed = valid.clone();
    reversed.subjects.reverse();
    assert!(reversed.validate().is_err());
    let mut duplicate = valid;
    duplicate.subjects = vec![first.clone(), first];
    assert!(duplicate.validate().is_err());
}

#[test]
fn maximum_shape_settings_fit_storage_and_wire_limits() {
    let subjects = (0..MAX_SUBJECTS)
        .map(|index| {
            let host = format!(
                "h{index:03}.{}.{}.{}.{}",
                "a".repeat(63),
                "b".repeat(63),
                "c".repeat(63),
                "d".repeat(56)
            );
            assert_eq!(host.len(), 253);
            SubjectRule {
                identity: identity(&host),
                permissions: allowed(Some(90)),
            }
        })
        .collect();
    let settings = SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: crate::protocol::MAX_SAFE_COUNTER,
        paused: true,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Normal,
        screen_share: crate::settings::ScreenShareMode::Hold,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        languages_chosen: false,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: false,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: (0..MAX_SUBJECTS)
            .map(|index| {
                (
                    format!("app{index:03}{}", "a".repeat(122)),
                    super::PredictionLanguage::German,
                )
            })
            .collect(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects,
    };
    settings.validate().expect("maximum-shape settings");
    let stored = super::store::encode_settings(&settings).expect("bounded settings encoding");
    assert!(stored.len() <= MAX_SETTINGS_BYTES);

    let wire_value = serde_json::json!({
        "v": 1,
        "id": "x".repeat(128),
        "type": "settings.replace",
        "mono_ms": crate::protocol::MAX_SAFE_COUNTER,
        "payload": {
            "expected_revision": crate::protocol::MAX_SAFE_COUNTER,
            "document": settings
        }
    });
    let envelope: WireEnvelope =
        serde_json::from_value(wire_value).expect("maximum settings envelope");
    envelope.validate_shape().expect("valid envelope shape");
    assert!(serde_json::to_vec(&envelope).expect("wire encoding").len() <= MAX_FRAME_BYTES);
}

#[test]
fn strict_json_rejects_unknown_fields_and_unsafe_combinations() {
    let unknown = br#"{
      "schema":"badi.settings.v1","revision":0,"paused":true,"subjects":[],
      "cloud":"allow"
    }"#;
    assert!(serde_json::from_slice::<SettingsV2>(unknown).is_err());

    let invalid = br#"{
      "schema":"badi.settings.v1","revision":1,"paused":false,
      "subjects":[{"identity":{"kind":"browser_origin","adapter":"chromium",
      "scheme":"https","host":"example.com","port":443},"permissions":{
      "suggest":"allow","display":"block","context_read":"allow","learn":"block",
      "retention":{"mode":"none"}}}]
    }"#;
    let decoded: SettingsV2 = serde_json::from_slice(invalid).expect("structurally valid");
    assert!(decoded.validate().is_err());
}

#[test]
fn legacy_v1_document_migrates_atomically_to_canonical_v2_on_load() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("paths");
    let storage = PrivateStorage::open(paths.clone()).expect("private storage");
    let legacy = format!(
        r#"{{
          "schema":"{SETTINGS_SCHEMA_V1}","revision":1,"paused":false,
          "subjects":[{{"identity":{{"kind":"browser_origin","adapter":"chromium",
          "scheme":"https","host":"example.com","port":443}},"permissions":{{
          "suggest":"allow","display":"allow","context_read":"allow","learn":"block",
          "retention":{{"mode":"none"}}}}}}]
        }}"#
    );
    fs::write(paths.settings_path(), legacy).expect("seed legacy settings");
    fs::set_permissions(
        paths.settings_path(),
        fs::Permissions::from_mode(PRIVATE_FILE_MODE),
    )
    .expect("private legacy settings");

    let settings = storage
        .settings_store()
        .expect("settings store")
        .load_or_initialize()
        .expect("migrate settings");
    assert_eq!(settings.schema, SETTINGS_SCHEMA);
    assert_eq!(settings.revision, 1);
    assert!(
        settings
            .resolve_identity(&identity("example.com"))
            .allows_suggestion()
    );

    let persisted: serde_json::Value =
        serde_json::from_slice(&fs::read(paths.settings_path()).expect("read migrated settings"))
            .expect("migrated JSON");
    assert_eq!(persisted["schema"], SETTINGS_SCHEMA);
    assert!(
        fs::read_dir(paths.config_dir())
            .expect("config directory")
            .all(|entry| !entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp"))
    );
}

#[test]
fn private_storage_locks_for_lifetime_and_uses_path_override() {
    let temporary = tempdir().expect("temporary directory");
    let paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("paths");
    let storage = PrivateStorage::open(paths.clone()).expect("first lock");
    assert!(matches!(
        PrivateStorage::open(paths),
        Err(PrivateStorageError::LockUnavailable)
    ));
    drop(storage);
}

#[test]
fn startup_removes_only_safe_owned_complete_document_temporaries() {
    let temporary = tempdir().expect("temporary directory");
    let storage_paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("paths");
    for directory in [storage_paths.config_dir(), storage_paths.data_dir()] {
        fs::create_dir_all(directory).expect("storage directory");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .expect("private directory");
    }
    let settings_temporary = storage_paths
        .config_dir()
        .join(".settings.json.00000000000000000000000000000001.tmp");
    let personalization_temporary = storage_paths
        .data_dir()
        .join(".personalization.json.00000000000000000000000000000002.tmp");
    let unrelated = storage_paths
        .data_dir()
        .join(".personalization.json.not-a-badi-nonce.tmp");
    for path in [&settings_temporary, &personalization_temporary, &unrelated] {
        fs::write(path, b"private complete document\n").expect("temporary file");
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .expect("private temporary mode");
    }

    let _storage = PrivateStorage::open(storage_paths).expect("storage recovery");

    assert!(!settings_temporary.exists());
    assert!(!personalization_temporary.exists());
    assert!(unrelated.exists());
}

#[test]
fn startup_preserves_and_rejects_an_unsafe_owned_temporary_file() {
    let temporary = tempdir().expect("temporary directory");
    let storage_paths = StoragePaths::new(
        temporary.path().join("config/badi"),
        temporary.path().join("data/badi"),
    )
    .expect("paths");
    fs::create_dir_all(storage_paths.config_dir()).expect("config directory");
    fs::create_dir_all(storage_paths.data_dir()).expect("data directory");
    fs::set_permissions(
        storage_paths.config_dir(),
        fs::Permissions::from_mode(0o700),
    )
    .expect("private config directory");
    fs::set_permissions(storage_paths.data_dir(), fs::Permissions::from_mode(0o700))
        .expect("private data directory");
    let unsafe_temporary = storage_paths
        .data_dir()
        .join(".personalization.json.00000000000000000000000000000003.tmp");
    fs::write(&unsafe_temporary, b"untrusted file\n").expect("unsafe temporary file");
    fs::set_permissions(&unsafe_temporary, fs::Permissions::from_mode(0o644))
        .expect("unsafe temporary mode");

    assert!(matches!(
        PrivateStorage::open(storage_paths),
        Err(PrivateStorageError::UnsafeFile)
    ));
    assert!(unsafe_temporary.exists());
}

#[test]
fn either_shared_storage_root_uses_the_same_interprocess_lock() {
    let temporary = tempdir().expect("temporary directory");
    let shared_config = temporary.path().join("config/shared-badi");
    let shared_data = temporary.path().join("data/shared-badi");
    let first =
        PrivateStorage::open(StoragePaths::new(&shared_config, &shared_data).expect("first paths"))
            .expect("first storage");

    assert!(matches!(
        PrivateStorage::open(
            StoragePaths::new(&shared_config, temporary.path().join("data/other-badi"),)
                .expect("shared config paths"),
        ),
        Err(PrivateStorageError::LockUnavailable)
    ));
    assert!(matches!(
        PrivateStorage::open(
            StoragePaths::new(temporary.path().join("config/other-badi"), &shared_data,)
                .expect("shared data paths"),
        ),
        Err(PrivateStorageError::LockUnavailable)
    ));
    drop(first);
}

#[test]
fn settings_store_initializes_privately_and_enforces_cas() {
    let temporary = tempdir().expect("temporary directory");
    let config = temporary.path().join("config/badi");
    let data = temporary.path().join("data/badi");
    let storage =
        PrivateStorage::open(StoragePaths::new(config, data).expect("paths")).expect("storage");
    let store = storage.settings_store().expect("settings store");
    let initial = store.load_or_initialize().expect("initial settings");
    assert_eq!(initial, SettingsV2::deny_by_default());
    let metadata = fs::metadata(store.path()).expect("settings metadata");
    assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

    let next = settings_with(SubjectRule {
        identity: identity("example.com"),
        permissions: allowed(Some(30)),
    });
    store
        .compare_and_replace(0, next.clone())
        .expect("first replacement");
    assert!(matches!(
        store.compare_and_replace(0, next.clone()),
        Err(SettingsStoreError::RevisionConflict {
            expected: 0,
            actual: 1
        })
    ));
    assert_eq!(store.load().expect("load"), Some(next));
}

#[test]
fn post_rename_write_error_reloads_new_authority_instead_of_restoring_old() {
    let temporary = tempdir().expect("temporary directory");
    let storage = PrivateStorage::open(
        StoragePaths::new(
            temporary.path().join("config/badi"),
            temporary.path().join("data/badi"),
        )
        .expect("paths"),
    )
    .expect("storage");
    let store = storage.settings_store().expect("settings store");
    store.load_or_initialize().expect("initial settings");
    let next = settings_with(SubjectRule {
        identity: identity("example.com"),
        permissions: allowed(Some(30)),
    });

    let replaced = store
        .compare_and_replace_with_writer(0, &next, |path, document| {
            write_settings(path, document)?;
            Err(SettingsStoreError::Storage(PrivateStorageError::Io(
                std::io::Error::other("injected post-rename failure"),
            )))
        })
        .expect("exact new document is authoritative");

    assert_eq!(replaced, next);
    assert_eq!(store.load().expect("load"), Some(replaced));
}

#[test]
fn store_rejects_symlinks_hardlinks_and_public_files() {
    let temporary = tempdir().expect("temporary directory");
    let directory = temporary.path().join("config/badi");
    let storage = PrivateStorage::open(
        StoragePaths::new(&directory, temporary.path().join("data/badi")).expect("paths"),
    )
    .expect("storage");
    let real = directory.join("real.json");
    fs::write(&real, b"{}").expect("real file");
    fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).expect("file mode");

    let symlink = directory.join("settings.json");
    std::os::unix::fs::symlink(&real, &symlink).expect("symlink");
    let store = storage.settings_store().expect("store");
    assert!(store.load().is_err());
    fs::remove_file(&symlink).expect("remove symlink");

    fs::hard_link(&real, &symlink).expect("hard link");
    assert!(store.load().is_err());
    fs::remove_file(&symlink).expect("remove hard link");
    fs::remove_file(&real).expect("remove real");

    fs::write(&symlink, b"{}").expect("public file");
    fs::set_permissions(&symlink, fs::Permissions::from_mode(0o644)).expect("public mode");
    assert!(store.load().is_err());
}

#[test]
fn clear_retry_requires_a_successful_directory_sync_after_visible_absence() {
    fn fail_sync(_: &std::path::Path) -> Result<(), PrivateStorageError> {
        Err(PrivateStorageError::CommitStateUnknown)
    }

    let temporary = tempdir().expect("temporary directory");
    let directory = temporary.path().join("data/badi");
    fs::create_dir_all(&directory).expect("data directory");
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).expect("private directory");
    let path = directory.join("personalization.json");
    fs::write(&path, b"private state\n").expect("private file");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("private mode");
    assert!(matches!(
        remove_private_file_with_sync(&path, fail_sync),
        Err(PrivateStorageError::CommitStateUnknown)
    ));
    assert!(!path.exists());
    assert!(matches!(
        remove_private_file_with_sync(&path, fail_sync),
        Err(PrivateStorageError::CommitStateUnknown)
    ));
    assert!(!path.exists());
    assert!(
        !remove_private_file_with_sync(&path, super::private_file::sync_directory)
            .expect("durably acknowledge absence")
    );
}

#[test]
fn context_words_need_v2_keep_per_subject_off_canonical_and_stay_off_until_chosen() {
    let site = identity("mail.example");
    let other = identity("docs.example");
    let mut settings = settings_with(SubjectRule {
        identity: site.clone(),
        permissions: allowed(None),
    });
    assert!(!settings.context_words_for(&site), "unchosen means off");
    let unchosen = serde_json::to_value(&settings).expect("json");
    assert!(unchosen.get("context_words").is_none() && unchosen.get("context_words_off").is_none());
    settings.context_words = Some(false);
    assert!(!settings.context_words_for(&site));
    settings.context_words = Some(true);
    assert!(settings.context_words_for(&site) && settings.context_words_for(&other));
    settings.context_words_off = vec![site.clone()];
    settings.validate().expect("valid");
    assert!(!settings.context_words_for(&site) && settings.context_words_for(&other));
    settings.paused = true;
    assert!(
        !settings.context_words_for(&other),
        "a pause stops it everywhere"
    );
    settings.paused = false;

    let round: SettingsV2 =
        serde_json::from_value(serde_json::to_value(&settings).expect("json")).expect("decode");
    assert_eq!(round, settings);
    let mut unsorted = settings.clone();
    unsorted.context_words_off = vec![site.clone(), other.clone()];
    unsorted.context_words_off.sort();
    unsorted.context_words_off.reverse();
    assert_eq!(
        unsorted.validate(),
        Err(crate::settings::SettingsValidationError::ContextWordsOffNotCanonical)
    );
    let v1 = serde_json::json!({
        "schema": crate::settings::SETTINGS_SCHEMA_V1, "revision": 1, "paused": false,
        "context_words": true, "subjects": [],
    });
    assert!(serde_json::from_value::<SettingsV2>(v1).is_err());
}
