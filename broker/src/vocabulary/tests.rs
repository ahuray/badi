use super::*;

fn window(text: &str) -> Vocabulary {
    Vocabulary::from_texts(text, &[] as &[&str])
}

fn finish(before: &str, language: &str, vocabulary: &Vocabulary) -> Result<Completion, Abstention> {
    complete(
        before,
        "",
        WritingLanguage::from_tag(language).expect("supported language"),
        vocabulary,
    )
}

#[test]
fn a_unique_word_returns_only_its_untyped_rest() {
    for (before, language, context, suffix, word) in [
        (
            "Please use the sched",
            "en",
            "The schedule is ready.",
            "ule",
            "schedule",
        ),
        ("Hi Mar", "en", "To: Marinella Costa", "inella", "Marinella"),
        (
            "Die Tagesord",
            "de-DE",
            "Tagesordnung liegt vor.",
            "nung",
            "Tagesordnung",
        ),
        (
            "Die Tagesordnung",
            "de",
            "Tagesordnungsentwurf liegt vor.",
            "sentwurf",
            "Tagesordnungsentwurf",
        ),
        ("من هر روز می‌ر", "FA-ir", "من هر روز می‌روم.", "وم", "می‌روم"),
        (
            "Gracias, Ximen",
            "es",
            "Asunto: Reunión con Ximena",
            "a",
            "Ximena",
        ),
    ] {
        let completion = finish(before, language, &window(context)).expect(before);
        assert_eq!(completion.suffix, suffix);
        assert_eq!(completion.word, word);
        assert_eq!(completion.sources, vec![Source::Window]);
    }
}

#[test]
fn every_source_counts_and_a_repeated_word_is_one_candidate() {
    let both = Vocabulary::from_texts("Tagesordnung. Tagesordnung.", &["Tagesordnung liegt vor."]);
    let completion = finish("Die Tagesordnung und Tagesord", "de", &both).unwrap();
    assert_eq!(
        completion.sources,
        vec![Source::Window, Source::Learned, Source::Draft]
    );
    let learned = Vocabulary::from_texts("", &["Ask Okonkwo first."]);
    assert_eq!(
        finish("I will ask Okon", "en", &learned).unwrap().sources,
        vec![Source::Learned]
    );
    assert_eq!(
        finish(
            "Die Tagesordnung und Tagesord",
            "de",
            &Vocabulary::default()
        )
        .unwrap()
        .sources,
        vec![Source::Draft]
    );
}

#[test]
fn the_drafts_first_word_and_words_joined_to_others_are_not_evidence() {
    assert_eq!(
        finish("Tagesordnung und Tagesord", "de", &Vocabulary::default()),
        Err(Abstention::NoMatch),
        "the draft's first word may be clipped"
    );
    for context in [
        "XTagesordnung.",
        "Tagesordnung-Entwurf.",
        "Tagesordnung's",
        "Tagesordnung/Entwurf.",
        "Tagesordnung_Entwurf.",
        "Tagesordnung1.",
        "1Tagesordnung.",
        "tagesordnung.",
    ] {
        assert_eq!(
            finish("Die Tagesord", "de", &window(context)),
            Err(Abstention::NoMatch),
            "{context}"
        );
    }
    assert_eq!(
        Vocabulary::words("read client.rs and x-ray don't"),
        vec!["read"]
    );
}

#[test]
fn two_fitting_words_abstain_before_any_suffix_check() {
    for (before, language, context) in [
        ("Die Fran", "de", "Frankfurt. Frankreich."),
        ("Das Caf", "de", "Caf\u{301}e. Cafe."),
        ("این میخ", "fa", "میخ‌ها. میخک."),
        ("Hi Mar", "en", "Marinella and Marisol"),
    ] {
        assert_eq!(
            finish(before, language, &window(context)),
            Err(Abstention::Ambiguous),
            "{before}"
        );
    }
    assert_eq!(
        finish("این میخ", "fa", &window("میخ‌ها.")),
        Err(Abstention::UnsafeSuffix)
    );
}

#[test]
fn a_word_that_begins_every_other_fitting_word_is_offered() {
    let completion = finish(
        "Add the alm",
        "en",
        &window("Toasted almonds, or one almond"),
    )
    .unwrap();
    assert_eq!(
        (completion.suffix.as_str(), completion.word.as_str()),
        ("ond", "almond")
    );
    assert_eq!(
        finish("Add the alm", "en", &window("almonds, almond, alms")),
        Err(Abstention::Ambiguous)
    );
}

#[test]
fn case_diacritics_and_joiners_are_never_normalized() {
    for (before, language, context) in [
        ("Die Tagesord", "de", "tagesordnung."),
        ("Die Übertr", "de", "U\u{308}berträge."),
        ("برای کیا", "fa", "كيان آماده است."),
        ("من می‌ر", "fa", "من میروم."),
    ] {
        assert_eq!(
            finish(before, language, &window(context)),
            Err(Abstention::NoMatch),
            "{before}"
        );
    }
}

#[test]
fn only_a_partial_word_of_three_to_twenty_three_letters_is_finished() {
    let vocabulary = window("Tagesordnung.");
    for before in [
        "Die Ta",
        "Die Tagesord ",
        "Die Tagesord-",
        "Die/Tagesord",
        "Die Tagesord\n",
        "Tagesord",
    ] {
        assert_eq!(
            finish(before, "de", &vocabulary),
            Err(Abstention::NoStem),
            "{before:?}"
        );
    }
    assert_eq!(
        finish(&format!("Die {}", "T".repeat(24)), "de", &vocabulary),
        Err(Abstention::NoStem)
    );
    assert_eq!(
        finish("The abc", "en", &window(&format!("abc{}.", "d".repeat(30)))),
        Err(Abstention::NoMatch),
        "an overlong token is not vocabulary"
    );
}

#[test]
fn a_caret_before_a_letter_finishes_nothing_but_one_before_punctuation_may() {
    let vocabulary = window("The schedule is ready.");
    let english = WritingLanguage::English;
    assert_eq!(
        complete("Use the sched", "x today", english, &vocabulary),
        Err(Abstention::NoStem)
    );
    assert_eq!(
        complete("Use the sched", ", please", english, &vocabulary).map(|done| done.suffix),
        Ok("ule".to_owned())
    );
}

#[test]
fn a_typed_english_word_may_be_finished_unless_its_capital_marks_a_name() {
    let vocabulary = window("Format the Marinella report.");
    assert_eq!(
        finish("This is for", "en", &vocabulary),
        Err(Abstention::KnownWord)
    );
    assert_eq!(
        finish("Thanks. For", "en", &window("Format.")),
        Err(Abstention::KnownWord)
    );
    assert_eq!(
        finish("Hi Mar", "en", &vocabulary).map(|done| done.word),
        Ok("Marinella".to_owned()),
        "mar is a word, but a capital mid-sentence marks a name"
    );
    assert_eq!(
        finish("Die Tag", "de", &window("Tagesordnung.")).map(|done| done.word),
        Ok("Tagesordnung".to_owned()),
        "only English has a lexicon to tell"
    );
}

#[test]
fn texts_become_bounded_distinct_word_lists() {
    assert_eq!(
        Vocabulary::words("Re: Venue deposit — the venue, the Venue. Okonkwo!"),
        vec!["Venue", "deposit", "venue", "Okonkwo"]
    );
    // Distinct letter-only words: "word" plus the index written in letters.
    let lettered = |index: usize| -> String {
        let mut word = String::from("word");
        let mut rest = index;
        loop {
            word.push(char::from(b'a' + u8::try_from(rest % 26).unwrap()));
            rest /= 26;
            if rest == 0 {
                return word;
            }
        }
    };
    let many: Vec<String> = (0..2000).map(lettered).collect();
    let bounded = Vocabulary::from_texts(&many.join(" "), &many);
    assert_eq!(bounded.window.len(), MAX_WINDOW_WORDS);
    assert_eq!(bounded.learned.len(), MAX_LEARNED_WORDS);
    assert_eq!(
        bounded.window[..3],
        many[..3],
        "the first words are kept, in order"
    );
    assert!(Vocabulary::default().is_empty());
}
