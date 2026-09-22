//! Exact contextual word reuse for an explicitly selected Lab experiment.
//! Attestation is neither dictionary validation nor authority to edit a field.

use unicode_segmentation::UnicodeSegmentation;

use super::Request;
use crate::writing::{self, WritingLanguage};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Source {
    Context,
    Style,
}

#[derive(Debug, Eq, PartialEq)]
pub(super) struct AttestedSuffix<'a> {
    pub suffix: &'a str,
    pub source: Source,
}

/// The caller must first match and remove the exact generated echo and prove
/// the candidate's completion using the existing stream boundary rules. Neither
/// a contextual match nor a terminal transport event proves word completion.
/// This helper does not select a word: it validates the model's completed word.
pub(super) fn recover_from_complete_candidate<'a>(
    request: &Request,
    echo: &str,
    candidate: &'a str,
) -> Option<AttestedSuffix<'a>> {
    // Recheck the bounded input surface even when invoked outside run(). Only
    // original context/style fields below can provide attestation.
    request.validate().ok()?;
    let language = WritingLanguage::from_tag(&request.language)?;
    let (stem_start, stem) = request.before.unicode_word_indices().next_back()?;
    if !matches!(language, WritingLanguage::German | WritingLanguage::Persian)
        || stem_start + stem.len() != request.before.len()
        || !(2..=24).contains(&stem.chars().count())
        || !word_scalars(stem)
        || (stem_start != 0
            && !request.before[..stem_start]
                .chars()
                .next_back()
                .is_some_and(lexical_separator))
        || (!echo.is_empty() && echo != stem)
        || !request
            .before
            .chars()
            .next_back()
            .is_some_and(char::is_alphabetic)
        || !candidate.chars().next().is_some_and(char::is_alphabetic)
        || !language.accepts_output(candidate)
    {
        return None;
    }
    crate::segment::sanitize_suggestion(candidate).ok()?;
    writing::validate_suggestion_shape(&request.before, "", candidate).ok()?;
    if writing::validate_proposal(&request.before, "", candidate, Some(&request.language)).is_ok() {
        return None;
    }

    let (start, suffix) = candidate.unicode_word_indices().next()?;
    if start != 0
        || !word_scalars(suffix)
        || candidate[suffix.len()..]
            .chars()
            .next()
            .is_some_and(|next| !lexical_separator(next))
    {
        return None;
    }
    // Healed can have an empty echo when its English lexicon recognizes a
    // German stem (for example, "lad"). The affected word still comes from the
    // original draft, never from the echo or contextual prefix selection.
    let completed = format!("{stem}{suffix}");
    if !word_scalars(&completed)
        || !language.accepts_output(&completed)
        || !language.accepts_output(suffix)
    {
        return None;
    }
    // Shortening a previously guarded multiword candidate must not introduce
    // an overlap or a standalone joiner at the resulting suggestion boundary.
    crate::segment::sanitize_suggestion(suffix).ok()?;
    writing::validate_suggestion_shape(&request.before, "", suffix).ok()?;

    let source = if contains_closed_word(&request.context, &completed) {
        Source::Context
    } else if request
        .style_examples
        .iter()
        .any(|example| contains_closed_word(example, &completed))
    {
        Source::Style
    } else {
        return None;
    };
    Some(AttestedSuffix { suffix, source })
}

pub(super) fn word_scalars(word: &str) -> bool {
    !word.is_empty()
        && word.chars().all(|character| {
            character.is_alphabetic()
                || matches!(
                    character,
                    '\u{200c}' | '\u{0300}'..='\u{036f}' | '\u{064b}'..='\u{065f}' | '\u{0670}'
                )
        })
        && crate::segment::valid_orthographic_joiners(word)
}

fn contains_closed_word(source: &str, word: &str) -> bool {
    source.match_indices(word).any(|(start, _)| {
        let end = start + word.len();
        (start == 0
            || source[..start]
                .chars()
                .next_back()
                .is_some_and(lexical_separator))
            // A source-ending fragment is not a complete attested word. Scan
            // each original field separately; prompt-added separators do not
            // count and adjacent style examples cannot complete each other.
            && source[end..]
                .chars()
                .next()
                .is_some_and(lexical_separator)
    })
}

pub(super) fn lexical_separator(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '.' | ','
                | ';'
                | ':'
                | '!'
                | '?'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '"'
                | '\u{201c}'
                | '\u{201d}'
                | '\u{201e}'
                | '\u{00ab}'
                | '\u{00bb}'
                | '\u{2026}'
                | '\u{060c}'
                | '\u{061b}'
                | '\u{061f}'
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::writing_lab::{Config, Mode, REQUEST_SCHEMA, complete_output};

    fn request(before: &str, language: &str, context: &str) -> Request {
        Request {
            schema: REQUEST_SCHEMA.to_owned(),
            id: "attestation-fixture".to_owned(),
            before: before.to_owned(),
            language: language.to_owned(),
            context: context.to_owned(),
            style_examples: Vec::new(),
            config: Config {
                mode: Mode::Healed,
                budget_ms: 550,
                max_tokens: 8,
                cache_prompt: true,
                temperature: 0.0,
                seed: 42,
            },
        }
    }

    #[test]
    fn exact_context_attestation_recovers_only_the_first_model_word_suffix() {
        for (before, language, context, candidate, expected) in [
            (
                "Die Tagesord",
                "de",
                "Die Tagesordnung liegt bereit.",
                "nung liegt bereit.",
                "nung",
            ),
            (
                "من هر روز می‌ر",
                "fa",
                "من هر روز می‌روم.",
                "وم و بازمی‌گردم",
                "وم",
            ),
        ] {
            let value = request(before, language, context);
            let echo = writing::healing_prefix(before).unwrap();
            assert!(writing::validate_proposal(before, "", candidate, Some(language)).is_err());
            assert_eq!(
                recover_from_complete_candidate(&value, echo, candidate),
                Some(AttestedSuffix {
                    suffix: expected,
                    source: Source::Context
                })
            );
        }
    }

    #[test]
    fn explicit_style_has_distinct_provenance_without_cross_field_completion() {
        let mut value = request(
            "Die Nachricht geht an Marl",
            "de",
            "Die Empfängerin wartet.",
        );
        value
            .style_examples
            .push("Marlene erhält eine kurze Nachricht.".to_owned());
        assert_eq!(
            recover_from_complete_candidate(&value, "Marl", "ene"),
            Some(AttestedSuffix {
                suffix: "ene",
                source: Source::Style
            })
        );
        value.context = "Marlene".to_owned();
        value.style_examples = vec![".".to_owned(), "Marlene".to_owned(), ".".to_owned()];
        assert_eq!(recover_from_complete_candidate(&value, "Marl", "ene"), None);
        value.context.push('.');
        assert_eq!(
            recover_from_complete_candidate(&value, "Marl", "ene")
                .unwrap()
                .source,
            Source::Context
        );
    }

    #[test]
    fn exact_model_word_validation_does_not_choose_between_shared_prefixes() {
        let value = request(
            "Die Nachricht geht an Marl",
            "de",
            "Marlene organisiert das Fest. Marlies sammelt die Zusagen.",
        );
        for suffix in ["ene", "ies"] {
            assert_eq!(
                recover_from_complete_candidate(&value, "Marl", suffix)
                    .unwrap()
                    .suffix,
                suffix
            );
        }
        assert_eq!(recover_from_complete_candidate(&value, "Marl", "en"), None);
    }

    #[test]
    fn exact_surface_comparison_preserves_case_diacritics_and_arabic_letters() {
        for (before, language, echo, suffix, source) in [
            (
                "Die Nachricht geht an Marl",
                "de",
                "Marl",
                "ene",
                "marlene.",
            ),
            ("Die Übertr", "de", "Übertr", "äge", "Übertrage."),
            ("Die Übertr", "de", "Übertr", "äge", "U\u{0308}berträge."),
            ("گزارش برای کیا", "fa", "کیا", "ن", "گیرنده كیان است."),
            ("این گزار", "fa", "گزار", "ش", "گُزارش آماده است."),
            ("من می‌ر", "fa", "می‌ر", "وم", "من میروم."),
        ] {
            assert_eq!(
                recover_from_complete_candidate(&request(before, language, source), echo, suffix),
                None
            );
        }
        let value = request("Die Übertr", "de", "Überträge sind vorhanden.");
        assert!(recover_from_complete_candidate(&value, "Übertr", "äge").is_some());
    }

    #[test]
    fn a_whole_source_word_needs_an_observed_conservative_lexical_boundary() {
        for source in [
            "Tagesordnung",
            "Tagesordnungsteil.",
            "XTagesordnung.",
            "Tagesordnung-Entwurf.",
            "Tagesordnung's",
            "Tagesordnung’s",
            "Tagesordnung_Entwurf.",
            "Tagesordnung1.",
            "1Tagesordnung.",
            "Tagesordnung\u{0301}.",
            "/Tagesordnung.",
        ] {
            assert_eq!(
                recover_from_complete_candidate(
                    &request("Die Tagesord", "de", source),
                    "Tagesord",
                    "nung"
                ),
                None,
                "source boundary: {source:?}"
            );
        }
        for source in [
            "Tagesordnung.",
            "[Tagesordnung]",
            "Tagesordnung\n",
            "Tagesordnung\u{a0}",
        ] {
            assert!(
                recover_from_complete_candidate(
                    &request("Die Tagesord", "de", source),
                    "Tagesord",
                    "nung"
                )
                .is_some()
            );
        }
        let value = request("این گزار", "fa", "گزارش، آماده است.");
        assert!(recover_from_complete_candidate(&value, "گزار", "ش").is_some());
    }

    #[test]
    fn removed_and_substring_only_attestation_never_authorize_forced_targets() {
        for (before, language, echo, suffix, source) in [
            (
                "Die Tagesord",
                "de",
                "Tagesord",
                "nung",
                "Die Liste liegt bereit.",
            ),
            (
                "Die Nachricht geht an Marl",
                "de",
                "Marl",
                "ene",
                "Marleneplatz ist bekannt.",
            ),
            ("من می‌نو", "fa", "می‌نو", "یسم", "تهیهٔ جواب با من است."),
            ("امروز نا", "fa", "نا", "مه", "این خبر در روزنامه چاپ شد."),
        ] {
            assert_eq!(
                recover_from_complete_candidate(&request(before, language, source), echo, suffix),
                None
            );
        }
    }

    #[test]
    fn draft_labels_and_generated_words_cannot_supply_attestation() {
        let mut value = request(
            "Die Tagesordnung fehlt in der Tagesord",
            "de",
            "Die Liste liegt bereit.",
        );
        value.id = "Tagesordnung.".to_owned();
        assert_eq!(
            recover_from_complete_candidate(&value, "Tagesord", "nung Tagesordnung"),
            None
        );
        let mut wire = serde_json::json!({
            "schema": REQUEST_SCHEMA, "id": "labels-only", "before": "Die Tagesord",
            "language": "de", "context": "Die Liste liegt bereit.", "style_examples": [],
            "config": value.config
        });
        wire["expected"] = serde_json::json!("Tagesordnung");
        assert!(serde_json::from_value::<Request>(wire).is_err());
    }

    #[test]
    fn original_joiner_guards_apply_to_the_draft_and_standalone_suffix() {
        for (before, echo, candidate) in [
            ("من هر روز می", "می", "‌روم"),
            ("من هر روز می‌", "می‌", "روم"),
            ("من هر روز می‌‌ر", "می‌‌ر", "وم"),
            ("من هر روز می‌ر", "می‌ر", "وم‌"),
        ] {
            let value = request(before, "fa", "من هر روز می‌روم.");
            assert_eq!(
                recover_from_complete_candidate(&value, echo, candidate),
                None
            );
        }
    }

    #[test]
    fn attestation_never_proves_an_unterminated_generated_word_complete() {
        let value = request("Die Tagesord", "de", "Die Tagesordnung liegt bereit.");
        for (raw, natural_stop, expected) in [
            ("nung", false, None),
            ("nu", false, None),
            ("nu ", false, None),
            ("nung", true, Some("nung")),
            ("nung jetzt", false, Some("nung")),
        ] {
            let complete = complete_output(raw, natural_stop);
            let recovered = complete.as_deref().and_then(|candidate| {
                recover_from_complete_candidate(&value, "Tagesord", candidate)
            });
            assert_eq!(recovered.map(|recovered| recovered.suffix), expected);
        }
    }

    #[test]
    fn fallback_preserves_echo_language_shape_and_bounded_input_requirements() {
        let value = request("Die Tagesord", "de", "Die Tagesordnung liegt bereit.");
        for echo in [" ", "Tagesor", "tagesord"] {
            assert_eq!(recover_from_complete_candidate(&value, echo, "nung"), None);
        }
        for candidate in [
            " nung",
            "nung  liegt",
            "nung\n",
            "nung بررسی",
            "nung<bad>",
            "nung-Entwurf",
            "nung-",
            "nung/Entwurf",
            "nung'",
            "nung’s",
            "nung\u{0301}",
            "nung_Entwurf",
            "Tagesord",
        ] {
            assert_eq!(
                recover_from_complete_candidate(&value, "Tagesord", candidate),
                None
            );
        }
        assert_eq!(
            recover_from_complete_candidate(&value, "Tagesord", &"n".repeat(65)),
            None
        );
        let mut oversized = value.clone();
        oversized.context = "x".repeat(4097);
        assert_eq!(
            recover_from_complete_candidate(&oversized, "Tagesord", "nung"),
            None
        );
        oversized = value.clone();
        oversized.style_examples = vec!["Tagesordnung.".to_owned(); 9];
        assert_eq!(
            recover_from_complete_candidate(&oversized, "Tagesord", "nung"),
            None
        );
        let english = request("Please review the docum", "en", "The document is ready.");
        assert_eq!(
            recover_from_complete_candidate(&english, "docum", "ent"),
            None
        );
    }

    #[test]
    fn an_empty_echo_still_requires_the_exact_original_stem_and_attested_word() {
        let value = request("Wir möchten jetzt lad", "de", "Wir können jetzt laden.");
        assert_eq!(writing::healing_prefix(&value.before), None);
        assert_eq!(
            recover_from_complete_candidate(&value, "", "en"),
            Some(AttestedSuffix {
                suffix: "en",
                source: Source::Context
            })
        );
        for echo in [" ", "la", "lad ", "Lad", "Tagesord"] {
            assert_eq!(recover_from_complete_candidate(&value, echo, "en"), None);
        }
        for before in [
            "Wir möchten jetzt lad ",
            "Wir möchten jetzt lad1",
            "Wir möchten jetzt Rad-lad",
        ] {
            let invalid_seam = request(before, "de", "Wir können jetzt laden.");
            assert_eq!(
                recover_from_complete_candidate(&invalid_seam, "", "en"),
                None
            );
        }
        let substring = request("Wir möchten jetzt lad", "de", "Der Fahrradladen ist offen.");
        assert_eq!(recover_from_complete_candidate(&substring, "", "en"), None);
    }

    #[test]
    fn context_attestation_can_reuse_a_typo_but_does_not_certify_spelling() {
        let value = request("Die Tagesord", "de", "Die Tagesordnugn liegt bereit.");
        let recovered = recover_from_complete_candidate(&value, "Tagesord", "nugn").unwrap();
        assert_eq!(recovered.suffix, "nugn");
        assert_eq!(recovered.source, Source::Context);
    }
}
