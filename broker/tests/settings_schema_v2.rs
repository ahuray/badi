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
