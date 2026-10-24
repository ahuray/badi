use super::{CarryScope, DISMISSED, Rejections, TYPED_OVER};

fn scope() -> CarryScope {
    CarryScope::desktop_app("org.telegram.desktop")
}

#[test]
fn a_dismissed_next_word_is_withheld_at_its_spot_only() {
    let mut rejections = Rejections::default();
    let before = "Thank you so much for your ";
    rejections.record(&scope(), before, "interest in our products.", DISMISSED);
    assert!(rejections.withholds(&scope(), before, "interest in our products."));
    assert!(
        rejections.withholds(&scope(), "THANKS so much  for your ", "Interest, really"),
        "the same last words and next word, in any case and spacing"
    );
    assert!(!rejections.withholds(&scope(), before, "help with this."));
    assert!(!rejections.withholds(&scope(), "Thanks for your ", "interest in our products."));
    assert!(
        rejections.withholds(&scope(), "Thank you so much for your", " interest"),
        "the suggestion's leading space belongs to the spot"
    );
    assert!(
        !rejections.withholds(&scope(), "Thank you so much for your", "interest"),
        "a word that would join the last one is another spot"
    );
    assert!(!rejections.withholds(&CarryScope::desktop_app("omawrite"), before, "interest"));
}

#[test]
fn taking_a_word_forgets_its_turn_down_at_that_spot() {
    let mut rejections = Rejections::default();
    let before = "Thank you so much for your ";
    rejections.record(&scope(), before, "interest in our products.", DISMISSED);
    rejections.record(&scope(), "Kind regards, ", "Ahura", DISMISSED);
    rejections.forget(&scope(), "thank you  SO much for your ", "Interest, really");
    assert!(!rejections.withholds(&scope(), before, "interest in our products."));
    assert!(rejections.withholds(&scope(), "Kind regards, ", "Ahura"));
}

#[test]
fn typing_over_takes_two_turn_downs_and_names_the_word_being_typed() {
    let mut rejections = Rejections::default();
    let before = "Thank you so much for your ";
    let text = "interest in our products.";
    rejections.record_typed_over(&scope(), before, text, "h");
    assert!(
        !rejections.withholds(&scope(), before, text),
        "one typed-over word may be a typo"
    );
    rejections.record_typed_over(&scope(), before, text, "he");
    assert!(rejections.withholds(&scope(), before, text));

    // Words typed in full were taken; the word typed over was not.
    let mut later = Rejections::default();
    later.record_typed_over(&scope(), before, text, "interest in my");
    later.record_typed_over(&scope(), before, text, "interest in m");
    let spot = "Thank you so much for your interest in ";
    assert!(later.withholds(&scope(), spot, "our products."));
    assert!(
        !later.withholds(&scope(), before, text),
        "interest itself was taken"
    );

    // Letters that match the suggestion so far turn nothing down.
    let mut matching = Rejections::default();
    matching.record_typed_over(&scope(), before, text, "inter");
    matching.record_typed_over(&scope(), before, text, "inter");
    assert!(!matching.withholds(&scope(), before, text));
}

#[test]
fn memory_is_bounded_and_forgets_the_oldest_spot_first() {
    let mut rejections = Rejections::default();
    rejections.record(&scope(), "first spot here ", "word", DISMISSED);
    for index in 0..super::CAPACITY {
        rejections.record(
            &scope(),
            &format!("spot number {index} "),
            "word",
            TYPED_OVER,
        );
    }
    assert!(!rejections.withholds(&scope(), "first spot here ", "word"));
    assert_eq!(rejections.entries.len(), super::CAPACITY);
    assert!(
        !rejections.withholds(&scope(), "", "  ..."),
        "no word, no key"
    );
}
