use super::{
    ENGLISH_LEXICON, caret_inside_word, complete_word_prefix, correction_word, english_word_prefix,
    known_english_word, line_rest, memory_retry_delay, mid_line_insertion, valid_correction,
    validate_proposal, word_follows_caret,
};

#[test]
fn a_word_after_the_caret_is_not_punctuation() {
    assert!(word_follows_caret("report today"));
    assert!(word_follows_caret(" report today"));
    assert!(!word_follows_caret("."));
    assert!(!word_follows_caret(" ."));
    assert!(!word_follows_caret(""));
    assert!(!word_follows_caret("\nreport"));
}

#[test]
fn mid_line_insertions_end_where_the_continuation_rejoins_the_line() {
    let cases = [
        // The next word after a space, after the caret's own space, and
        // after punctuation.
        (
            "Please find the",
            " report today",
            " final report for",
            Some(" final"),
        ),
        (
            "Please find the ",
            "report today",
            "final report for",
            Some("final "),
        ),
        ("Thanks", ", John.", " a lot, John", Some(" a lot")),
        // The rest of a word being typed, with or without a rejoin.
        (
            "Please find the upd",
            " report",
            "ated report",
            Some("ated"),
        ),
        (
            "Please find the upd",
            " report",
            "ated version of the",
            Some("ated"),
        ),
        ("I will send the docu", ".", "ment", Some("ment")),
        // Nothing fits: the continuation starts with the next word, never
        // rejoins at a word boundary, or rejoins inside a word.
        ("Please find the", " report", " report for you", None),
        ("Please find the", " report", " latest version of", None),
        ("Please find the", " report", " reports and", None),
        ("Please find the", " port", " report", None),
    ];
    for (before, line, continuation, expected) in cases {
        assert_eq!(
            mid_line_insertion(before, line, continuation).as_deref(),
            expected,
            "{before:?} | {line:?} <- {continuation:?}"
        );
    }
}

#[test]
fn a_phrase_closed_with_a_clause_mark_fits_before_the_line_s_mark() {
    let insert =
        |line, continuation| mid_line_insertion("I hope this email finds you", line, continuation);
    // The stop word stays in the continuation, so a period rejoins.
    assert_eq!(insert(". Best", " well.").as_deref(), Some(" well"));
    assert_eq!(insert(", Best", " well.").as_deref(), Some(" well"));
    assert_eq!(
        insert("? Best", " well today?").as_deref(),
        Some(" well today")
    );
    // Not closed, or the next word did not come back: no proof it fits.
    assert_eq!(insert(", Best", " well and"), None);
    assert_eq!(insert(" today", " well."), None);
}

#[test]
fn mid_line_requests_need_a_word_boundary_and_text_on_the_line() {
    assert!(caret_inside_word("Please find the rep", "ort"));
    assert!(!caret_inside_word("Please find the rep", " today"));
    assert!(!caret_inside_word("Please find the ", "report"));
    assert_eq!(line_rest(" report\nNext"), Some(" report"));
    assert_eq!(line_rest(" \nNext paragraph"), None);
    assert_eq!(line_rest(""), None);
    // A word completion may end right before punctuation or a space, and
    // the inserted words must not repeat the word after the caret.
    assert!(validate_proposal("I will send the docu", ".", "ment", Some("en")).is_ok());
    assert!(validate_proposal("Please find the upd", " report", "ated", Some("en")).is_ok());
    assert!(validate_proposal("Please find the", " report", " report", Some("en")).is_err());
}

#[test]
fn in_place_lexicon_lookups_match_a_sorted_word_list() {
    let words: Vec<&str> = ENGLISH_LEXICON.lines().collect();
    assert_eq!(words.len(), 77_928);
    assert!(words.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(ENGLISH_LEXICON.ends_with('\n') && !ENGLISH_LEXICON.contains('\r'));
    let known = |word: &str| {
        word.bytes().all(|byte| byte.is_ascii_alphabetic())
            && words
                .binary_search(&word.to_ascii_lowercase().as_str())
                .is_ok()
    };
    let prefix = |word: &str| {
        let index = words.partition_point(|candidate| *candidate < word);
        words
            .get(index)
            .is_some_and(|candidate| candidate.starts_with(word))
    };
    let mut probes: Vec<String> = [
        "", "A", "Zzz", "zzzzzz", "{", "0", "`", "a b", "\u{200c}", "naïve", "é", "Teh",
    ]
    .map(str::to_owned)
    .to_vec();
    for word in &words {
        // Each word in both cases, its shorter prefix, and probes just
        // after it, among its extensions and past them.
        probes.push((*word).to_owned());
        probes.push(word.to_ascii_uppercase());
        probes.push(word[..word.len() - 1].to_owned());
        probes.push(format!("{word}\0"));
        probes.push(format!("{word}q"));
        probes.push(format!("{word}{{"));
    }
    for probe in &probes {
        assert_eq!(known_english_word(probe), known(probe), "{probe:?}");
        assert_eq!(english_word_prefix(probe), prefix(probe), "{probe:?}");
    }
}

#[test]
fn short_memory_delays_start_with_bounded_backoff() {
    let model = crate::model_selection::catalog(crate::model_selection::ModelUseCase::Writing)[1];
    assert_eq!(memory_retry_delay(model, Some(16_000), None), None);
    assert_eq!(memory_retry_delay(model, None, None), None);
    let mut delays = Vec::new();
    let mut previous = None;
    for _ in 0..7 {
        previous = memory_retry_delay(model, Some(4_000), previous);
        delays.push(previous.expect("short memory waits").as_secs());
    }
    assert_eq!(delays, [2, 4, 8, 16, 30, 30, 30]);
    assert_eq!(memory_retry_delay(model, Some(8_000), previous), None);
}

#[test]
fn token_healing_binds_the_exact_typed_stem() {
    let request = crate::provider::ProviderRequest {
        before: "Please review the docum".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let payload = super::completion_payload(&request);
    assert_eq!(payload["prompt"], "Please review the");
    assert_eq!(payload["grammar"], "root ::= \" docum\" [^<>\\n\\r`]*");
    let spaced = crate::provider::ProviderRequest {
        before: "Please review the docum ".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let spaced = super::completion_payload(&spaced);
    assert_eq!(spaced["prompt"], "Please review the");
    assert_eq!(spaced["grammar"], "root ::= \" docum \" [^<>\\n\\r`]*");
    assert_eq!(payload["cache_prompt"], true);
    assert_eq!(
        super::strip_healed_prefix("document", Some("docum")),
        Some("ent")
    );
    assert_eq!(super::strip_healed_prefix("doc", Some("docum")), None);
    assert_eq!(super::strip_healed_prefix("report", Some("docum")), None);
    assert_eq!(super::healing_prefix("Please review "), None);
    assert_eq!(super::healing_prefix("Please review."), None);
    assert_eq!(super::healing_prefix("We use version1"), None);
    assert_eq!(super::healing_prefix("Please find attached the"), None);
    let rolled_back = crate::provider::ProviderRequest {
        before: "Please review the docum ".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let rolled_back = super::completion_plan(&rolled_back, &[]);
    assert_eq!(rolled_back.prompt, "Please review the docum ");
    assert_eq!(rolled_back.echo, None);
    let lone_word = crate::provider::ProviderRequest {
        before: "Docum ".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let lone_word = super::completion_plan(&lone_word, super::TRAILING_SPACE_HEALING);
    assert_eq!(lone_word.prompt, "Docum");
    assert_eq!(lone_word.echo, Some(" "));
    let bare_stem = crate::provider::ProviderRequest {
        before: "Hel".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let bare_stem = super::completion_plan(&bare_stem, super::TRAILING_SPACE_HEALING);
    assert_eq!(bare_stem.prompt, "");
    assert_eq!(bare_stem.echo, None);
    assert_eq!(super::healing_prefix("او می‌تواند"), Some("می‌تواند"));
    assert_eq!(
        super::available_complete_words("ent and prov"),
        Some("ent and".to_owned())
    );
    assert_eq!(super::available_complete_words("partial"), None);
}

#[test]
fn cold_inference_uses_a_bounded_sentence_without_changing_authority_context() {
    let original = format!(
        "{} Previous sentence. Please review the docum",
        "Earlier context. ".repeat(20)
    );
    let request = crate::provider::ProviderRequest {
        before: original.clone(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let payload = super::completion_payload(&request);
    assert_eq!(payload["prompt"], "Please review the");
    assert_eq!(request.before, original);
    for context in ["word ".repeat(100), "فارسی ".repeat(100), "x".repeat(300)] {
        // The coarse cut lets the window grow one step past 160 scalars.
        assert!(super::inference_context(&context).chars().count() <= 224);
    }
    assert_eq!(
        super::inference_context("We use version 2.5 in production"),
        "We use version 2.5 in production"
    );
    for (context, expected) in [
        (
            "Thanks for sharing the draft. I will read it",
            "I will read it",
        ),
        (
            "Vielen Dank für Ihre Nachricht. Ich werde sie morgen",
            "Ich werde sie morgen",
        ),
        ("از پیام شما ممنونم. من فردا گزارش را", "من فردا گزارش را"),
        ("آماده هستید؟ من فردا گزارش را", "من فردا گزارش را"),
        (
            "The user chose blue.\nMake the heading bold",
            "Make the heading bold",
        ),
        ("A complete sentence. ", "A complete sentence. "),
        (
            "domain.example remains intact",
            "domain.example remains intact",
        ),
    ] {
        assert_eq!(super::inference_context(context), expected);
    }
    let german = crate::provider::ProviderRequest {
        before: "Vielen Dank für Ihre".to_owned(),
        after: String::new(),
        language: Some("de".to_owned()),
    };
    assert_eq!(super::completion_payload(&german)["prompt"], german.before);
    assert!(super::completion_plan(&german, &[]).echo.is_none());
}

#[test]
fn a_short_sentence_start_keeps_the_previous_sentences_in_the_window() {
    for context in [
        // Fewer than four words after the last sentence end.
        "Thanks for sharing the draft. I will",
        "Thanks for sharing the draft. I will send",
        "Vielen Dank für Ihre Nachricht. Ich werde",
        "Aus dem Protokoll ist der Grund nicht ersichtlich. Deshalb ",
        "از پیام شما ممنونم. من فردا",
        "آماده هستید؟ من فردا",
        "Great news! We",
        "The user chose blue.\nMake the heading",
        "Previous line\nNext line",
        "Hi Anna,\n\nThanks",
        "Hi. Thanks. See you",
        "First sentence. Second one. ",
        // ZWNJ joins one word: three words, not four.
        "پیام شما رسید. من می\u{200c}خواهم فردا",
        // A lone dash is not a word: three words, not four.
        "Thanks for the draft. Yes – I will",
        // Abbreviations end a sentence; each piece is still short.
        "Wir brauchen Obst, z. B. Äpfel",
    ] {
        assert_eq!(super::inference_context(context), context, "{context:?}");
    }
    for (context, expected) in [
        // Extension stops once the kept sentences have four words.
        (
            "I read your notes. The draft looks good to me. Thanks. Please",
            "The draft looks good to me. Thanks. Please",
        ),
        (
            "Thanks for sharing the draft. I will send it",
            "I will send it",
        ),
        (
            "پیام شما رسید. من می\u{200c}خواهم فردا صبح",
            "من می\u{200c}خواهم فردا صبح",
        ),
        // `z. B.` is treated as a sentence end (documented limitation).
        (
            "Ich brauche Zutaten, z. B. Mehl, Zucker, Eier und Butter",
            "Mehl, Zucker, Eier und Butter",
        ),
    ] {
        assert_eq!(super::inference_context(context), expected, "{context:?}");
    }
    // The window and its whitespace cut still bound the prompt (160 scalars
    // plus at most one 64-scalar cut step), even when the previous sentence
    // begins before the window.
    for (sentence, start) in [
        ("word ".repeat(40), "Please"),
        ("Wort ".repeat(40), "Bitte"),
        ("کلمه ".repeat(40), "لطفا"),
    ] {
        let before = format!("{}. {start} ", sentence.trim_end());
        let context = super::inference_context(&before);
        assert!(context.chars().count() <= 224, "{context:?}");
        assert!(context.ends_with(&format!(". {start} ")), "{context:?}");
        let removed = &before[..before.len() - context.len()];
        assert!(removed.is_empty() || removed.ends_with(' '), "{removed:?}");
        assert!(context.starts_with(sentence.split(' ').next().expect("word")));
    }
    // Typed in order, the previous request's prompt is a prefix of the
    // prompt at the next sentence start, so the runtime reuses its cache.
    let previous = super::inference_context("Thanks for sharing the draft");
    let next = super::inference_context("Thanks for sharing the draft. I");
    assert_eq!(next, "Thanks for sharing the draft. I");
    assert!(next.starts_with(previous));

    // A single sentence past the window keeps one fixed start per cut step,
    // so word-by-word appends extend the previous prompt instead of sliding
    // its first bytes: consecutive contexts share a prefix except at the
    // rare step boundary, where the new context is a suffix of the old text.
    let words: Vec<String> = (0..80).map(|index| format!("word{index}")).collect();
    let mut typed = String::new();
    let mut previous = String::new();
    let mut steps = 0;
    let mut extensions = 0;
    for word in &words {
        if !typed.is_empty() {
            typed.push(' ');
        }
        typed.push_str(word);
        let context = super::inference_context(&typed).to_owned();
        if !previous.is_empty() {
            if context.starts_with(&previous) {
                extensions += 1;
            } else {
                steps += 1;
                assert!(
                    typed.ends_with(&context) && context.chars().count() >= 150,
                    "{context:?}"
                );
            }
        }
        previous = context;
    }
    // 80 words ≈ 560 scalars: with 64-scalar steps the start can move only
    // a handful of times; a per-request slide would move it nearly always.
    assert!(extensions > 60, "extensions={extensions} steps={steps}");
    assert!(steps <= 8, "extensions={extensions} steps={steps}");
    let request = crate::provider::ProviderRequest {
        before: "Previous notes are here. Thanks for sharing the draft. Please ".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let plan = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
    assert_eq!(plan.prompt, "Thanks for sharing the draft. Please");
    assert_eq!(plan.echo, Some(" "));
}

#[test]
fn trailing_space_healing_is_promoted_per_language() {
    use super::WritingLanguage::{English, German, Persian};
    // A reviewed per-language promotion: a blinded review cleared English
    // and Persian, and German failed it. Change it only after a new review.
    assert_eq!(super::TRAILING_SPACE_HEALING, [English, Persian]);
    for (tag, language, before) in [
        ("en-US", English, "Please review the "),
        ("de", German, "Bitte lies das "),
        ("fa-IR", Persian, "لطفا این گزارش را "),
    ] {
        let request = crate::provider::ProviderRequest {
            before: before.to_owned(),
            after: String::new(),
            language: Some(tag.to_owned()),
        };
        let legacy = super::completion_plan(&request, &[]).payload();
        assert_eq!(legacy["prompt"], before);
        assert!(legacy.get("grammar").is_none());
        let promoted = super::completion_plan(&request, &[language]);
        assert_eq!(promoted.prompt, before.trim_end_matches(' '));
        assert_eq!(promoted.echo, Some(" "));
        // Promoting the other languages leaves this one unhealed.
        let others: Vec<_> = [English, German, Persian]
            .into_iter()
            .filter(|other| *other != language)
            .collect();
        assert_eq!(super::completion_plan(&request, &others).payload(), legacy);
        let production = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
        assert_eq!(
            production.echo.is_some(),
            super::TRAILING_SPACE_HEALING.contains(&language)
        );
        assert_eq!(
            super::heals_trailing_space(tag),
            super::TRAILING_SPACE_HEALING.contains(&language)
        );
    }
    assert!(!super::heals_trailing_space("ar"));
}

#[test]
fn trailing_space_boundary_preserves_exact_bytes_and_the_eight_token_budget() {
    for (language, before) in [
        ("en-US", "Please review the "),
        ("de-DE", "Bitte lies das "),
        ("fa", "لطفا این گزارش را "),
        ("en", "Please review the  "),
    ] {
        let request = crate::provider::ProviderRequest {
            before: before.to_owned(),
            after: String::new(),
            language: Some(language.to_owned()),
        };
        // The mechanism, independent of which languages production promotes.
        let all = [
            super::WritingLanguage::English,
            super::WritingLanguage::German,
            super::WritingLanguage::Persian,
        ];
        let plan = super::completion_plan(&request, &all);
        let original = super::inference_context(before);
        assert_eq!(
            format!("{}{}", plan.prompt, plan.echo.expect("ASCII echo")),
            original
        );
        assert_eq!(
            plan.echo,
            Some(&original[original.trim_end_matches(' ').len()..])
        );
        assert_eq!(plan.max_tokens, 8);
        let payload = plan.payload();
        assert_eq!(payload["stop"], serde_json::json!([".", "\n"]));
        assert_eq!(payload["seed"], 42);
        assert_eq!(payload["cache_prompt"], true);
        assert_eq!(payload["timings_per_token"], true);
        assert_eq!(request.before, before, "editing context must not change");
        let legacy = super::completion_plan(&request, &[]);
        assert_eq!(legacy.prompt, original);
        assert_eq!(legacy.echo, None);
        assert_eq!(legacy.max_tokens, 8);
    }
}

#[test]
fn boundary_candidate_keeps_legacy_stems_and_never_trims_other_whitespace() {
    for (language, before, tokens) in [
        ("en", "Please review the docum", 12),
        ("en", "Please review the", 8),
        ("de", "Bitte lies die Dokum", 8),
        ("fa", "لطفا این گزار", 8),
        ("en", "Please review the\u{a0}", 8),
        ("en", "Please review the\t", 8),
        ("en", "Please review the\n", 8),
        ("en", "Please review the \n", 8),
    ] {
        let request = crate::provider::ProviderRequest {
            before: before.to_owned(),
            after: String::new(),
            language: Some(language.to_owned()),
        };
        let legacy = super::completion_plan(&request, &[]);
        let candidate = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
        if before == "Please review the docum" {
            assert_eq!(legacy.prompt, before);
            assert!(
                legacy.echo.is_none(),
                "removing English rolls word healing back"
            );
            assert_eq!(candidate.prompt, "Please review the");
            assert_eq!(candidate.echo, Some(" docum"));
        } else {
            assert_eq!(legacy.payload(), candidate.payload());
        }
        assert_eq!(candidate.max_tokens, tokens);
        assert_eq!(
            format!("{}{}", candidate.prompt, candidate.echo.unwrap_or("")),
            super::inference_context(before)
        );
        assert_eq!(request.before, before);
    }
    let before = format!(
        "{} Earlier sentence. Please review the  ",
        "Background. ".repeat(30)
    );
    let request = crate::provider::ProviderRequest {
        before: before.clone(),
        after: String::new(),
        language: Some("en".to_owned()),
    };
    let plan = super::completion_plan(&request, super::TRAILING_SPACE_HEALING);
    // Three words keep the previous sentence, but not the whole window.
    assert_eq!(plan.prompt, "Earlier sentence. Please review the");
    assert_eq!(plan.echo, Some("  "));
    assert_eq!(request.before, before);
}

#[test]
fn writing_continues_same_script_words_without_relaxing_other_boundaries() {
    for (before, suggestion) in [
        ("Please review the docum", "entation before the meeting"),
        ("The software should autom", "atically save changes"),
        ("Die neue Dokum", "entation ist verfügbar"),
        ("این گزار", "ش را بررسی کنید"),
    ] {
        assert!(super::validate_suggestion_shape(before, "", suggestion).is_ok());
        assert!(crate::segment::validate_suggestion_shape(before, "", suggestion).is_err());
    }
    for (before, after, suggestion) in [
        ("Please review ", "", " the document"),
        ("Please review", "", " review"),
        ("Please review", "", "  the document"),
        ("Please rev", " existing", "iew existing"),
        ("Please rev", "iew", "iew"),
        ("Please rev", "", "بررسی"),
        ("version 12", "", "34"),
    ] {
        assert!(super::validate_suggestion_shape(before, after, suggestion).is_err());
    }
}

#[test]
fn partial_word_proposals_require_a_real_english_word() {
    for (before, suggestion) in [
        ("Please review the docum", "ent and provide a"),
        ("The software should autom", "atically detect changes"),
    ] {
        assert!(super::validate_proposal(before, "", suggestion, Some("en")).is_ok());
    }
    for (before, suggestion, language) in [
        ("Please review the docum", "net and provide a", "en"),
        ("I use omarchy", "thm's method for the", "en"),
        ("I use Badi", "ou's concept", "en"),
        ("Neue Dokum", "entation ist verfügbar", "de"),
        ("این گزار", "ش را بررسی کنید", "fa"),
    ] {
        assert!(super::validate_proposal(before, "", suggestion, Some(language)).is_err());
    }
    for (before, suggestion, language) in [
        ("I use Badi", " for writing", "en"),
        ("Vielen Dank für Ihre", " Nachricht.", "de"),
        ("گزارش را", " بررسی کنید", "fa"),
    ] {
        assert!(super::validate_proposal(before, "", suggestion, Some(language)).is_ok());
    }
}

#[test]
fn fact_fence_rejects_invented_numbers_and_stop_word_ordinals() {
    use super::introduces_unsupported_fact as rejects;
    for (context, proposal) in [
        ("Ich freue mich auf", " die Teilnahme am 1."),
        ("Ich freue mich auf", " die Teilnahme am 1"),
        ("The survey ran in ", "2024, but"),
        ("This essay is ", "100% original and"),
        ("Our office is open from ", "10:00 AM to"),
        ("Meet me at room 42, then room", " 4"),
        ("Meet me at room 42, then room", " 420"),
        ("Meet me at room 42, then room", " 42 or 43"),
        ("We use version 2.5 in production and", " 2.6 later"),
        // The number is known, but a stop-word period may be an ordinal
        // or a decimal cut at the stop word.
        ("Die Frist ist der 1. Mai. Wir treffen uns am", " 1."),
        ("Meet me at room 42, then room", " 42."),
        ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۳ برویم"),
        ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۲."),
        ("پیام شما رسید. من فردا", " ساعت ۱۰ می‌آیم"),
        ("پیام شما رسید. من فردا", " ساعت \u{0661}\u{0660} می‌آیم"),
    ] {
        assert!(rejects(context, proposal), "{context:?} + {proposal:?}");
    }
    for (context, proposal) in [
        ("Meet me at room 42, then room", " 42"),
        ("Meet me at room 42, then room", " 42 again"),
        ("We use version 2.5 in production and", " 2.5 remains"),
        ("جلسه در اتاق ۴۲ است. بعد به اتاق", " ۴۲ برویم"),
        // Digit scripts normalize before comparison.
        ("جلسه در اتاق ۴۲ است. بعد به اتاق", " 42 برویم"),
        ("Meet me at room 42, then room", " \u{0664}\u{0662}"),
        ("Ich freue mich auf", " die Teilnahme."),
        ("Thank you", " for your time."),
        ("Please find attached the", " latest version of the"),
    ] {
        assert!(!rejects(context, proposal), "{context:?} + {proposal:?}");
    }
}

#[test]
fn activation_report_begins_with_the_provider_line() {
    use crate::model_selection::{ModelUseCase, catalog};
    use crate::semantic::runtime::WarmUpReport;
    use std::time::Duration;

    let model = catalog(ModelUseCase::Writing)[0];
    let provider = format!(
        "provider=local_model model={} quantization={}",
        model.filename, model.quantization
    );
    assert_eq!(
        super::activation_report(&model, WarmUpReport::new(Duration::from_millis(412), None)),
        format!("{provider}\nbadi-broker: writing runtime warm-up completed elapsed_ms=412")
    );
    assert_eq!(
        super::activation_report(
            &model,
            WarmUpReport::new(Duration::from_millis(2000), Some("timeout"))
        ),
        format!(
            "{provider}\nbadi-broker: writing runtime warm-up failed class=timeout elapsed_ms=2000"
        )
    );
}

#[test]
fn writing_languages_have_explicit_script_and_format_boundaries() {
    use super::WritingLanguage::{English, German, Persian};
    assert_eq!(super::WritingLanguage::from_tag("DE-de"), Some(German));
    assert_eq!(super::WritingLanguage::from_tag("fa-IR"), Some(Persian));
    assert_eq!(super::WritingLanguage::from_tag("ar"), None);
    assert!(English.accepts_output(" the next step"));
    assert!(German.accepts_output(" für Ihre Unterstützung"));
    assert!(Persian.accepts_output(" بررسی کنید"));
    assert!(Persian.accepts_output(" می\u{200c}شود"));
    for language in [English, German] {
        assert!(!language.accepts_output(" بررسی کنید"));
    }
    for output in [
        " next step",
        " 世界",
        " 👍",
        "\u{202e}بررسی",
        " می\u{200c} شود",
    ] {
        assert!(!Persian.accepts_output(output));
    }
    assert_eq!(
        complete_word_prefix(" بررسی کنید", 4, true).as_deref(),
        Some(" بررسی کنید")
    );
    assert_eq!(complete_word_prefix(" \u{202e}بررسی", 4, true), None);
}

#[test]
fn an_installed_filename_does_not_bypass_model_verification() {
    let directory = tempfile::tempdir().expect("model directory");
    let model = crate::model_selection::catalog(crate::model_selection::ModelUseCase::Writing)[1];
    std::fs::create_dir(directory.path().join("models")).expect("models");
    std::fs::create_dir_all(directory.path().join("runtimes/llama-b10726")).expect("runtime");
    std::fs::write(
        directory.path().join("models").join(model.filename),
        b"corrupt",
    )
    .expect("invalid weights");
    std::fs::write(
        directory.path().join("runtimes/llama-b10726/llama-server"),
        b"unused",
    )
    .expect("unused runtime");
    assert!(matches!(
        super::verified_launch(directory.path(), model, 4),
        Err(super::WritingError::Provenance(
            crate::semantic::provenance::ProvenanceError::SizeMismatch
        ))
    ));
}

#[test]
fn spelling_proposals_are_conservative_and_leave_names_alone() {
    for (before, after) in [
        ("teh", "the"),
        ("recieve", "receive"),
        ("adress", "address"),
        ("definately", "definitely"),
    ] {
        assert!(valid_correction(before, after));
    }
    for (before, after) in [
        ("omarchy", "monarchy"),
        ("their", "their"),
        ("cat", "elephant"),
        ("color", "colon"),
        ("badi", "badix"),
    ] {
        assert!(!valid_correction(before, after));
    }
    assert_eq!(correction_word("I use Badi"), None);
    assert_eq!(correction_word("Please find attached the"), None);
    assert_eq!(correction_word("This is teh "), None);
    assert_eq!(correction_word("This is teh"), Some("teh"));
    for before in [
        "Please review this project",
        "Please check the documents",
        "Use the latest",
        "I use Obsidian",
    ] {
        assert_eq!(correction_word(before), None, "{before}");
    }
}

#[test]
fn dictionary_spelling_returns_only_unambiguous_single_edits() {
    for (original, corrected) in [
        ("adress", "address"),
        ("definately", "definitely"),
        ("exampel", "example"),
        ("langauge", "language"),
        ("seperate", "separate"),
        ("occured", "occurred"),
        ("acheive", "achieve"),
    ] {
        assert_eq!(
            super::unambiguous_correction(original).as_deref(),
            Some(corrected),
            "{original}"
        );
    }
    // Frequency and context are not present in a word set. Never choose
    // the first alphabetic candidate for an ambiguous misspelling.
    for original in [
        "teh", "recieve", "wierd", "realy", "Badi", "omarchy", "badi", "color", "thier",
        "tommorow", "ab", "naïve", "миp", "the",
    ] {
        assert_eq!(super::unambiguous_correction(original), None, "{original}");
    }
    for partial in ["expl", "docum", "autom", "attache", "programmi"] {
        assert_eq!(correction_word(partial), None, "{partial}");
        assert_eq!(super::unambiguous_correction(partial), None, "{partial}");
    }
}

#[test]
fn spelling_target_owns_only_the_word_and_a_single_typed_space() {
    for (before, suffix, delimiter) in [
        ("This is teh", "teh", ""),
        ("This is teh ", "teh ", " "),
        ("teh ", "teh ", " "),
    ] {
        let target = super::correction_target(before).expect("correction target");
        assert_eq!(target.word, "teh");
        assert_eq!(target.suffix, suffix);
        assert_eq!(target.delimiter, delimiter);
    }
    for before in [
        "This is teh  ",
        "This is teh\t",
        "This is teh\n",
        "This is teh\u{00a0}",
        "This is teh.",
        "This is the ",
        "This is Teh ",
        "This is docum ",
    ] {
        assert!(super::correction_target(before).is_none(), "{before:?}");
    }
}

#[test]
fn streaming_never_returns_a_partial_last_word() {
    for apostrophe in ['\'', '\u{2019}'] {
        assert_eq!(
            super::available_complete_words(&format!(" don{apostrophe}")),
            None
        );
        assert_eq!(
            complete_word_prefix(&format!(" one two three don{apostrophe}"), 4, false),
            None
        );
        assert_eq!(
            super::available_complete_words(&format!(" one two three don{apostrophe}")),
            Some(" one two three".to_owned())
        );
        assert_eq!(
            complete_word_prefix(&format!(" don{apostrophe}t "), 1, false),
            Some(format!(" don{apostrophe}t"))
        );
    }
    assert_eq!(
        complete_word_prefix(" the updated project rep", 4, false),
        None
    );
    assert_eq!(
        complete_word_prefix(" the updated project report for", 4, false).as_deref(),
        Some(" the updated project report")
    );
    assert_eq!(
        complete_word_prefix(" tomorrow.", 4, true).as_deref(),
        Some(" tomorrow.")
    );
    assert_eq!(complete_word_prefix(" the next step", 4, false), None);
    assert_eq!(
        complete_word_prefix("<think>private reasoning", 4, true),
        None
    );
}

/// UAX #29 splits `re-try` into two words, so a limit counting its words
/// would cut ` be able to re-try the request` to `be able to re`. The
/// limit counts space-separated words.
#[test]
fn the_word_limit_never_cuts_inside_a_joined_word() {
    for (raw, finished, expected) in [
        ("be able to re-try the", false, Some("be able to re-try")),
        (" be able to re-try the", false, Some(" be able to re-try")),
        (
            "be able to re-try the request",
            true,
            Some("be able to re-try"),
        ),
        ("be able to re-try", false, None),
        (" a state-of-the-art tool is", false, None),
        (
            " a state-of-the-art tool is here",
            false,
            Some(" a state-of-the-art tool is"),
        ),
        (" die E-Mail an Herrn", false, None),
        (
            " die E-Mail an Herrn Meyer",
            false,
            Some(" die E-Mail an Herrn"),
        ),
        (" را به‌روز کنیم و", false, None),
        (" را به‌روز کنیم و بعد", false, Some(" را به‌روز کنیم و")),
        // Trailing punctuation of an intermediate word is still dropped.
        (
            " we agreed, then left it",
            false,
            Some(" we agreed, then left"),
        ),
        (
            " one two three four, five",
            false,
            Some(" one two three four"),
        ),
        (
            " one - two three four five",
            false,
            Some(" one - two three four"),
        ),
        (" don't re-send it now", false, None),
        (
            " don't re-send it now ",
            false,
            Some(" don't re-send it now"),
        ),
    ] {
        assert_eq!(
            complete_word_prefix(raw, 4, finished).as_deref(),
            expected,
            "{raw:?}"
        );
    }
    // Deadline salvage counts the same words.
    for (raw, expected) in [
        ("be able to re-try the", Some("be able to re-try")),
        (" a b c re-try d", Some(" a b c re-try")),
        (" re-", None),
        (" able to re-", Some(" able to")),
    ] {
        assert_eq!(
            super::available_complete_words(raw).as_deref(),
            expected,
            "{raw:?}"
        );
    }
}

#[test]
fn window_words_bias_only_english_topic_words_not_typed_just_now() {
    let window =
        "The data migration moves invoices to the warehouse on Thursday, which is after the audit.";
    let words = super::bias_words("Can we finish the migration", window, Some("en"));
    assert!(
        words.contains(&"invoices") && words.contains(&"warehouse") && words.contains(&"Thursday")
    );
    assert!(
        !words.contains(&"migration"),
        "a word just typed invites a repeat"
    );
    assert!(
        !words.contains(&"which") && !words.contains(&"after"),
        "function words carry no topic"
    );
    assert!(
        !words.contains(&"data"),
        "short words start too many others"
    );
    assert_eq!(
        super::bias_words("سلام", "کتابخانه موزه", Some("fa")),
        Vec::<&str>::new()
    );
    assert_eq!(
        super::bias_words("Hallo", "Lagerhaus Rechnungen", Some("de")),
        Vec::<&str>::new()
    );
    assert_eq!(
        super::bias_words("Hello there", "", Some("en")),
        Vec::<&str>::new()
    );
}
