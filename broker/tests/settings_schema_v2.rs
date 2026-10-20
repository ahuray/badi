use std::fs;
use std::path::{Path, PathBuf};

use badi_broker::settings::{SETTINGS_SCHEMA, SettingsV2, StableIdentity};
use serde_json::{Value, json};

fn schema_path(version: u8) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("schemas")
        .join(format!("badi.settings.v{version}.schema.json"))
}

fn validator(version: u8) -> jsonschema::Validator {
    let schema: Value = serde_json::from_str(
        &fs::read_to_string(schema_path(version)).expect("settings schema file"),
    )
    .expect("settings schema JSON");
    jsonschema::validator_for(&schema).expect("settings schema compiles")
}

fn permissions() -> Value {
    json!({
        "suggest": "allow",
        "display": "allow",
        "context_read": "allow",
        "learn": "block",
        "retention": { "mode": "none" }
    })
}

#[test]
fn v2_schema_accepts_mixed_exact_identities_while_v1_remains_immutable() {
    let document = json!({
        "schema": "badi.settings.v2",
        "revision": 7,
        "paused": false,
        "subjects": [
            {
                "identity": {
                    "kind": "browser_origin",
                    "adapter": "chromium",
                    "scheme": "https",
                    "host": "example.com",
                    "port": 443
                },
                "permissions": permissions()
            },
            {
                "identity": {
                    "kind": "linux_app",
                    "adapter": "fcitx",
                    "app_id": "omawrite"
                },
                "permissions": permissions()
            }
        ]
    });
    assert!(validator(2).is_valid(&document));
    assert!(!validator(1).is_valid(&document));

    let decoded: SettingsV2 = serde_json::from_value(document).expect("typed v2 settings");
    decoded.validate().expect("valid v2 settings");
    assert_eq!(decoded.schema, SETTINGS_SCHEMA);
    assert!(matches!(
        decoded.subjects[1].identity,
        StableIdentity::LinuxApp { .. }
    ));
}

#[test]
fn legacy_v1_deserialization_is_lossless_and_canonicalizes_only_the_schema() {
    let legacy = json!({
        "schema": "badi.settings.v1",
        "revision": 3,
        "paused": false,
        "subjects": [{
            "identity": {
                "kind": "browser_origin",
                "adapter": "chromium",
                "scheme": "https",
                "host": "example.com",
                "port": 443
            },
            "permissions": permissions()
        }]
    });
    assert!(validator(1).is_valid(&legacy));

    let migrated: SettingsV2 = serde_json::from_value(legacy).expect("migrate v1 settings");
    assert_eq!(migrated.schema, SETTINGS_SCHEMA);
    assert_eq!(migrated.revision, 3);
    assert_eq!(migrated.subjects.len(), 1);
    assert!(matches!(
        migrated.subjects[0].identity,
        StableIdentity::BrowserOrigin { .. }
    ));
}

#[test]
fn v1_schema_tag_cannot_smuggle_a_linux_application_rule() {
    let invalid = json!({
        "schema": "badi.settings.v1",
        "revision": 1,
        "paused": false,
        "subjects": [{
            "identity": {
                "kind": "linux_app",
                "adapter": "fcitx",
                "app_id": "omawrite"
            },
            "permissions": permissions()
        }]
    });
    assert!(serde_json::from_value::<SettingsV2>(invalid).is_err());
}

#[test]
fn linux_application_learning_is_rejected_until_personalization_is_versioned() {
    let document = json!({
        "schema": "badi.settings.v2",
        "revision": 1,
        "paused": false,
        "subjects": [{
            "identity": {
                "kind": "linux_app",
                "adapter": "fcitx",
                "app_id": "omawrite"
            },
            "permissions": {
                "suggest": "allow",
                "display": "allow",
                "context_read": "allow",
                "learn": "allow",
                "retention": { "mode": "none" }
            }
        }]
    });
    assert!(!validator(2).is_valid(&document));

    let decoded: SettingsV2 = serde_json::from_value(document).expect("typed v2 settings");
    assert!(decoded.validate().is_err());
}

#[test]
fn linux_application_segments_match_the_formal_schema() {
    for app_id in ["omawrite.1editor", "omawrite._editor", "omawrite.-editor"] {
        let document = json!({
            "schema": "badi.settings.v2",
            "revision": 1,
            "paused": false,
            "subjects": [{
                "identity": {
                    "kind": "linux_app",
                    "adapter": "fcitx",
                    "app_id": app_id
                },
                "permissions": permissions()
            }]
        });
        assert!(
            !validator(2).is_valid(&document),
            "schema accepted {app_id}"
        );
        let decoded: SettingsV2 = serde_json::from_value(document).expect("typed invalid settings");
        assert!(decoded.validate().is_err(), "Rust accepted {app_id}");
    }
}

#[test]
fn all_linux_apps_is_an_optional_v2_only_boolean() {
    let mut document = json!({
        "schema": "badi.settings.v2",
        "revision": 4,
        "paused": false,
        "all_linux_apps": true,
        "subjects": []
    });
    assert!(validator(2).is_valid(&document));
    let decoded: SettingsV2 = serde_json::from_value(document.clone()).expect("typed settings");
    decoded.validate().expect("valid settings");
    assert!(decoded.all_linux_apps && !decoded.all_web_origins);
    assert_eq!(serde_json::to_value(&decoded).expect("encoded"), document);
    document["all_linux_apps"] = json!("yes");
    assert!(!validator(2).is_valid(&document));
    assert!(serde_json::from_value::<SettingsV2>(document.clone()).is_err());
    document["all_linux_apps"] = json!(false);
    let encoded =
        serde_json::to_value(serde_json::from_value::<SettingsV2>(document).expect("off"))
            .expect("encoded");
    assert!(
        encoded.get("all_linux_apps").is_none(),
        "off is the canonical absence"
    );
    let legacy = json!({"schema": "badi.settings.v1", "revision": 4, "paused": false,
                        "all_linux_apps": true, "subjects": []});
    assert!(!validator(1).is_valid(&legacy));
    assert!(serde_json::from_value::<SettingsV2>(legacy).is_err());
}

#[test]
fn suggestion_length_is_an_optional_v2_only_choice() {
    let mut document = json!({
        "schema": "badi.settings.v2",
        "revision": 4,
        "paused": false,
        "suggestion_length": "short",
        "subjects": []
    });
    assert!(validator(2).is_valid(&document));
    let decoded: SettingsV2 = serde_json::from_value(document.clone()).expect("typed");
    decoded.validate().expect("valid settings");
    assert_eq!(
        decoded.suggestion_length,
        badi_broker::settings::SuggestionLength::Short
    );
    assert_eq!(serde_json::to_value(&decoded).expect("encoded"), document);
    // Long is measured in the Lab only: the schema omits it and validation
    // refuses it.
    document["suggestion_length"] = json!("long");
    assert!(!validator(2).is_valid(&document));
    let long: SettingsV2 = serde_json::from_value(document.clone()).expect("typed");
    assert!(long.validate().is_err());
    document["suggestion_length"] = json!("normal");
    assert!(validator(2).is_valid(&document));
    let encoded = serde_json::to_value(
        serde_json::from_value::<SettingsV2>(document.clone()).expect("normal"),
    )
    .expect("encoded");
    assert!(
        encoded.get("suggestion_length").is_none(),
        "normal is the canonical absence"
    );
    for invalid in [json!("medium"), json!(8)] {
        document["suggestion_length"] = invalid;
        assert!(!validator(2).is_valid(&document));
        assert!(serde_json::from_value::<SettingsV2>(document.clone()).is_err());
    }
    // Producers never write null; typed decoding reads it as absence, as it
    // does for every optional settings key.
    document["suggestion_length"] = json!(null);
    assert!(!validator(2).is_valid(&document));
    let legacy = json!({"schema": "badi.settings.v1", "revision": 4, "paused": false,
                        "suggestion_length": "short", "subjects": []});
    assert!(!validator(1).is_valid(&legacy));
    assert!(serde_json::from_value::<SettingsV2>(legacy).is_err());
}

#[test]
fn all_web_origins_is_an_optional_v2_only_boolean() {
    let mut document = json!({
        "schema": "badi.settings.v2",
        "revision": 4,
        "paused": false,
        "all_web_origins": true,
        "subjects": []
    });
    assert!(validator(2).is_valid(&document));
    let decoded: SettingsV2 = serde_json::from_value(document.clone()).expect("typed settings");
    decoded.validate().expect("valid settings");
    assert!(decoded.all_web_origins);
    assert_eq!(serde_json::to_value(&decoded).expect("encoded"), document);

    document["all_web_origins"] = json!("yes");
    assert!(!validator(2).is_valid(&document));
    assert!(serde_json::from_value::<SettingsV2>(document.clone()).is_err());

    // Off is the canonical absence of the property.
    document["all_web_origins"] = json!(false);
    let off: SettingsV2 = serde_json::from_value(document.clone()).expect("explicit off");
    assert!(!off.all_web_origins);
    let encoded = serde_json::to_value(&off).expect("encoded");
    assert!(encoded.get("all_web_origins").is_none());
    assert!(validator(2).is_valid(&encoded));

    for value in [true, false] {
        let legacy = json!({
            "schema": "badi.settings.v1",
            "revision": 4,
            "paused": false,
            "all_web_origins": value,
            "subjects": []
        });
        assert!(!validator(1).is_valid(&legacy));
        assert!(serde_json::from_value::<SettingsV2>(legacy).is_err());
    }
}

#[test]
fn language_overrides_match_the_schema_and_do_not_grant_access() {
    let schema = validator(2);
    let oversized: serde_json::Map<String, Value> =
        (0..65).map(|i| (format!("app{i}"), json!("en"))).collect();
    for (overrides, valid) in [
        (
            json!({"brave-origin": "de", "zen": "fa", "chromium": "en"}),
            true,
        ),
        (json!({}), true),
        (Value::Null, false),
        (json!({"zen": "fr", "chromium": "es"}), true),
        (json!({"zen": "it"}), false),
        (json!({"zen": "de-DE"}), false),
        (json!({"app.1invalid": "en"}), false),
        (Value::Object(oversized), false),
    ] {
        let mut document = json!({"schema": SETTINGS_SCHEMA, "revision": 1,
            "paused": false, "subjects": [], "language_overrides": overrides});
        assert_eq!(schema.is_valid(&document), valid);
        let decoded = serde_json::from_value::<SettingsV2>(document.clone());
        assert_eq!(decoded.as_ref().is_ok_and(|s| s.validate().is_ok()), valid);
        if valid {
            let settings = decoded.expect("valid overrides");
            let app = StableIdentity::linux_app(
                badi_broker::settings::LinuxAdapter::Fcitx,
                "brave-origin",
            )
            .expect("app");
            assert!(!settings.resolve_identity(&app).allows_suggestion());
        }
        document["schema"] = json!("badi.settings.v1");
        assert!(serde_json::from_value::<SettingsV2>(document).is_err());
    }
}

#[test]
fn writing_languages_and_healing_choices_match_the_schema() {
    let schema = validator(2);
    for (field, value, valid) in [
        (
            "writing_languages",
            json!(["en", "de", "fa", "fr", "es", "pt"]),
            true,
        ),
        ("writing_languages", json!(["en", "fr"]), true),
        ("writing_languages", json!(["en", "pt"]), true),
        ("writing_languages", json!([]), true),
        // Only the broker enforces the canonical order, as for spelling.
        ("writing_languages", json!(["fr", "en"]), false),
        ("writing_languages", json!(["en", "en"]), false),
        ("writing_languages", json!(["it"]), false),
        (
            "trailing_space_healing",
            json!({"de": false, "fr": true, "pt": true}),
            true,
        ),
        ("trailing_space_healing", json!({}), true),
        ("trailing_space_healing", json!({"it": true}), false),
        ("trailing_space_healing", json!({"de": "on"}), false),
    ] {
        let mut document = json!({"schema": SETTINGS_SCHEMA, "revision": 1,
            "paused": false, "subjects": []});
        document[field] = value.clone();
        let unordered = value == json!(["fr", "en"]);
        assert_eq!(
            schema.is_valid(&document),
            valid || unordered,
            "{field} {value}"
        );
        let decoded = serde_json::from_value::<SettingsV2>(document.clone());
        assert_eq!(
            decoded.as_ref().is_ok_and(|s| s.validate().is_ok()),
            valid,
            "{field} {value}"
        );
        if valid {
            // Producers omit the defaults and keep every other value.
            let settings = decoded.expect("valid");
            let written = serde_json::to_value(&settings).expect("serializes");
            let omitted = (field == "writing_languages" && value == json!(["en", "de", "fa"]))
                || value == json!({});
            assert_eq!(written.get(field).is_none(), omitted, "{field} {value}");
            if !omitted {
                assert_eq!(written[field], value);
            }
        }
        document["schema"] = json!("badi.settings.v1");
        assert!(serde_json::from_value::<SettingsV2>(document).is_err());
    }
    let default: SettingsV2 = serde_json::from_value(json!({"schema": SETTINGS_SCHEMA,
        "revision": 1, "paused": false, "subjects": []}))
    .expect("default");
    let preferences = default.language_preferences();
    assert_eq!(
        preferences,
        badi_broker::writing::LanguagePreferences::default()
    );
}
