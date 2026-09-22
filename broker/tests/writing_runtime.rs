#![cfg(feature = "local-model")]

use badi_broker::provider::{CompletionProvider, ProviderRequest};
use tokio_util::sync::CancellationToken;

/// Opt-in real boundary: uses the pinned model/runtime already installed locally.
/// Only synthetic text is emitted; never run against a user's active document.
#[tokio::test]
#[ignore = "requires the pinned local writing model and runtime"]
async fn installed_model_produces_continuations_and_spelling_replacements() {
    let directory = badi_broker::writing::data_directory().expect("model directory");
    let (runtime, model) = badi_broker::writing::activate(directory)
        .await
        .expect("verified runtime");
    eprintln!("model={}", model.filename);
    for (before, expected) in [
        ("This is teh", Some("the")),
        ("This is teh ", Some("the ")),
        ("I will recieve", Some("receive")),
        ("I will recieve ", Some("receive ")),
        ("I use Badi", None),
        ("I use omarchy", None),
        ("The shipping adress", Some("address")),
        ("The shipping adress ", Some("address ")),
        ("I will definately", Some("definitely")),
        ("Please find attached the", None),
        ("We need to review the changes before", None),
    ] {
        let started = std::time::Instant::now();
        let result = runtime
            .propose(
                ProviderRequest {
                    before: before.to_owned(),
                    after: String::new(),
                    language: Some("en".to_owned()),
                },
                CancellationToken::new(),
                true,
            )
            .await
            .expect("inference");
        eprintln!(
            "synthetic_input={before:?} proposal={result:?} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        assert!(
            started.elapsed() < std::time::Duration::from_millis(600),
            "exceeds adapter budget"
        );
        if before.starts_with("I use ") {
            // Names may make the model abstain. They must never become spelling edits.
            assert!(
                result
                    .as_ref()
                    .is_none_or(|proposal| proposal.replace_before.is_none())
            );
            assert!(
                result.as_ref().is_none_or(|proposal| {
                    !proposal
                        .text
                        .chars()
                        .next()
                        .is_some_and(char::is_alphabetic)
                }),
                "unknown names must not be extended into invented words"
            );
            continue;
        }
        let result = result.expect("proposal for the writing/correction case");
        if let Some(expected) = expected {
            assert_eq!(result.text, expected);
            assert!(result.replace_before.is_some());
        } else {
            assert!(result.replace_before.is_none());
            assert!(result.text.split_whitespace().count() <= 4);
        }
    }
}

/// Development probes are deliberately separate from the frozen held-out corpus.
#[tokio::test]
#[ignore = "requires the pinned local writing model and runtime"]
async fn installed_model_completes_partial_words_and_selected_languages() {
    let (runtime, _) = badi_broker::writing::activate(
        badi_broker::writing::data_directory().expect("model directory"),
    )
    .await
    .expect("verified runtime");
    for (before, language) in [
        ("Please review the docum", "en"),
        ("The software should autom", "en"),
        ("Ich freue mich auf die", "de"),
        ("Vielen Dank für Ihre", "de"),
        ("لطفا این گزارش را", "fa"),
        ("برای حل این مشکل باید", "fa"),
    ] {
        let started = std::time::Instant::now();
        let proposal = runtime
            .propose(
                ProviderRequest {
                    before: before.to_owned(),
                    after: String::new(),
                    language: Some(language.to_owned()),
                },
                CancellationToken::new(),
                false,
            )
            .await
            .expect("inference");
        eprintln!(
            "synthetic_input={before:?} language={language} proposal={proposal:?} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(600));
        let proposal = proposal.expect("continuation in the development probe");
        assert!(proposal.replace_before.is_none());
        assert!(proposal.text.chars().count() <= 64);
        if before.ends_with("docum") {
            assert!(
                proposal.text.starts_with("ent"),
                "must complete document correctly"
            );
        } else if before.ends_with("autom") {
            assert!(
                proposal.text.starts_with("atically"),
                "must complete automatically correctly"
            );
        }
    }
    let background =
        "Earlier background remains available in the guarded editor snapshot. ".repeat(5);
    for ending in [
        "please review the latest",
        "please review the latest changes",
    ] {
        let before = format!("{background} Before sending the final report, {ending}");
        assert!(before.chars().count() <= 512);
        let started = std::time::Instant::now();
        let proposal = runtime
            .propose(
                ProviderRequest {
                    before,
                    after: String::new(),
                    language: Some("en".to_owned()),
                },
                CancellationToken::new(),
                false,
            )
            .await
            .expect("bounded cold/warm inference");
        eprintln!(
            "synthetic_long_context_end={ending:?} proposal={proposal:?} elapsed_ms={}",
            started.elapsed().as_millis()
        );
        assert!(started.elapsed() < std::time::Duration::from_millis(600));
        assert!(
            proposal.is_some(),
            "bounded recent sentence must remain usable on first request"
        );
    }
}
