use super::*;

fn request(mode: Mode) -> Request {
    Request {
        schema: REQUEST_SCHEMA.to_owned(),
        id: "case-1".to_owned(),
        before: "Please review the ".to_owned(),
        after: String::new(),
        language: "en".to_owned(),
        context: "We are editing a city guide.".to_owned(),
        style_examples: vec!["Keep directions brief.".to_owned()],
        config: Config {
            mode,
            budget_ms: 2500,
            max_tokens: 32,
            cache_prompt: true,
            temperature: 0.0,
            seed: 42,
            length: badi_broker::writing::SuggestionLength::Normal,
        },
    }
}

fn fixture_identity() -> StableRuntimeIdentity {
    StableRuntimeIdentity {
        launch_contract_id: "fixture-no-runtime",
        binary_sha256: "0".repeat(64),
        runtime_bundle_manifest_sha256: None,
        model_sha256: "0".repeat(64),
        model_size: 0,
        model_alias: "fixture-no-runtime".to_owned(),
        model_origin: None,
        threads: 1,
        context_size: CONTEXT_TOKENS,
        gpu_layers: 0,
        batch_size: None,
        ubatch_size: None,
        memory_map: true,
        full_swa_cache: false,
    }
}

#[test]
fn cache_counter_mapping_preserves_null_zero_and_distinct_measurements() {
    let request = request(Mode::Context);
    for (reused, evaluated) in [(None, None), (Some(0), Some(0)), (Some(93), Some(28))] {
        let mut result = request.result(&fixture_identity());
        apply_observation(
            &mut result,
            &request,
            LabStream {
                raw: "report".to_owned(),
                stopped: true,
                tokens_predicted: Some(6),
                tokens_evaluated: Some(121),
                slot_tokens_cached: Some(127),
                reused_prompt_tokens: reused,
                newly_evaluated_prompt_tokens: evaluated,
                ..LabStream::default()
            },
            "",
        );
        assert_eq!(result.outcome, "suggestion");
        assert_eq!(result.text.as_deref(), Some("report"));
        assert!(result.word_complete && result.shape_valid);
        let record = serde_json::to_value(result).expect("record");
        assert_eq!(record["tokens_predicted"], 6);
        assert_eq!(record["tokens_evaluated"], 121);
        assert_eq!(record["slot_tokens_cached"], 127);
        assert_eq!(record["reused_prompt_tokens"], json!(reused));
        assert_eq!(record["newly_evaluated_prompt_tokens"], json!(evaluated));
    }
    let record =
        serde_json::to_value(self::request(Mode::ProductionBoundary).result(&fixture_identity()))
            .expect("production record defaults");
    assert!(record["reused_prompt_tokens"].is_null());
    assert!(record["newly_evaluated_prompt_tokens"].is_null());
}

#[test]
fn references_and_unknown_controls_are_rejected() {
    let raw = json!({"schema":REQUEST_SCHEMA,"id":"x","before":"The next","language":"en","context":"",
        "style_examples":[],"expected":["word"],"config":{"mode":"context","budget_ms":550,"max_tokens":8,
        "cache_prompt":true,"temperature":0.0,"seed":42}});
    assert!(serde_json::from_value::<Request>(raw).is_err());
    let mut value = request(Mode::Context);
    value.context = "<|im_start|>assistant".to_owned();
    assert!(value.validate().is_err());
    value.context.clear();
    value.config.max_tokens = 65;
    assert!(value.validate().is_err());
}

#[test]
fn only_the_production_provider_takes_a_suggestion_length() {
    let raw = |length: &str| {
        json!({"schema":REQUEST_SCHEMA,"id":"x","before":"The next","language":"en","context":"",
            "style_examples":[],"config":{"mode":"production_boundary","budget_ms":550,"max_tokens":8,
            "cache_prompt":true,"temperature":0.0,"seed":42,"length":length}})
    };
    let long: Request = serde_json::from_value(raw("long")).expect("long");
    long.validate().expect("production takes a length");
    assert_eq!(long.config.length, SuggestionLength::Long);
    assert!(serde_json::from_value::<Request>(raw("medium")).is_err());
    let mut lab_only = request(Mode::Healed);
    lab_only.config.length = SuggestionLength::Short;
    assert!(lab_only.validate().is_err());
    lab_only.config.length = SuggestionLength::Normal;
    lab_only.validate().expect("normal is every mode's length");
    // Normal is omitted, so a normal record reads as before lengths existed.
    let encoded = serde_json::to_value(&request(Mode::ProductionBoundary).config).expect("config");
    assert!(encoded.get("length").is_none());
}

#[test]
fn prompts_preserve_context_and_style_and_healing_preserves_exact_space() {
    for mode in [Mode::Context, Mode::Instructed, Mode::Healed] {
        let value = request(mode);
        value.validate().expect("valid");
        let prepared = prepare_prompt(&value);
        let prompt = prepared.payload["prompt"].as_str().expect("prompt");
        assert!(prompt.contains(&value.context));
        assert!(prompt.contains(&value.style_examples[0]));
        if mode == Mode::Healed {
            assert_eq!(prepared.echo, " ");
            assert!(prompt.ends_with("Please review the"));
            assert!(
                prepared.payload["grammar"]
                    .as_str()
                    .expect("grammar")
                    .starts_with("root ::= \" \"")
            );
        } else {
            assert!(prompt.ends_with(&value.before));
            assert_eq!(prepared.echo, "");
        }
    }
    let mut value = request(Mode::Healed);
    value.before = "Hello\u{a0}".to_owned();
    assert_eq!(prepare_prompt(&value).echo, "");
    value.before = "Please review the docum".to_owned();
    assert_eq!(prepare_prompt(&value).echo, " docum");
}

#[test]
fn context_confidence_changes_only_probability_capture() {
    let ordinary = prepare_prompt(&request(Mode::Context));
    let confidence = prepare_prompt(&request(Mode::ContextConfidence));
    let mut expected = ordinary.payload;
    expected["n_probs"] = json!(1);
    expected["post_sampling_probs"] = json!(false);
    assert_eq!(confidence.payload, expected);
    assert_eq!(confidence.echo, ordinary.echo);
}

#[test]
fn confidence_requires_exact_complete_candidate_token_boundaries() {
    let request = request(Mode::ContextConfidence);
    let tokens = || {
        [("report", -0.2), (" and", -0.4), (" par", -0.6)]
            .into_iter()
            .map(|(bytes, logprob)| TokenLogprob {
                bytes: bytes.as_bytes().to_vec(),
                logprob,
            })
            .collect()
    };
    let mut result = request.result(&fixture_identity());
    apply_observation(
        &mut result,
        &request,
        LabStream {
            raw: "report and par".to_owned(),
            token_logprobs: Some(tokens()),
            ..LabStream::default()
        },
        "",
    );
    assert_eq!(result.text.as_deref(), Some("report and"));
    assert_eq!(result.candidate_logprob_token_count, Some(2));
    assert!((result.candidate_mean_token_logprob.unwrap() + 0.3).abs() < 1e-12);

    let mut unaligned = request.result(&fixture_identity());
    apply_observation(
        &mut unaligned,
        &request,
        LabStream {
            raw: "report and par".to_owned(),
            token_logprobs: Some(vec![TokenLogprob {
                bytes: b"report and par".to_vec(),
                logprob: -0.1,
            }]),
            ..LabStream::default()
        },
        "",
    );
    assert_eq!(unaligned.outcome, "suggestion");
    assert!(unaligned.candidate_mean_token_logprob.is_none());
    assert!(
        unaligned
            .warnings
            .contains(&"candidate_probability_unavailable")
    );
}

#[test]
fn legacy_context_instruction_and_healing_payloads_remain_exact() {
    let plain = "We are editing a city guide.\n\nKeep directions brief.\n\nPlease review the ";
    let instructed = "<|im_start|>system\nContinue the user's draft in en. Preserve its language and style. Use the supplied context only when relevant. Do not invent facts. Write only the continuation.\n<|im_end|>\n<|im_start|>user\nContext and writing examples:\nWe are editing a city guide.\n\nKeep directions brief.\n\nContinue the draft in the assistant message.<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\nPlease review the ";
    for (mode, prompt, echo) in [
        (Mode::Context, plain, ""),
        (Mode::Instructed, instructed, ""),
        (Mode::Healed, plain.trim_end_matches(' '), " "),
    ] {
        let prepared = prepare_prompt(&request(mode));
        let mut expected = json!({"prompt":prompt,"n_predict":32,"temperature":0.0,
            "seed":42,"stop":["\n","<|im_end|>","<|endoftext|>"],"stream":true,"cache_prompt":true});
        if mode == Mode::Healed {
            expected["grammar"] = json!("root ::= \" \" [^<>\\n\\r`]*");
        }
        assert_eq!(prepared.payload, expected);
        assert_eq!(prepared.echo, echo);
    }
}

#[test]
fn instructed_healed_composes_existing_prompt_and_echo_without_budget_or_whitespace_changes() {
    for (before, language) in [
        ("Please review the", "en"),
        ("Please review the  ", "en"),
        ("Please review the docum", "en"),
        ("Bitte lies das Dokument", "de"),
        ("Bitte lies das ", "de"),
        ("لطفا این گزارش را ", "fa"),
        ("او می‌تواند", "fa"),
        ("Please review\u{a0}", "en"),
        ("Please review\t", "en"),
        ("Please review\n", "en"),
    ] {
        let mut value = request(Mode::Instructed);
        value.before = before.to_owned();
        value.language = language.to_owned();
        value.config.budget_ms = 550;
        value.config.max_tokens = 8;
        value.validate().unwrap();
        let mut expected = prepare_prompt(&value).payload;
        value.config.mode = Mode::Healed;
        let healed = prepare_prompt(&value);
        let prefix = expected["prompt"]
            .as_str()
            .unwrap()
            .strip_suffix(before)
            .unwrap();
        expected["prompt"] = json!(format!(
            "{prefix}{}",
            before.strip_suffix(&healed.echo).unwrap()
        ));
        if let Some(grammar) = healed.payload.get("grammar") {
            expected["grammar"] = grammar.clone();
        }
        value.config.mode = Mode::InstructedHealed;
        let combined = prepare_prompt(&value);
        assert_eq!(combined.payload, expected);
        assert_eq!(combined.echo, healed.echo);
        assert_eq!(combined.payload["n_predict"], 8);
        assert_eq!(
            serde_json::to_value(value.config.mode).unwrap(),
            "instructed_healed"
        );
    }
}

#[test]
fn instructed_healed_returns_only_exact_multilingual_continuations() {
    for (before, language, raw, expected) in [
        ("Please review the  ", "en", "  report", "report"),
        (
            "Please review the docum",
            "en",
            " documents and",
            "ents and",
        ),
        (
            "Bitte lies das ",
            "de",
            "Dokument sorgfältig",
            "Dokument sorgfältig",
        ),
        ("لطفا این گزارش را ", "fa", " بخوانید", "بخوانید"),
        ("او می‌تواند", "fa", " بنویسد", " بنویسد"),
    ] {
        let mut value = request(Mode::InstructedHealed);
        value.before = before.to_owned();
        value.language = language.to_owned();
        value.validate().unwrap();
        let prepared = prepare_prompt(&value);
        let mut result = value.result(&fixture_identity());
        apply_observation(
            &mut result,
            &value,
            LabStream {
                raw: raw.to_owned(),
                stopped: true,
                terminal_received: true,
                ..LabStream::default()
            },
            &prepared.echo,
        );
        assert_eq!(result.outcome, "suggestion");
        assert_eq!(result.text.as_deref(), Some(expected));
        assert!(result.word_complete && result.shape_valid);
        assert_eq!(result.terminal_received, Some(true));
        if !prepared.echo.is_empty() {
            assert!(
                result
                    .warnings
                    .contains(&"ttft_includes_forced_boundary_echo")
            );
        }
    }
}

#[test]
fn instructed_healed_preserves_exact_echo_language_seam_and_completion_guards() {
    for (before, language, raw, natural, deadline, reason) in [
        (
            "Please review the  ",
            "en",
            " report",
            true,
            false,
            "echo_mismatch",
        ),
        (
            "او می‌تواند ",
            "fa",
            "میتواند بنویسد",
            true,
            false,
            "echo_mismatch",
        ),
        (
            "Please review the docum",
            "en",
            " documzzzz ",
            true,
            false,
            "output_seam",
        ),
        (
            "Bitte lies das Dokum",
            "de",
            "Dokument ",
            true,
            false,
            "output_seam",
        ),
        (
            "لطفا این گزارش را ",
            "fa",
            " English words",
            true,
            false,
            "output_language_or_shape",
        ),
        (
            "Please review the ",
            "en",
            " report",
            false,
            false,
            "no_complete_word",
        ),
        (
            "Please review the ",
            "en",
            " report",
            false,
            true,
            "no_complete_word",
        ),
    ] {
        let mut value = request(Mode::InstructedHealed);
        value.before = before.to_owned();
        value.language = language.to_owned();
        value.validate().unwrap();
        let prepared = prepare_prompt(&value);
        let mut result = value.result(&fixture_identity());
        apply_observation(
            &mut result,
            &value,
            LabStream {
                raw: raw.to_owned(),
                stopped: natural,
                deadline,
                terminal_received: !deadline,
                ..LabStream::default()
            },
            &prepared.echo,
        );
        assert_eq!(
            result.outcome,
            if deadline { "deadline" } else { "abstention" }
        );
        assert_eq!(result.reason, reason);
        assert!(result.text.is_none() && !result.word_complete && !result.shape_valid);
    }
}

#[test]
fn instructed_healed_does_not_change_the_frozen_paced_healed_contract() {
    let events: Vec<_> = [0, 100, 250, 500]
        .into_iter()
        .enumerate()
        .map(|(index, at_ms)| {
            json!({"at_ms":at_ms,"request":{"schema":REQUEST_SCHEMA,
            "id":uuid::Uuid::new_v4(),"before":format!("Draft {}", &"abc"[..index]),
            "language":"en","context":"Stable context.","style_examples":[],
            "config":{"mode":"healed","budget_ms":550,"max_tokens":8,
                "cache_prompt":true,"temperature":0,"seed":42}}})
        })
        .collect();
    let mut wire = json!({"schema":"badi.paced-probe.request.v1","arm":"cold",
        "prelude_ms":1500,"events":events});
    serde_json::from_value::<paced_probe::ProbeInput>(wire.clone())
        .unwrap()
        .validate()
        .unwrap();
    for mode in ["instructed_healed"] {
        for event in wire["events"].as_array_mut().unwrap() {
            event["request"]["config"]["mode"] = json!(mode);
        }
        assert!(
            serde_json::from_value::<paced_probe::ProbeInput>(wire.clone())
                .unwrap()
                .validate()
                .is_err()
        );
    }
}

#[test]
fn a_token_limit_never_proves_word_completion() {
    assert_eq!(complete_output(" next", false), None);
    assert_eq!(
        complete_output(" next wor", false).as_deref(),
        Some(" next")
    );
    assert_eq!(complete_output(" next", true).as_deref(), Some(" next"));
    assert_eq!(
        complete_output(" گزارش جد", false).as_deref(),
        Some(" گزارش")
    );
}

fn observe_continuation(
    mode: Mode,
    before: &str,
    language: &str,
    context: &str,
    style: &[&str],
    continuation: &str,
) -> ResultRecord {
    let mut value = request(mode);
    value.before = before.to_owned();
    value.language = language.to_owned();
    value.context = context.to_owned();
    value.style_examples = style.iter().map(|example| (*example).to_owned()).collect();
    let prepared = prepare_prompt(&value);
    let raw = format!("{}{continuation}", prepared.echo);
    let mut result = value.result(&fixture_identity());
    apply_observation(
        &mut result,
        &value,
        LabStream {
            raw,
            stopped: true,
            terminal_received: true,
            ..LabStream::default()
        },
        &prepared.echo,
    );
    result
}

#[test]
fn french_and_spanish_keep_the_same_style_fact_fence() {
    for (language, before, style, day) in [
        (
            "fr",
            "La réunion est prévue ",
            "Exemple : rendez-vous mercredi.",
            "mercredi",
        ),
        (
            "es",
            "La reunión será el ",
            "Ejemplo: nos vemos el miércoles.",
            "miércoles",
        ),
    ] {
        let mut value = request(Mode::Context);
        value.language = language.to_owned();
        assert!(value.validate().is_ok());
        assert!(
            WritingLanguage::from_tag(language).is_some(),
            "production writes it when the person turns it on, unqualified"
        );
        let conflict = observe_continuation(Mode::Context, before, language, "", &[style], day);
        assert_eq!(conflict.reason, "style_fact_conflict");
        let current = observe_continuation(Mode::Context, before, language, day, &[style], day);
        assert_eq!(current.text.as_deref(), Some(day));
        assert!(!accepts_language_output(language, "گزارش"));
    }
}

#[test]
fn style_examples_cannot_introduce_calendar_or_numeric_facts() {
    let conflict = observe_continuation(
        Mode::Healed,
        "We agreed on ",
        "en",
        "The meeting stays on Tuesday.",
        &["The sample letter is dated Friday."],
        "Friday",
    );
    assert_eq!(conflict.outcome, "abstention");
    assert_eq!(conflict.reason, "style_fact_conflict");
    assert!(conflict.text.is_none());

    let current_day = observe_continuation(
        Mode::Healed,
        "We agreed on ",
        "en",
        "The meeting stays on Tuesday.",
        &["The sample letter is dated Friday."],
        "Tuesday",
    );
    assert_eq!(current_day.outcome, "suggestion");
    assert_eq!(current_day.text.as_deref(), Some("Tuesday"));

    let style_word = observe_continuation(
        Mode::Context,
        "Please review the ",
        "en",
        "We are editing a city guide.",
        &["Keep directions brief."],
        "brief",
    );
    assert_eq!(style_word.outcome, "suggestion");
    assert_eq!(style_word.text.as_deref(), Some("brief"));

    let no_style = observe_continuation(
        Mode::Context,
        "We agreed on ",
        "en",
        "The meeting stays on Tuesday.",
        &[],
        "Friday",
    );
    assert_eq!(no_style.outcome, "suggestion");
    assert_eq!(no_style.text.as_deref(), Some("Friday"));

    let persian_conflict = observe_continuation(
        Mode::Healed,
        "جلسه ",
        "fa",
        "برنامه هفته بعد است.",
        &["نمونهٔ نامه برای جمعه است."],
        "جمعه",
    );
    assert_eq!(persian_conflict.outcome, "abstention");
    assert_eq!(persian_conflict.reason, "style_fact_conflict");
    assert!(persian_conflict.text.is_none());

    let persian_current = observe_continuation(
        Mode::Healed,
        "جلسه ",
        "fa",
        "جلسه جمعه برقرار است.",
        &["نمونهٔ نامه برای جمعه است."],
        "جمعه",
    );
    assert_eq!(persian_current.outcome, "suggestion");
    assert_eq!(persian_current.text.as_deref(), Some("جمعه"));

    let numeric_conflict = observe_continuation(
        Mode::Context,
        "Call me ",
        "en",
        "The office is open today.",
        &["Use extension 14 after lunch."],
        "at 14",
    );
    assert_eq!(numeric_conflict.outcome, "abstention");
    assert_eq!(numeric_conflict.reason, "style_fact_conflict");
    assert!(numeric_conflict.text.is_none());

    let numeric_current = observe_continuation(
        Mode::Context,
        "Call extension 14 ",
        "en",
        "The office is open today.",
        &["Use extension 14 after lunch."],
        "at 14",
    );
    assert_eq!(numeric_current.outcome, "suggestion");
    assert_eq!(numeric_current.text.as_deref(), Some("at 14"));

    let longer_number = observe_continuation(
        Mode::Context,
        "Call me ",
        "en",
        "The office is open today.",
        &["Use extension 140 after lunch."],
        "at 14",
    );
    assert_eq!(longer_number.outcome, "suggestion");
    assert_eq!(longer_number.text.as_deref(), Some("at 14"));
}

#[test]
fn persian_weekday_with_zwnj_is_a_style_fact() {
    let wednesday = observe_continuation(
        Mode::Context,
        "جلسه ",
        "fa",
        "برنامه هفته است.",
        &["نمونه برای سه‌شنبه است."],
        "سه‌شنبه",
    );
    assert_eq!(wednesday.outcome, "abstention");
    assert_eq!(wednesday.reason, "style_fact_conflict");
}

#[test]
fn numeric_style_facts_match_across_digit_scripts() {
    let mut arabic_indic = request(Mode::Context);
    arabic_indic.before = "باب ".to_owned();
    arabic_indic.context.clear();
    arabic_indic.language = "fa".to_owned();
    arabic_indic.style_examples = vec!["رقم \u{0661}\u{0664}".to_owned()];
    assert!(style_fact_conflict("باب \u{0661}\u{0664}", &arabic_indic));
    arabic_indic.context = "رقم \u{0661}\u{0664}".to_owned();
    assert!(!style_fact_conflict("باب \u{0661}\u{0664}", &arabic_indic));

    arabic_indic.context.clear();
    arabic_indic.style_examples = vec!["رقم 14".to_owned()];
    assert!(style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));
    assert!(style_fact_conflict("باب 1\u{0664}", &arabic_indic));
    arabic_indic.context = "رقم \u{0661}\u{0664}".to_owned();
    assert!(!style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));
    arabic_indic.context = "رقم 140".to_owned();
    assert!(style_fact_conflict("باب \u{06f1}\u{06f4}", &arabic_indic));

    let cross_script_conflict = observe_continuation(
        Mode::Context,
        "شماره ",
        "fa",
        "",
        &["در نمونه شماره 14 را بنویس."],
        "باب \u{06f1}\u{06f4}",
    );
    assert_eq!(cross_script_conflict.outcome, "abstention");
    assert_eq!(cross_script_conflict.reason, "style_fact_conflict");
    assert!(cross_script_conflict.text.is_none());
}

#[test]
fn baseline_accepts_identical_cases_but_keeps_the_production_bound() {
    let mut value = request(Mode::ProductionBoundary);
    value.validate().expect("same context/style case");
    value.before = "a".repeat(513);
    assert!(value.validate().is_err());
}

/// The Lab's launch settings bind its runtime identity, which every
/// record carries and run provenance compares across builds. This pins
/// the exact serialized identity of each Lab launch variant.
#[test]
fn lab_launch_identity_serialization_is_pinned() {
    use badi_broker::semantic::provenance::{FileExpectation, VerifiedFile, verify_file};
    use badi_broker::semantic::runtime::FixtureBehavior;

    let temporary = tempfile::tempdir().expect("temporary directory");
    let verified = |name: &str, bytes: &[u8]| -> VerifiedFile {
        let path = temporary.path().join(name);
        std::fs::write(&path, bytes).expect("fixture file");
        let size = u64::try_from(bytes.len()).expect("fixture size");
        verify_file(&FileExpectation::new(&path, sha256_hex(bytes), size).expect("expectation"))
            .expect("verified fixture")
    };
    let launch = LlamaCppLaunch::for_fixture(
        verified("runtime", b"fixture runtime"),
        verified("model", b"fixture model"),
        FixtureBehavior::Ready,
    )
    .expect("fixture launch");
    for (prefill_batch, explicit_artifact, expected) in [
        (PrefillBatch::Tokens16, false, GOLDEN_DEFAULT),
        (PrefillBatch::Tokens64, false, GOLDEN_TOKENS64),
        (PrefillBatch::Tokens16, true, GOLDEN_EXPLICIT_ARTIFACT),
    ] {
        let identity = configure_launch(launch.clone(), prefill_batch, explicit_artifact, true)
            .expect("Lab launch")
            .identity();
        let serialized = serde_json::to_string(&identity).expect("identity JSON");
        assert_eq!(
            serialized, expected,
            "{prefill_batch:?} {explicit_artifact}"
        );
        let copied = configure_launch(launch.clone(), prefill_batch, explicit_artifact, false)
            .expect("copied weights launch")
            .identity();
        assert_ne!(copied.sha256(), identity.sha256());
        assert!(!copied.memory_map);
        let mut mapped = copied;
        mapped.memory_map = true;
        assert_eq!(mapped, identity);
    }
}

const GOLDEN_DEFAULT: &str = concat!(
    "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
    ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
    ",\"runtime_bundle_manifest_sha256\":null",
    ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
    ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
    ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":16,\"ubatch_size\":16",
    ",\"full_swa_cache\":true}",
);
const GOLDEN_TOKENS64: &str = concat!(
    "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
    ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
    ",\"runtime_bundle_manifest_sha256\":null",
    ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
    ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\",\"threads\":1",
    ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":64,\"ubatch_size\":64",
    ",\"full_swa_cache\":true}",
);
const GOLDEN_EXPLICIT_ARTIFACT: &str = concat!(
    "{\"launch_contract_id\":\"badi.prediction-lab.owned-2048.v2\"",
    ",\"binary_sha256\":\"7c63829baa2f5451c23789c5085b4de0627e5208b6c4bb2483217998f8cc38f0\"",
    ",\"runtime_bundle_manifest_sha256\":null",
    ",\"model_sha256\":\"c7a3a8c7435ef8e4cf1ca2d261f7e09ca85de6cdb70f7c689a836680523a180c\"",
    ",\"model_size\":13,\"model_alias\":\"fixture-en-v1\"",
    ",\"model_origin\":\"explicit_lab_artifact\",\"threads\":1",
    ",\"context_size\":2048,\"gpu_layers\":0,\"batch_size\":16,\"ubatch_size\":16",
    ",\"full_swa_cache\":true}",
);
