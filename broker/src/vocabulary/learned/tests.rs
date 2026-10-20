use tempfile::{TempDir, tempdir};

use super::{
    LEARNED_RETENTION_DAYS, LearnedStore, MAX_LEARNED_SUBJECTS, MAX_WORDS_PER_SUBJECT,
    OFFER_AFTER_FIELDS,
};
use crate::settings::{
    BrowserAdapter, LinuxAdapter, SettingsV2, StableIdentity, WebScheme, read_private_limited,
};
use crate::style::{StyleError, StyleKey, StyleKeyState};

const TODAY: u64 = 20_000;

fn app(id: &str) -> StableIdentity {
    StableIdentity::LinuxApp {
        adapter: LinuxAdapter::Fcitx,
        app_id: id.to_owned(),
    }
}

fn site(host: &str) -> StableIdentity {
    StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
        .expect("identity")
}

fn context(on: Option<bool>, mut off: Vec<StableIdentity>) -> SettingsV2 {
    off.sort();
    SettingsV2 {
        paused: false,
        context_words: on,
        context_words_off: off,
        ..SettingsV2::deny_by_default()
    }
}

fn key(byte: &str) -> StyleKey {
    StyleKey::from_hex(&byte.repeat(16), &byte.repeat(32)).expect("key")
}

fn store() -> (TempDir, LearnedStore) {
    let directory = tempdir().expect("directory");
    let store =
        LearnedStore::open(directory.path().join("badi/learned-words.json")).expect("store");
    (directory, store)
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

#[test]
fn a_word_is_offered_only_after_two_fields_and_only_where_context_is_on() {
    let (_directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    let claude = app("com.anthropic.claude");
    assert!(matches!(
        store.record(&on, TODAY, &claude, &words(&["Okonkwo"])),
        Err(StyleError::KeyUnavailable)
    ));
    assert_eq!(store.set_key(key("ab")), StyleKeyState::Ready);
    assert!(
        store
            .record(&on, TODAY, &claude, &words(&["Okonkwo", "tokenizer"]))
            .unwrap()
    );
    assert!(
        store.offered(&on, TODAY, &claude).is_empty(),
        "one field is not enough"
    );
    assert_eq!(OFFER_AFTER_FIELDS, 2);
    store
        .record(&on, TODAY + 1, &claude, &words(&["Okonkwo"]))
        .unwrap();
    assert_eq!(store.offered(&on, TODAY + 1, &claude), vec!["Okonkwo"]);
    assert!(
        store.offered(&on, TODAY + 1, &app("telegram")).is_empty(),
        "per subject"
    );
    for settings in [
        context(None, Vec::new()),
        context(Some(false), Vec::new()),
        context(Some(true), vec![claude.clone()]),
    ] {
        assert_eq!(
            store.offered(&settings, TODAY + 1, &claude),
            Vec::<String>::new()
        );
        assert!(
            !store
                .record(&settings, TODAY + 1, &claude, &words(&["Okonkwo"]))
                .unwrap()
        );
    }
    let status = store.status();
    assert_eq!(
        (status.key, status.subjects, status.words),
        (StyleKeyState::Ready, 1, Some(2))
    );
}

#[test]
fn the_file_holds_no_word_in_the_clear_and_another_key_cannot_read_it() {
    let (directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    let mail = site("mail.example");
    store.set_key(key("ab"));
    store
        .record(&on, TODAY, &mail, &words(&["Marinella", "Okonkwo"]))
        .unwrap();
    store
        .record(&on, TODAY, &mail, &words(&["Marinella"]))
        .unwrap();
    let path = directory.path().join("badi/learned-words.json");
    let bytes = read_private_limited(&path, super::MAX_LEARNED_BYTES)
        .unwrap()
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("Marinella") && !text.contains("Okonkwo"));
    assert!(
        text.contains("mail.example"),
        "the subject stays in the clear for purging"
    );

    let mut reopened = LearnedStore::open(path.clone()).unwrap();
    assert_eq!(reopened.status().words, None, "no key, no words");
    assert_eq!(reopened.set_key(key("cd")), StyleKeyState::Mismatch);
    assert_eq!(reopened.offered(&on, TODAY, &mail), Vec::<String>::new());
    // A new key's words replace what it cannot read.
    assert!(
        reopened
            .record(&on, TODAY, &mail, &words(&["Priyanka"]))
            .unwrap()
    );
    assert_eq!(reopened.key_state(), StyleKeyState::Ready);
    assert_eq!(reopened.status().words, Some(1));
}

#[test]
fn turning_context_off_for_a_subject_and_retention_purge_without_the_key() {
    let (directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    store.set_key(key("ab"));
    store
        .record(&on, TODAY, &site("mail.example"), &words(&["Marinella"]))
        .unwrap();
    store
        .record(&on, TODAY, &app("telegram"), &words(&["Okonkwo"]))
        .unwrap();
    let mut keyless = LearnedStore::open(directory.path().join("badi/learned-words.json")).unwrap();
    let off = context(Some(true), vec![app("telegram")]);
    assert!(keyless.reconcile(&off, TODAY).unwrap());
    assert_eq!(keyless.status().subjects, 1);
    assert!(
        !keyless
            .reconcile(&off, TODAY + LEARNED_RETENTION_DAYS - 1)
            .unwrap()
    );
    assert!(
        keyless
            .reconcile(&off, TODAY + LEARNED_RETENTION_DAYS)
            .unwrap()
    );
    assert_eq!(keyless.status().subjects, 0);
    assert!(!directory.path().join("badi/learned-words.json").exists());
}

#[test]
fn a_word_unused_past_retention_is_no_longer_offered_or_kept() {
    let (_directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    let claude = app("com.anthropic.claude");
    store.set_key(key("ab"));
    store
        .record(&on, TODAY, &claude, &words(&["Okonkwo"]))
        .unwrap();
    store
        .record(&on, TODAY, &claude, &words(&["Okonkwo"]))
        .unwrap();
    let later = TODAY + LEARNED_RETENTION_DAYS;
    assert!(
        store
            .offered(&on, later - 1, &claude)
            .contains(&"Okonkwo".to_owned())
    );
    assert_eq!(store.offered(&on, later, &claude), Vec::<String>::new());
    store
        .record(&on, later, &claude, &words(&["tokenizer"]))
        .unwrap();
    assert_eq!(store.status().words, Some(1), "the stale word was dropped");
}

#[test]
fn only_finishable_words_are_kept_and_every_bound_holds() {
    let (_directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    let claude = app("com.anthropic.claude");
    store.set_key(key("ab"));
    store
        .record(
            &on,
            TODAY,
            &claude,
            &words(&["short", "with1digit", "x-ray-ish", "tokenizer"]),
        )
        .unwrap();
    assert_eq!(store.status().words, Some(1));
    // Distinct letter-only words beyond the per-subject bound.
    let many: Vec<String> = (0..MAX_WORDS_PER_SUBJECT + 40)
        .map(|index| {
            let mut word = String::from("wordss");
            let mut rest = index;
            loop {
                word.push(char::from(b'a' + u8::try_from(rest % 26).unwrap()));
                rest /= 26;
                if rest == 0 {
                    return word;
                }
            }
        })
        .collect();
    store.record(&on, TODAY, &claude, &many).unwrap();
    assert_eq!(store.status().words, Some(MAX_WORDS_PER_SUBJECT as u64));
    for index in 0..=MAX_LEARNED_SUBJECTS {
        store
            .record(
                &on,
                TODAY,
                &app(&format!("app{}", "x".repeat(index + 1))),
                &words(&["Okonkwo"]),
            )
            .unwrap();
    }
    assert_eq!(store.status().subjects, MAX_LEARNED_SUBJECTS as u64);
}

#[test]
fn forget_removes_every_word_but_keeps_the_shared_key() {
    let (directory, mut store) = store();
    let on = context(Some(true), Vec::new());
    store.set_key(key("ab"));
    store
        .record(&on, TODAY, &app("telegram"), &words(&["Okonkwo"]))
        .unwrap();
    assert!(store.forget().unwrap());
    assert!(!directory.path().join("badi/learned-words.json").exists());
    let status = store.status();
    assert_eq!(
        (status.key, status.subjects, status.words),
        (StyleKeyState::Ready, 0, Some(0))
    );
}
