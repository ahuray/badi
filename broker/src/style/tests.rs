use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;

use tempfile::{TempDir, tempdir};

use super::{
    MAX_SAMPLES_PER_SUBJECT, MAX_STYLE_SAMPLES, StyleError, StyleKey, StyleKeyState, StyleSample,
    StyleStore, authored_sample, decode_hex,
};
use crate::protocol::{MAX_FRAME_BYTES, StyleSamplesPayload};
use crate::settings::{
    BatteryMode, BrowserAdapter, PermissionDecision, RetentionPermission, SETTINGS_SCHEMA,
    SettingsV2, StableIdentity, SubjectPermissions, SubjectRule, WebScheme,
};

const TODAY: u64 = 20_000;

fn site(host: &str) -> StableIdentity {
    StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
        .expect("identity")
}

fn rule(host: &str, learn: bool, days: Option<u16>) -> SubjectRule {
    SubjectRule {
        identity: site(host),
        permissions: SubjectPermissions {
            suggest: PermissionDecision::Allow,
            display: PermissionDecision::Allow,
            context_read: PermissionDecision::Allow,
            learn: if learn {
                PermissionDecision::Allow
            } else {
                PermissionDecision::Block
            },
            retention: days.map_or(RetentionPermission::None, |days| {
                RetentionPermission::Bounded { days }
            }),
        },
    }
}

fn settings(enabled: bool, mut subjects: Vec<SubjectRule>) -> SettingsV2 {
    subjects.sort_by(|left, right| left.identity.cmp(&right.identity));
    SettingsV2 {
        schema: SETTINGS_SCHEMA.to_owned(),
        revision: 1,
        paused: false,
        all_web_origins: false,
        all_linux_apps: false,
        mid_line: false,
        escape_quiet_seconds: 0,
        battery_mode: BatteryMode::Quiet,
        suggestion_length: crate::settings::SuggestionLength::Normal,
        spelling_languages: vec![crate::spelling::SpellingLanguage::English],
        style_learning: enabled,
        context_words: None,
        context_words_off: Vec::new(),
        language_overrides: BTreeMap::new(),
        writing_model: None,
        writing_languages: crate::settings::DEFAULT_WRITING_LANGUAGES.to_vec(),
        trailing_space_healing: std::collections::BTreeMap::new(),
        subjects,
    }
}

fn learning() -> SettingsV2 {
    settings(
        true,
        vec![
            rule("mail.example", true, Some(30)),
            rule("docs.example", true, Some(30)),
        ],
    )
}

fn key(byte: &str) -> StyleKey {
    StyleKey::from_hex(&byte.repeat(16), &byte.repeat(32)).expect("key")
}

fn store() -> (TempDir, StyleStore) {
    let directory = tempdir().expect("directory");
    let store = StyleStore::open(directory.path().join("badi/style.json")).expect("store");
    (directory, store)
}

const SAMPLE: &str = "Thanks for the quick reply, I will look into it today.";

#[test]
fn ciphertext_sealed_by_chacha20poly1305_0_10_opens() {
    // Produced by chacha20poly1305 0.10.1 with the test key `ab`, nonce
    // 0011…6677, and the AAD `open_text` rebuilds for this subject and day.
    // Do not regenerate it with the current crate: the point is that a
    // record sealed before the 0.11 upgrade still opens. Learned words use
    // the same `open_text`.
    let directory = tempdir().expect("directory");
    let path = directory.path().join("badi/style.json");
    std::fs::create_dir_all(path.parent().expect("parent")).expect("directory");
    std::fs::write(
        &path,
        r#"{"schema":"badi.style.v1","key_id":"abababababababababababababababab","records":[{"identity":{"kind":"browser_origin","adapter":"chromium","scheme":"https","host":"mail.example","port":443},"day":20000,"nonce":"00112233445566778899aabbccddeeff0011223344556677","ciphertext":"c947bfe6039b79fccf776613d4cc4af6c94457da8287dc6171046ffc77c1e325baff43ffe72fb91a15ddd06faf57ed8d3d29bf58c076445c3b16b70d8503eb2736d7c21fd50b"}]}"#,
    )
    .expect("fixture");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("private mode");
    let mut store = StyleStore::open(path).expect("store");
    assert_eq!(store.set_key(key("ab")), StyleKeyState::Ready);
    assert_eq!(
        store.export().expect("export"),
        vec![StyleSample {
            identity: site("mail.example"),
            day: TODAY,
            text: SAMPLE.to_owned(),
        }]
    );
    // The same plaintext is the sample already stored, so sealing again
    // changes nothing.
    assert!(
        !store
            .record(&learning(), TODAY, &site("mail.example"), SAMPLE)
            .expect("duplicate")
    );
    assert_eq!(store.status().samples, 1);
}

#[test]
fn samples_are_sealed_and_only_an_export_reads_them() {
    let (directory, mut store) = store();
    store.set_key(key("ab"));
    assert!(
        store
            .record(&learning(), TODAY, &site("mail.example"), SAMPLE)
            .unwrap()
    );
    let disk = std::fs::read_to_string(directory.path().join("badi/style.json")).unwrap();
    assert!(!disk.contains("quick reply"), "no plaintext on disk");
    assert!(
        disk.contains("mail.example"),
        "subject metadata stays readable"
    );
    let reopened = {
        let mut store = StyleStore::open(directory.path().join("badi/style.json")).unwrap();
        store.set_key(key("ab"));
        store.export().unwrap()
    };
    assert_eq!(
        reopened,
        vec![StyleSample {
            identity: site("mail.example"),
            day: TODAY,
            text: SAMPLE.to_owned(),
        }]
    );
    // A duplicate of the same subject changes nothing.
    assert!(
        !store
            .record(&learning(), TODAY, &site("mail.example"), SAMPLE)
            .unwrap()
    );
    assert_eq!(store.status().samples, 1);
}

#[test]
fn nothing_is_kept_without_opt_in_learning_retention_or_a_key() {
    let (directory, mut store) = store();
    let path = directory.path().join("badi/style.json");
    assert!(matches!(
        store.record(&learning(), TODAY, &site("mail.example"), SAMPLE),
        Err(StyleError::KeyUnavailable)
    ));
    store.set_key(key("ab"));
    for (settings, host) in [
        (
            settings(false, vec![rule("mail.example", true, Some(30))]),
            "mail.example",
        ),
        (
            settings(true, vec![rule("mail.example", false, None)]),
            "mail.example",
        ),
        (
            settings(true, vec![rule("mail.example", true, None)]),
            "mail.example",
        ),
        (learning(), "other.example"),
    ] {
        assert!(!store.record(&settings, TODAY, &site(host), SAMPLE).unwrap());
    }
    assert!(!path.exists());
}

#[test]
fn records_are_bound_to_their_key_subject_and_day() {
    let (directory, mut store) = store();
    let path = directory.path().join("badi/style.json");
    store.set_key(key("ab"));
    store
        .record(&learning(), TODAY, &site("mail.example"), SAMPLE)
        .unwrap();
    // Another key cannot read or extend the store.
    let mut other = StyleStore::open(path.clone()).unwrap();
    assert_eq!(other.set_key(key("cd")), StyleKeyState::Mismatch);
    assert!(matches!(other.export(), Err(StyleError::KeyMismatch)));
    assert!(matches!(
        other.record(&learning(), TODAY, &site("docs.example"), SAMPLE),
        Err(StyleError::KeyMismatch)
    ));
    // A record moved to another subject or day no longer opens.
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("mail.example", "docs.example");
    std::fs::write(&path, edited).unwrap();
    let mut moved = StyleStore::open(path).unwrap();
    moved.set_key(key("ab"));
    assert!(matches!(moved.export(), Err(StyleError::KeyMismatch)));
}

#[test]
fn revocation_and_expiry_purge_without_the_key() {
    let (directory, mut store) = store();
    let path = directory.path().join("badi/style.json");
    store.set_key(key("ab"));
    store
        .record(&learning(), TODAY - 10, &site("mail.example"), SAMPLE)
        .unwrap();
    store
        .record(
            &learning(),
            TODAY,
            &site("docs.example"),
            "The draft is ready for review now.",
        )
        .unwrap();
    let mut keyless = StyleStore::open(path.clone()).unwrap();
    assert_eq!(keyless.key_state(), StyleKeyState::Absent);
    let revoked = settings(
        true,
        vec![
            rule("mail.example", false, None),
            rule("docs.example", true, Some(30)),
        ],
    );
    assert!(keyless.reconcile(&revoked, TODAY).unwrap());
    assert_eq!(keyless.status().samples, 1);
    // Ten days old under a five-day retention is expired.
    let short = settings(true, vec![rule("docs.example", true, Some(5))]);
    assert!(!keyless.reconcile(&short, TODAY).unwrap());
    assert!(keyless.reconcile(&short, TODAY + 6).unwrap());
    assert!(!path.exists(), "an empty store leaves no file");
}

#[test]
fn subjects_and_the_store_are_capped_by_dropping_the_oldest() {
    let (_directory, mut store) = store();
    store.set_key(key("ab"));
    let settings = settings(
        true,
        (0..8)
            .map(|index| rule(&format!("site{index}.example"), true, Some(90)))
            .collect(),
    );
    for day in 0..MAX_SAMPLES_PER_SUBJECT + 3 {
        let text = format!("Sample number {} of this long subject.", "x".repeat(day));
        store
            .record(
                &settings,
                TODAY - 60 + day as u64,
                &site("site0.example"),
                &text,
            )
            .unwrap();
    }
    assert_eq!(store.status().samples, MAX_SAMPLES_PER_SUBJECT as u64);
    let oldest = store
        .export()
        .unwrap()
        .iter()
        .map(|sample| sample.day)
        .min();
    assert_eq!(oldest, Some(TODAY - 60 + 3));
    for index in 1..8 {
        for day in 0..MAX_SAMPLES_PER_SUBJECT {
            let text = format!("Another sample {} for this other subject.", "y".repeat(day));
            store
                .record(
                    &settings,
                    TODAY - 30 + day as u64,
                    &site(&format!("site{index}.example")),
                    &text,
                )
                .unwrap();
        }
    }
    assert_eq!(store.status().samples, MAX_STYLE_SAMPLES as u64);
}

#[test]
fn forget_removes_samples_and_the_key_and_corruption_is_reported() {
    let (directory, mut store) = store();
    let path = directory.path().join("badi/style.json");
    store.set_key(key("ab"));
    store
        .record(&learning(), TODAY, &site("mail.example"), SAMPLE)
        .unwrap();
    assert!(store.forget().unwrap());
    assert!(!path.exists());
    assert_eq!(store.key_state(), StyleKeyState::Absent);
    std::fs::write(&path, "{\"schema\":\"badi.style.v1\",\"key_id\":\"zz\"}").unwrap();
    assert!(StyleStore::open(path).is_err());
}

#[test]
fn keys_are_exact_lowercase_hex() {
    assert!(StyleKey::from_hex(&"ab".repeat(16), &"cd".repeat(32)).is_ok());
    for (id, key) in [
        ("AB".repeat(16), "cd".repeat(32)),
        ("ab".repeat(15), "cd".repeat(32)),
        ("ab".repeat(16), "cd".repeat(31)),
        ("zz".repeat(16), "cd".repeat(32)),
    ] {
        assert!(StyleKey::from_hex(&id, &key).is_err(), "{id} {key}");
    }
    assert_eq!(decode_hex("0a1f"), Some(vec![10, 31]));
    assert!(!format!("{:?}", key("ab")).contains(&"ab".repeat(32)));
}

#[test]
fn samples_are_the_last_complete_sentences_without_facts() {
    assert_eq!(
        authored_sample(
            "We met yesterday. Thanks for the quick reply, I will look into it today. Also the"
        ),
        Some(SAMPLE.to_owned())
    );
    // Short sentences join the ones before them.
    assert_eq!(
        authored_sample("I agree with the plan. Sounds good. Thanks! And"),
        Some("I agree with the plan. Sounds good. Thanks!".to_owned())
    );
    assert_eq!(
        authored_sample("Nothing is complete in this draft yet"),
        None
    );
    assert_eq!(authored_sample("Too short. So"), None);
    assert_eq!(
        authored_sample("Call me at 555 0100 tomorrow morning please. Ok"),
        None
    );
    assert_eq!(
        authored_sample("Write to anna@example.com about the invoice. Ok"),
        None
    );
    assert_eq!(
        authored_sample("See https example dot com for the draft, please. Ok"),
        None
    );
    assert_eq!(
        authored_sample("Dear team,\nThe rollout went well and nobody noticed it at all. Next"),
        Some("The rollout went well and nobody noticed it at all.".to_owned())
    );
    assert_eq!(
        authored_sample("آیا گزارش را برای تیم فرستادی؟ بله"),
        Some("آیا گزارش را برای تیم فرستادی؟".to_owned())
    );
    // A decimal point is not a sentence end.
    assert_eq!(authored_sample("Version 1.5 works"), None);
}

#[test]
fn a_full_export_fits_one_frame() {
    let longest = "ع".repeat(super::MAX_SAMPLE_CHARS);
    let samples = (0..MAX_STYLE_SAMPLES)
        .map(|index| StyleSample {
            identity: StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Https,
                &format!(
                    "{label}.{label}.{label}.s{index}.example",
                    label = "a".repeat(60)
                ),
                Some(65_535),
            )
            .expect("identity"),
            day: u64::MAX >> 11,
            text: longest.clone(),
        })
        .collect::<Vec<_>>();
    let payload = serde_json::to_vec(&StyleSamplesPayload { samples }).unwrap();
    assert!(payload.len() + 256 < MAX_FRAME_BYTES, "{}", payload.len());
}
