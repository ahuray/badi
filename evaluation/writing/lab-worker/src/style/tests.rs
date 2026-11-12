use super::*;

#[test]
fn strengths_select_recent_examples_in_order_without_sharing_cases() {
    let mut request = crate::tests::request(Mode::StyleGentle);
    request.style_examples = ["Oldest.", "Earlier.", "Recent.", "Newest."]
        .map(str::to_owned)
        .to_vec();
    for (mode, expected) in [
        (Mode::StyleGentle, 1),
        (Mode::StyleBalanced, 2),
        (Mode::StyleStrong, 4),
    ] {
        request.config.mode = mode;
        let selected = selected_examples(&request).unwrap();
        assert_eq!(selected.len(), expected);
        assert!(prefix(&selected).ends_with("Newest.\n\n"));
    }
    // A different subject/language is another explicit case, with no history
    // lookup or mutable sample cache from the earlier request.
    let mut other = crate::tests::request(Mode::StyleStrong);
    other.language = "de".to_owned();
    other.style_examples.clear();
    assert_eq!(prefix(&selected_examples(&other).unwrap()), "");
    assert!(selected_examples(&crate::tests::request(Mode::ProductionBoundary)).is_none());
}

#[test]
fn style_fact_fence_checks_names_numbers_weekdays_and_the_caret_seam() {
    let mut request = crate::tests::request(Mode::StyleStrong);
    request.before = "We will meet on ".into();
    let samples = vec!["Meet Alice on Friday with ۴۲ copies.".into()];
    for candidate in ["Friday", "Alice", "42 copies"] {
        assert!(fact_conflict(candidate, &request, &samples), "{candidate}");
    }
    request.context = "Alice on Friday with 42 copies".into();
    assert!(fact_conflict("Alice", &request, &samples));
    request.before = "We will meet on Fri".into();
    assert!(fact_conflict("day", &request, &samples));
    request.before = "Alice has 42 copies. We meet on Friday. Ask ".into();
    assert!(!fact_conflict("Alice", &request, &samples));
    assert!(!fact_conflict("about the copies", &request, &samples));
}

#[test]
fn possessive_names_share_a_fact_key_in_samples_draft_and_additions() {
    let mut request = crate::tests::request(Mode::StyleGentle);
    let names = ["Alice", "Alice's", "Alice’s"];
    for sample_name in names {
        let samples = vec![format!("Please ask {sample_name} about the proposal.")];
        request.before = "Please send me".into();
        for candidate_name in names {
            let candidate = format!(" {candidate_name} proposal.");
            assert!(fact_conflict(&candidate, &request, &samples));
        }
        request.before = "Please send me Ali".into();
        for candidate in ["ce", "ce's proposal.", "ce’s proposal."] {
            assert!(fact_conflict(candidate, &request, &samples));
        }
        for draft_name in names {
            request.before = format!("{draft_name} proposal is ready. Please send me");
            for candidate_name in names {
                let candidate = format!(" {candidate_name} proposal.");
                assert!(!fact_conflict(&candidate, &request, &samples));
            }
        }
        request.before = "Please send me".into();
        request.after = " Alice’s proposal is ready.".into();
        assert!(!fact_conflict(" Alice's proposal.", &request, &samples));
        request.after.clear();
    }
}

#[test]
fn portuguese_style_cannot_supply_a_weekday_even_across_the_seam() {
    let mut request = crate::tests::request(Mode::StyleGentle);
    request.language = "pt-BR".into();
    let examples = vec!["Alice chega na sexta-feira para a reunião.".into()];
    request.before = "Vamos nos encontrar".into();
    assert!(fact_conflict(" na sexta-feira, às", &request, &examples));
    request.before = "Vamos nos encontrar na sex".into();
    assert!(fact_conflict("ta-feira", &request, &examples));
    request.before = "Na sexta-feira vamos nos encontrar".into();
    assert!(!fact_conflict(" na sexta-feira", &request, &examples));
}

#[test]
fn unselected_samples_cannot_veto_a_suggestion() {
    let mut request = crate::tests::request(Mode::StyleGentle);
    request.before = "We should ask ".into();
    request.style_examples = vec!["Ask Alice on Friday.".into(), "Keep it concise.".into()];
    let selected = selected_examples(&request).unwrap();
    assert!(!fact_conflict("Alice", &request, &selected));
}

#[test]
fn context_budget_reserves_generation_and_one_spare_token() {
    let budget = PromptBudget {
        draft_tokens: 40,
        prompt_tokens: 503,
        generation_tokens: 8,
        context_tokens: 512,
    };
    assert!(budget.fits());
    assert!(
        !PromptBudget {
            prompt_tokens: 504,
            ..budget
        }
        .fits()
    );
}

#[test]
fn style_inputs_keep_the_store_sample_bound_and_production_draft_bound() {
    let mut request = crate::tests::request(Mode::StyleGentle);
    request.style_examples = vec!["a".repeat(240)];
    assert!(request.validate().is_ok());
    request.style_examples[0].push('a');
    assert!(request.validate().is_err());
    request.style_examples.clear();
    request.before = "a".repeat(513);
    assert!(request.validate().is_err());
}

#[test]
fn skipped_spelling_and_overflow_do_not_claim_style_was_used() {
    let request = crate::tests::request(Mode::StyleGentle);
    let examples = selected_examples(&request).unwrap();
    let mut result = request.result(&crate::tests::fixture_identity());
    finish(&mut result, &request, &examples, vec![]);
    assert!(!result.used_style);
    finish(
        &mut result,
        &request,
        &examples,
        vec![PromptBudget {
            draft_tokens: 20,
            prompt_tokens: 510,
            generation_tokens: 8,
            context_tokens: 512,
        }],
    );
    assert_eq!(result.reason, "style_context_overflow");
    assert!(!result.used_style);
    assert_eq!(result.text, None);
}
