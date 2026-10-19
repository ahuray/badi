use super::*;

/// A tiny German and French dictionary in a temporary directory.
fn fixture() -> (tempfile::TempDir, Spelling) {
    let directory = tempfile::tempdir().expect("directory");
    let aff = "SET UTF-8\nTRY esianrtolcdugmphbyfvkwzäöüßESIANRTOLCDUGMPHBYFVKWZ\n";
    for (name, words) in [
        ("de_DE", "Haus\nHass\nnämlich\nmorgen\nich\ngehe\nins\n"),
        ("fr_FR", "bonjour\nmerci\nbeaucoup\n"),
    ] {
        std::fs::write(directory.path().join(format!("{name}.aff")), aff).expect("aff");
        let count = words.lines().count();
        std::fs::write(
            directory.path().join(format!("{name}.dic")),
            format!("{count}\n{words}"),
        )
        .expect("dic");
    }
    let spelling = Spelling::new(vec![directory.path().to_owned()]);
    (directory, spelling)
}

fn enable(spelling: &Spelling, languages: &[SpellingLanguage]) {
    let generation = spelling.enable(languages);
    spelling.load(generation);
}

fn located_word(located: Option<Located<'_>>) -> Option<&str> {
    located.map(|located| match located {
        Located::Dictionary { target, .. } | Located::English(target) => target.word,
    })
}

#[test]
fn an_unknown_word_after_its_space_gets_close_candidates() {
    let (_directory, spelling) = fixture();
    enable(&spelling, &[SpellingLanguage::German]);
    let before = "ich gehe ins Hauss ";
    let Some(Located::Dictionary { language, target }) = spelling.locate(before, Some("de"), false)
    else {
        panic!("expected a German correction target")
    };
    assert_eq!(
        (target.word, target.suffix, target.delimiter),
        ("Hauss", "Hauss ", " ")
    );
    let candidates = spelling.candidates(language, target.word);
    assert!(candidates.contains(&"Haus".to_owned()), "{candidates:?}");
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.starts_with('H'))
    );
    assert_eq!(
        located_word(spelling.locate("ich gehe ins Haus ", Some("de"), false)),
        None
    );
}

#[test]
fn a_word_still_being_typed_is_checked_only_on_request() {
    let (_directory, spelling) = fixture();
    enable(&spelling, &[SpellingLanguage::German]);
    assert_eq!(
        located_word(spelling.locate("ich gehe morgn", Some("de"), false)),
        None
    );
    assert_eq!(
        located_word(spelling.locate("ich gehe morgn", Some("de"), true)),
        Some("morgn")
    );
}

#[test]
fn only_enabled_languages_with_a_dictionary_are_corrected() {
    let (_directory, spelling) = fixture();
    // German is not enabled.
    assert_eq!(
        located_word(spelling.locate("ich gehe ins Hauss ", Some("de"), false)),
        None
    );
    // Enabled, but no dictionary is installed for it.
    enable(&spelling, &[SpellingLanguage::Italian]);
    assert_eq!(
        located_word(spelling.locate("vado a casaa ", Some("it"), false)),
        None
    );
}

#[test]
fn a_confident_detection_outranks_the_fields_hint() {
    let (_directory, spelling) = fixture();
    enable(
        &spelling,
        &[SpellingLanguage::English, SpellingLanguage::German],
    );
    // French prose in a field hinted English is never corrected as English.
    let french = "Merci beaucoup pour votre message et pour le temps que vous nous avez consacré hier soir, bonjur ";
    assert_eq!(
        located_word(spelling.locate(french, Some("en"), false)),
        None
    );
    enable(&spelling, &[SpellingLanguage::French]);
    assert_eq!(
        located_word(spelling.locate(french, Some("en"), false)),
        Some("bonjur")
    );
}

#[test]
fn names_acronyms_and_joined_words_are_left_alone() {
    let french = SpellingLanguage::French;
    assert!(
        dictionary_target("je parle avec Marrie ", french).is_none(),
        "a name mid-sentence"
    );
    assert!(
        dictionary_target("Bonjur ", french).is_some(),
        "a sentence start"
    );
    assert!(
        dictionary_target("das grosse Hauss ", SpellingLanguage::German).is_some(),
        "a German noun"
    );
    assert!(
        dictionary_target("le PDFF ", french).is_none(),
        "an acronym"
    );
    assert!(
        dictionary_target("un mot-clef ", french).is_none(),
        "a joined word"
    );
    assert!(dictionary_target("x2bonjur ", french).is_none());
}

#[test]
fn a_newer_configuration_wins_over_a_slow_load() {
    let (_directory, spelling) = fixture();
    let stale = spelling.enable(&[SpellingLanguage::German]);
    let current = spelling.enable(&[SpellingLanguage::French]);
    spelling.load(stale);
    assert!(spelling.dictionary(SpellingLanguage::German).is_none());
    spelling.load(current);
    assert!(spelling.dictionary(SpellingLanguage::French).is_some());
}

#[test]
fn candidates_are_close_words_of_letters_and_the_model_must_name_one() {
    assert_eq!(edit_distance("recieve", "receive"), 1);
    assert_eq!(edit_distance("Strasse", "Straße"), 2);
    assert!(plausible_candidate("grasias", "gracias"));
    assert!(
        !plausible_candidate("adress", "dress"),
        "the first letter stays"
    );
    assert!(!plausible_candidate("adress", "a dress"));
    assert!(!plausible_candidate("vieleicht", "gleichviel"));
    let candidates = vec!["gracias".to_owned(), "grasas".to_owned()];
    assert_eq!(chosen(" gracias.\n", &candidates), Some(&candidates[0]));
    assert_eq!(chosen("Gracias", &candidates), None);
    assert_eq!(chosen("thanks", &candidates), None);
}

#[test]
fn the_choice_prompt_opens_the_answer_on_its_own_line() {
    let candidates = vec!["Haus".to_owned(), "Hass".to_owned()];
    let payload = choice_payload("Wir treffen uns morgen im Hauss ", "Hauss", &candidates);
    let prompt = payload["prompt"].as_str().expect("prompt");
    assert_eq!(
        prompt,
        "Wir treffen uns morgen im Hauss\nIn this sentence, \"Hauss\" is a typo for one of: Haus, Hass.\nThe intended word is \""
    );
    assert_eq!(payload["stop"], json!(["\n"]));
    assert_eq!(payload["cache_prompt"], json!(false));
    // The answer is read up to its closing quote; an echoed typo matches no
    // candidate and abstains.
    assert_eq!(chosen("Haus\" because", &candidates), Some(&candidates[0]));
    assert_eq!(chosen("Hass\"", &candidates), Some(&candidates[1]));
    assert_eq!(chosen("Hauss\" is", &candidates), None);
    assert_eq!(chosen("\"", &candidates), None);
}

#[test]
fn language_tags_name_their_primary_language() {
    assert_eq!(
        SpellingLanguage::from_tag("de-AT"),
        Some(SpellingLanguage::German)
    );
    assert_eq!(
        SpellingLanguage::from_tag("fr_FR"),
        Some(SpellingLanguage::French)
    );
    assert_eq!(SpellingLanguage::from_tag("fa"), None);
}
