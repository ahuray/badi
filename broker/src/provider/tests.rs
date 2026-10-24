use tokio_util::sync::CancellationToken;

use crate::protocol::MAX_BEFORE_CHARS;

use super::{CompletionProvider, DeterministicPhraseProvider, ProviderRequest};

#[tokio::test]
async fn phrase_provider_is_deterministic() {
    let provider = DeterministicPhraseProvider::default();
    let request = ProviderRequest {
        before: "Thank you".to_owned(),
        after: String::new(),
        language: Some("en".to_owned()),
        vocabulary: None,
    };
    let first = provider
        .complete(request.clone(), CancellationToken::new())
        .await
        .expect("provider result");
    let second = provider
        .complete(request, CancellationToken::new())
        .await
        .expect("provider result");
    assert_eq!(first, second);
    assert_eq!(first.as_deref(), Some(" for your time"));
}

#[tokio::test]
async fn default_phrase_provider_is_silent_without_an_explicit_rule() {
    let result = DeterministicPhraseProvider::default()
        .complete(
            ProviderRequest {
                before: "An unmatched sentence".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
                vocabulary: None,
            },
            CancellationToken::new(),
        )
        .await
        .expect("provider result");

    assert_eq!(result, None);
}

#[tokio::test]
async fn default_phrase_provider_abstains_on_unsafe_near_matches() {
    let provider = DeterministicPhraseProvider::default();
    for request in [
        ProviderRequest {
            before: "displease".to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
            vocabulary: None,
        },
        ProviderRequest {
            before: "I will not say thank you".to_owned(),
            after: String::new(),
            language: Some("en-US".to_owned()),
            vocabulary: None,
        },
        ProviderRequest {
            before: "thank you".to_owned(),
            after: " already written".to_owned(),
            language: Some("en".to_owned()),
            vocabulary: None,
        },
        ProviderRequest {
            before: "thank you".to_owned(),
            after: String::new(),
            language: Some("de".to_owned()),
            vocabulary: None,
        },
        ProviderRequest {
            before: "thank you".to_owned(),
            after: String::new(),
            language: None,
            vocabulary: None,
        },
        ProviderRequest {
            before: "thank you ".to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
            vocabulary: None,
        },
        ProviderRequest {
            before: format!("{}thank you", " ".repeat(MAX_BEFORE_CHARS - 9)),
            after: String::new(),
            language: Some("en".to_owned()),
            vocabulary: None,
        },
    ] {
        let result = provider
            .complete(request, CancellationToken::new())
            .await
            .expect("provider result");
        assert_eq!(result, None);
    }
}

#[tokio::test]
async fn default_phrase_provider_accepts_an_english_locale() {
    let result = DeterministicPhraseProvider::default()
        .complete(
            ProviderRequest {
                before: "Dear reviewer,\n  Looking Forward".to_owned(),
                after: String::new(),
                language: Some("en-GB".to_owned()),
                vocabulary: None,
            },
            CancellationToken::new(),
        )
        .await
        .expect("provider result");

    assert_eq!(result.as_deref(), Some(" to hearing from you"));
}

#[test]
fn explicit_and_spelling_requests_have_longer_writing_budgets() {
    use super::RequestTrigger;
    use std::time::Duration;

    assert_eq!(RequestTrigger::default(), RequestTrigger::Automatic);
    assert_eq!(
        RequestTrigger::from_explicit(false).writing_budget(),
        Duration::from_millis(550)
    );
    assert_eq!(
        RequestTrigger::from_explicit(true).writing_budget(),
        Duration::from_millis(1_200)
    );
    assert_eq!(
        RequestTrigger::Spelling.writing_budget(),
        Duration::from_millis(2_500)
    );
}

#[tokio::test]
async fn phrase_outcomes_separate_request_and_model_abstention() {
    use super::{ProviderOutcome, RequestTrigger};
    use crate::metrics::NoSuggestionReason;

    let provider = DeterministicPhraseProvider::default();
    for (before, after, language, expected) in [
        ("thank you", "", None, NoSuggestionReason::RequestAbstained),
        (
            "thank you",
            "",
            Some("de"),
            NoSuggestionReason::RequestAbstained,
        ),
        (
            "thank you",
            " tail",
            Some("en"),
            NoSuggestionReason::RequestAbstained,
        ),
        (
            "unmatched",
            "",
            Some("en"),
            NoSuggestionReason::ModelAbstained,
        ),
    ] {
        let outcome = provider
            .propose_outcome(
                ProviderRequest {
                    before: before.to_owned(),
                    after: after.to_owned(),
                    language: language.map(str::to_owned),
                    vocabulary: None,
                },
                CancellationToken::new(),
                false,
                RequestTrigger::Automatic,
            )
            .await
            .expect("provider outcome")
            .outcome;
        assert_eq!(outcome, ProviderOutcome::NoSuggestion(expected));
    }
    let outcome = provider
        .propose_outcome(
            ProviderRequest {
                before: "Thank you".to_owned(),
                after: String::new(),
                language: Some("en".to_owned()),
                vocabulary: None,
            },
            CancellationToken::new(),
            false,
            RequestTrigger::Explicit,
        )
        .await
        .expect("provider outcome")
        .outcome;
    assert_eq!(
        outcome
            .into_proposal()
            .map(|proposal| proposal.text)
            .as_deref(),
        Some(" for your time")
    );
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(
        provider
            .propose_outcome(
                ProviderRequest {
                    before: "Thank you".to_owned(),
                    after: String::new(),
                    language: Some("en".to_owned()),
                    vocabulary: None,
                },
                cancelled,
                false,
                RequestTrigger::Automatic,
            )
            .await,
        Err(super::ProviderError::Cancelled)
    ));
}

#[tokio::test]
async fn phrase_fixture_offers_only_negotiated_unambiguous_dictionary_corrections() {
    use super::{ProviderOutcome, RequestTrigger, WritingProposal};
    let provider = DeterministicPhraseProvider::default();
    let propose = async |before: &str, after: &str, language: &str, allow_replacement: bool| {
        provider
            .propose_outcome(
                ProviderRequest {
                    before: before.to_owned(),
                    after: after.to_owned(),
                    language: Some(language.to_owned()),
                    vocabulary: None,
                },
                CancellationToken::new(),
                allow_replacement,
                RequestTrigger::Explicit,
            )
            .await
            .map(|reply| reply.outcome)
    };
    assert_eq!(
        propose("We need an adress ", "", "en-US", true)
            .await
            .unwrap(),
        ProviderOutcome::Proposal(WritingProposal {
            text: "address ".to_owned(),
            replace_before: Some("adress ".to_owned()),
        })
    );
    assert_eq!(
        propose("We need an adress", "", "en", true).await.unwrap(),
        ProviderOutcome::Proposal(WritingProposal {
            text: "address".to_owned(),
            replace_before: Some("adress".to_owned()),
        })
    );
    // No negotiated replacement, text after the caret, another language
    // or an ambiguous word (teh: the/ten/tea...) never yields an edit.
    for (before, after, language, allow) in [
        ("We need an adress ", "", "en", false),
        ("We need an adress", " here", "en", true),
        ("We need an adress", "", "de", true),
        ("This is teh", "", "en", true),
    ] {
        let outcome = propose(before, after, language, allow).await.unwrap();
        assert!(
            outcome
                .into_proposal()
                .is_none_or(|proposal| proposal.replace_before.is_none()),
            "{before:?} {after:?} {language} {allow}"
        );
    }
}
