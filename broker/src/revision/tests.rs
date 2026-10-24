use super::{
    RevisionAbstain, RevisionMode, accept, changed_word_share, chatml_prompt, language, messages,
};

fn english() -> &'static super::RevisionLanguage {
    language("en-US").expect("English")
}

#[test]
fn the_model_is_told_the_field_language_and_shown_an_example_in_it() {
    for (tag, name) in [
        ("en", "English"),
        ("de", "German"),
        ("fr", "French"),
        ("es", "Spanish"),
        ("pt-BR", "Portuguese"),
    ] {
        let language = language(tag).expect("a revision language");
        let [system, example, answer, selection] =
            messages(RevisionMode::Grammar, language, "", "  text ");
        assert!(system.1.contains(name), "{tag}");
        assert_eq!(
            (system.0, example.0, answer.0, selection.0),
            ("system", "user", "assistant", "user")
        );
        assert_eq!(
            selection.1, "text",
            "the selection is sent without its spaces"
        );
        assert!(
            crate::language::confident_language(&example.1).is_none_or(|detected| {
                super::detected_tag(detected).is_none_or(|detected| detected == language.tag)
            }),
            "{tag}'s example is in {name}"
        );
    }
    assert!(language("fa").is_none(), "Persian is not offered");
    let prompt = chatml_prompt(&messages(RevisionMode::Refine, english(), "", "x"));
    assert!(prompt.starts_with("<|im_start|>system\n"));
    assert!(prompt.ends_with("<|im_start|>assistant\n<think>\n\n</think>\n\n"));
    assert_eq!(prompt.matches("<|im_end|>").count(), 4);
}

#[test]
fn a_corrected_selection_keeps_its_spaces_case_and_open_end() {
    let fix = |before, selected, after, output| {
        accept(
            RevisionMode::Grammar,
            english(),
            before,
            selected,
            after,
            output,
        )
    };
    assert_eq!(
        fix(
            "",
            "I has went to the store and buyed three apple.",
            "",
            "I went to the store and bought three apples."
        )
        .as_deref(),
        Ok("I went to the store and bought three apples.")
    );
    // The selection's own surrounding spaces stay; quotes the model added go.
    assert_eq!(
        fix("", " their going home ", "", "\"They're going home.\"").as_deref(),
        Ok(" They're going home. ")
    );
    // Mid-sentence: the model reads the sentence's start too, the first
    // letter keeps its case and no period is added before the rest of the
    // line, whether words or marks follow.
    assert_eq!(
        fix(
            "Yesterday ",
            "i has went to the shop",
            " with Anna.",
            "Yesterday I went to the shop."
        )
        .as_deref(),
        Ok("i went to the shop")
    );
    assert_eq!(
        fix(
            "When you get home, ",
            "please call me so we can discussed the plan",
            ", if you have time.",
            "When you get home, please call me so we can discuss the plan."
        )
        .as_deref(),
        Ok("please call me so we can discuss the plan")
    );
    // A sentence start that came back changed leaves the selection's
    // revision unknown.
    assert_eq!(
        fix(
            "Yesterday ",
            "i has went to the shop",
            "",
            "Today I went to the shop."
        ),
        Err(RevisionAbstain::Context)
    );
    // A selection at a sentence start may gain its capital.
    assert_eq!(
        fix("Done. ", "i has went", "", "I went").as_deref(),
        Ok("I went")
    );
}

#[test]
fn answers_that_change_language_facts_shape_or_too_much_are_not_shown() {
    let fix = |selected, output| accept(RevisionMode::Grammar, english(), "", selected, "", output);
    assert_eq!(
        fix("Thanks for the reply.", "Thanks for the reply."),
        Err(RevisionAbstain::Unchanged)
    );
    assert_eq!(
        fix(
            "She dont know where is the station, sadly for her.",
            "Sie weiß nicht, wo die Station ist, leider für sie."
        ),
        Err(RevisionAbstain::Language)
    );
    assert_eq!(
        fix("We meet at 10 on Friday", "We meet at 11 on Friday."),
        Err(RevisionAbstain::Facts)
    );
    assert_eq!(
        fix("ask Anna about it", "Ask her about it."),
        Err(RevisionAbstain::Facts)
    );
    assert_eq!(
        fix("mail me at a@b.example pls", "Email me, please."),
        Err(RevisionAbstain::Facts)
    );
    assert_eq!(fix("one line", "two\nlines"), Err(RevisionAbstain::Shape));
    assert_eq!(fix("plain", "<b>plain</b>"), Err(RevisionAbstain::Shape));
    assert_eq!(fix("x", ""), Err(RevisionAbstain::Shape));
    assert_eq!(
        fix(
            "the cat sat on the mat today",
            "A feline rested upon the woven rug now"
        ),
        Err(RevisionAbstain::Edits)
    );
    assert_eq!(
        fix(
            "ok",
            "Okay, I agree with everything you said and I will act on it right away today."
        ),
        Err(RevisionAbstain::Length)
    );
}

#[test]
fn a_refinement_may_reword_but_keeps_language_and_facts() {
    let refine =
        |selected, output| accept(RevisionMode::Refine, english(), "", selected, "", output);
    assert_eq!(
        refine(
            "so basically the meeting is moved to thursday because alot of people cant come on wednesday",
            "The meeting moved to Thursday because many people cannot attend on Wednesday."
        )
        .as_deref(),
        Ok("The meeting moved to Thursday because many people cannot attend on Wednesday.")
    );
    assert_eq!(
        refine("we ship 3 boxes", "We are shipping three boxes."),
        Err(RevisionAbstain::Facts)
    );
    // German nouns are capitalized, so names are not checked there.
    let german = language("de").expect("German");
    assert_eq!(
        accept(
            RevisionMode::Grammar,
            german,
            "",
            "Ich habe nach Hause gegangen",
            "",
            "Ich bin nach Hause gegangen"
        )
        .as_deref(),
        Ok("Ich bin nach Hause gegangen")
    );
}

#[test]
fn a_mid_sentence_selection_is_read_with_its_sentence_start() {
    use super::sentence_lead;
    assert_eq!(sentence_lead(""), "");
    assert_eq!(sentence_lead("It rained. "), "");
    assert_eq!(sentence_lead("Er sagt, dass "), "Er sagt, dass ");
    assert_eq!(sentence_lead("Done! Then we "), "Then we ");
    assert_eq!(sentence_lead("First line\nsecond "), "second ");
    // A long sentence start is cut to its last words, at a word.
    let long = format!("{} end ", "word ".repeat(60));
    let lead = sentence_lead(&long);
    assert!(lead.chars().count() <= 160 && lead.starts_with("word") && lead.ends_with("end "));
    let german = language("de").expect("German");
    let [_, _, _, selection] = messages(
        RevisionMode::Grammar,
        german,
        "Er sagt, dass ",
        "die Rechnung ist bezahlt worden",
    );
    assert_eq!(selection.1, "Er sagt, dass die Rechnung ist bezahlt worden");
    // The clause keeps its verb last, and only the selection is replaced.
    assert_eq!(
        accept(
            RevisionMode::Grammar,
            german,
            "Er sagt, dass ",
            "die Rechnung ist schon gestern bezahlt worden",
            " und alles gut ist.",
            "Er sagt, dass die Rechnung schon gestern bezahlt worden ist."
        )
        .as_deref(),
        Ok("die Rechnung schon gestern bezahlt worden ist")
    );
}

#[test]
fn the_changed_share_counts_words_outside_the_longest_common_run() {
    assert!((changed_word_share("a b c d", "a b c d") - 0.0).abs() < f64::EPSILON);
    assert!((changed_word_share("a b c d", "a x c d") - 0.25).abs() < f64::EPSILON);
    assert!((changed_word_share("", "") - 0.0).abs() < f64::EPSILON);
    assert!((changed_word_share("a b", "c d e f") - 1.0).abs() < f64::EPSILON);
    assert!(
        (changed_word_share("their going home", "They're going home.") - 1.0 / 3.0).abs() < 1e-9
    );
    // Short selections may change every word.
    assert_eq!(
        accept(
            RevisionMode::Grammar,
            english(),
            "",
            "its ok",
            "",
            "It's okay."
        )
        .as_deref(),
        Ok("It's okay.")
    );
}
