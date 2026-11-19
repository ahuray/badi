use super::{CarryScope, MIN_SLID_SHARED_BYTES, TypeThrough, typed_prefix};
use crate::metrics::CarryMiss;
use crate::protocol::{
    Activation, AdapterKind, ContextChangedPayload, FieldDescriptor, FieldPurpose, OffsetUnit,
    Selection, TargetKind,
};
use std::time::{Duration, Instant};

fn scope(app_id: &str) -> CarryScope {
    CarryScope {
        adapter_kind: AdapterKind::Fcitx,
        kind: TargetKind::DesktopApplication,
        app_id: app_id.to_owned(),
        origin: None,
    }
}

fn carry(before: &str, text: &str, expires_in: Duration) -> TypeThrough {
    TypeThrough {
        scope: scope("org.telegram.desktop"),
        before: before.to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
        text: text.to_owned(),
        expires_at: Instant::now() + expires_in,
        aggregate_day: None,
        turned_down: false,
    }
}

fn context(before: &str) -> ContextChangedPayload {
    ContextChangedPayload {
        fingerprint: "fingerprint_0000000000000001".to_owned(),
        before: before.to_owned(),
        after: String::new(),
        selection: Selection {
            anchor: 0,
            head: 0,
            unit: OffsetUnit::UnicodeScalarValues,
        },
        field: FieldDescriptor {
            purpose: FieldPurpose::Normal,
            editable: true,
            multiline: true,
            composing: false,
            sensitive: false,
            identity_known: true,
            focused: true,
            lock_screen: false,
        },
        activation: Activation::Always,
        explicit: false,
        language: Some("en".to_owned()),
        window_text: None,
    }
}

#[test]
fn typing_into_a_mid_sentence_word_keeps_its_separating_space() {
    let mut shown = carry("Please find the ", "final ", Duration::from_secs(5));
    shown.after = "report today".to_owned();
    let mut next = context("Please find the f");
    next.after = "report today".to_owned();
    assert_eq!(
        shown
            .remainder(&scope("org.telegram.desktop"), &next, Instant::now(), false)
            .as_deref(),
        Ok("inal ")
    );
}

#[test]
fn typed_characters_of_the_suggestion_carry_its_remainder() {
    let shown = carry(
        "Please find attached the",
        " report for",
        Duration::from_secs(5),
    );
    let here = scope("org.telegram.desktop");
    let now = Instant::now();
    for (typed, rest) in [
        (" ", "report for"),
        (" re", "port for"),
        (" report", " for"),
        (" report f", "or"),
    ] {
        let context = context(&format!("Please find attached the{typed}"));
        assert_eq!(
            shown.remainder(&here, &context, now, false).as_deref(),
            Ok(rest)
        );
    }
}

#[test]
fn anything_but_an_exact_typed_prefix_carries_nothing() {
    let shown = carry(
        "Please find attached the",
        " report for",
        Duration::from_secs(5),
    );
    let here = scope("org.telegram.desktop");
    let now = Instant::now();
    for before in [
        "Please find attached the report for", // all of it typed
        "Please find attached the rep0",       // a different character
        "Please find attached the  ",          // an extra space
        "please find attached the re",         // an edit before the caret
        "Please find attached them re",        // an edit at the old caret
    ] {
        assert_eq!(
            shown.remainder(&here, &context(before), now, false),
            Err(CarryMiss::PrefixMismatch),
            "{before:?}"
        );
    }

    let mut after_changed = context("Please find attached the re");
    after_changed.after = "x".to_owned();
    assert_eq!(
        shown.remainder(&here, &after_changed, now, false),
        Err(CarryMiss::AfterChanged)
    );
    let mut language_changed = context("Please find attached the re");
    language_changed.language = Some("de".to_owned());
    assert_eq!(
        shown.remainder(&here, &language_changed, now, false),
        Err(CarryMiss::LanguageChanged)
    );
    assert_eq!(
        shown.remainder(
            &scope("org.other.app"),
            &context("Please find attached the re"),
            now,
            false
        ),
        Err(CarryMiss::Scope)
    );
}

#[test]
fn the_same_context_again_keeps_the_whole_suggestion_unless_asked_anew() {
    let shown = carry(
        "Please find attached the l",
        "atest version",
        Duration::from_secs(5),
    );
    let here = scope("org.telegram.desktop");
    let now = Instant::now();
    let same = context("Please find attached the l");
    assert_eq!(
        shown.remainder(&here, &same, now, false).as_deref(),
        Ok("atest version")
    );
    // An explicit request for that context asks the model.
    assert_eq!(
        shown.remainder(&here, &same, now, true),
        Err(CarryMiss::PrefixMismatch)
    );
    // Typing still carries for an explicit request.
    assert_eq!(
        shown
            .remainder(&here, &context("Please find attached the la"), now, true)
            .as_deref(),
        Ok("test version")
    );
    // Every other gate still applies to the repeat.
    let mut after_changed = same.clone();
    after_changed.after = "x".to_owned();
    assert_eq!(
        shown.remainder(&here, &after_changed, now, false),
        Err(CarryMiss::AfterChanged)
    );
    assert_eq!(
        shown.remainder(&scope("org.other.app"), &same, now, false),
        Err(CarryMiss::Scope)
    );
    let tight = carry(
        "Please find attached the l",
        "atest version",
        Duration::from_millis(300),
    );
    assert_eq!(
        tight.remainder(&here, &same, now, false),
        Err(CarryMiss::DeadlineTight)
    );
}

#[test]
fn a_remainder_colliding_with_following_text_is_shape_rejected() {
    let mut shown = carry(
        "Please find attached the",
        " report",
        Duration::from_secs(5),
    );
    shown.after = "report today".to_owned();
    let mut next = context("Please find attached the ");
    next.after = "report today".to_owned();
    assert_eq!(
        shown.remainder(&scope("org.telegram.desktop"), &next, Instant::now(), false),
        Err(CarryMiss::ShapeRejected)
    );
}

#[test]
fn a_nearly_expired_suggestion_carries_nothing() {
    let shown = carry("Thanks for the", " update", Duration::from_millis(300));
    assert_eq!(
        shown.remainder(
            &scope("org.telegram.desktop"),
            &context("Thanks for the up"),
            Instant::now(),
            false
        ),
        Err(CarryMiss::DeadlineTight)
    );
}

#[test]
fn a_sliding_window_needs_a_long_exact_shared_tail() {
    let long = "x".repeat(MIN_SLID_SHARED_BYTES + 10);
    assert_eq!(
        typed_prefix(&format!("ab{long}"), &format!("b{long} n"), " next"),
        Some(2)
    );
    assert_eq!(typed_prefix("ab short", "b short n", " next"), None);
    assert_eq!(typed_prefix("Hello", "Hello wor", " world"), Some(4));
    // Lengths are bytes of whole characters.
    assert_eq!(typed_prefix("سلام", "سلام د", " دوست"), Some(3));
}
