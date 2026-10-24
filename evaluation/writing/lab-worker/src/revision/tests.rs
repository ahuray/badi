use super::*;

fn request(before: &str, selected: &str, after: &str) -> RevisionRequest {
    RevisionRequest {
        schema: REQUEST_SCHEMA.to_owned(),
        id: "case-1".to_owned(),
        language: "en".to_owned(),
        mode: RevisionMode::Grammar,
        before: before.to_owned(),
        selected: selected.to_owned(),
        after: after.to_owned(),
    }
}

fn answer(output: &str, truncated: bool) -> RevisionReply {
    RevisionReply {
        output: output.to_owned(),
        truncated,
        cold: false,
        generated_tokens: Some(9),
    }
}

fn english() -> &'static RevisionLanguage {
    badi_broker::revision::language("en").unwrap()
}

#[test]
fn a_request_carries_no_answer_and_keeps_the_protocol_bounds() {
    let frame = serde_json::json!({"schema": REQUEST_SCHEMA, "id": "case-1", "language": "en",
        "mode": "grammar", "selected": "She dont know.", "expected": ["She doesn't know."]});
    assert!(serde_json::from_value::<RevisionRequest>(frame).is_err());

    assert_eq!(
        request("", "She dont know.", "").validate().unwrap().tag,
        "en"
    );
    let mut regional = request("", "Ele vai.", "");
    regional.language = "pt-BR".to_owned();
    assert_eq!(regional.validate().unwrap().tag, "pt");
    let mut persian = request("", "Ele vai.", "");
    persian.language = "fa".to_owned();
    assert!(matches!(
        persian.validate(),
        Err(LabError::UnsupportedLanguage)
    ));
    for invalid in [
        request("", "two\nlines", ""),
        request("", "   ", ""),
        request("", &"a".repeat(MAX_SELECTED_SCALARS + 1), ""),
        request(&"a".repeat(MAX_BEFORE_CHARS + 1), "She dont know.", ""),
        request("", "She dont know.", &"a".repeat(MAX_AFTER_CHARS + 1)),
        request("", "tab\tinside", ""),
    ] {
        assert!(matches!(invalid.validate(), Err(LabError::InvalidRequest)));
    }
    let mut unnamed = request("", "She dont know.", "");
    unnamed.id = String::new();
    assert!(unnamed.validate().is_err());
}

#[test]
fn the_gate_judges_the_answer_with_the_selection_surroundings() {
    let elapsed = Duration::from_millis(1500);
    let alone = request("", "the documents was sent", "");
    let shown = record(
        &alone,
        english(),
        Ok(answer("The documents were sent.", false)),
        elapsed,
    );
    assert_eq!(shown.outcome, "shown");
    assert_eq!(shown.text.as_deref(), Some("The documents were sent."));

    // Mid-sentence and followed by more text: the model answers with the
    // sentence's start it was given, the first letter stays lowercase and
    // the added period goes.
    let inside = request("I told them that ", "the documents was sent", " and left.");
    let shown = record(
        &inside,
        english(),
        Ok(answer("I told them that the documents were sent.", false)),
        elapsed,
    );
    assert_eq!(shown.text.as_deref(), Some("the documents were sent"));
    assert_eq!(
        shown.output.as_deref(),
        Some("I told them that the documents were sent.")
    );
    assert_eq!(shown.latency_ms, 1500.0);
    assert_eq!(shown.generated_tokens, Some(9));
    assert!((shown.changed_word_share.unwrap() - 0.25).abs() < 1e-9);
}

#[test]
fn truncated_answers_and_failed_calls_keep_the_engine_outcome_names() {
    let elapsed = Duration::from_millis(10);
    let case = request("", "She dont know.", "");
    let truncated = record(
        &case,
        english(),
        Ok(answer("She doesn't know.", true)),
        elapsed,
    );
    assert_eq!(truncated.outcome, "truncated");
    assert_eq!(truncated.text, None);
    assert_eq!(truncated.output.as_deref(), Some("She doesn't know."));
    assert!(truncated.truncated);

    let unchanged = record(
        &case,
        english(),
        Ok(answer(" She dont know. ", false)),
        elapsed,
    );
    assert_eq!(unchanged.outcome, "unchanged");
    assert_eq!(unchanged.changed_word_share, Some(0.0));

    for (error, name) in [
        (RevisionError::Timeout, "timeout"),
        (RevisionError::Unavailable, "unavailable"),
        (RevisionError::Failed, "failed"),
    ] {
        let failed = record(&case, english(), Err(error), elapsed);
        assert_eq!(failed.outcome, name);
        assert_eq!((failed.output, failed.changed_word_share), (None, None));
    }
}

/// The development suite's expected corrections are answers the gate shows
/// as they are, and its correct controls come back unchanged: a suite the
/// gate could never score as fixed would measure nothing.
#[test]
fn the_development_suite_is_reachable_through_the_gate() {
    let suite: serde_json::Value = serde_json::from_str(include_str!(
        "../../../lab/examples/revision-development.json"
    ))
    .unwrap();
    let normalized = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut checked = 0;
    for case in suite["cases"].as_array().unwrap() {
        let field = |key: &str| case[key].as_str().unwrap_or_default().to_owned();
        let mut revision = request(&field("before"), &field("selected"), &field("after"));
        revision.language = field("language");
        let language = revision.validate().unwrap();
        let elapsed = Duration::ZERO;
        match field("kind").as_str() {
            "error" => {
                for expected in case["expected"].as_array().unwrap() {
                    let expected = expected.as_str().unwrap();
                    // The model answers with the sentence start it was given.
                    let answered = format!(
                        "{}{expected}",
                        badi_broker::revision::sentence_lead(&revision.before)
                    );
                    let shown = record(&revision, language, Ok(answer(&answered, false)), elapsed);
                    assert_eq!(shown.outcome, "shown", "{}", case["id"]);
                    assert_eq!(normalized(&shown.text.unwrap()), normalized(expected));
                    checked += 1;
                }
            }
            "correct" => {
                let unchanged = format!(
                    "{}{}",
                    badi_broker::revision::sentence_lead(&revision.before),
                    field("selected")
                );
                let same = record(&revision, language, Ok(answer(&unchanged, false)), elapsed);
                assert_eq!(same.outcome, "unchanged", "{}", case["id"]);
                checked += 1;
            }
            _ => {}
        }
    }
    assert!(checked >= 100);
}
