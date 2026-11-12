//! Explicit examples belong to this case alone, in its declared language.
//! Nothing queries the desktop store or carries examples into another request.

use std::collections::BTreeSet;

use badi_broker::semantic::client::PromptBudget;
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

use crate::{Mode, Request, collect_fact_tokens};

#[derive(Debug, Serialize)]
pub struct StyleBudget {
    pub samples_supplied: usize,
    pub samples_selected: usize,
    /// Style never evicts draft text to make room. An overflowing arm abstains.
    pub draft_characters_dropped_for_style: usize,
    pub prompts: Vec<PromptBudget>,
}

pub(crate) const fn sample_limit(mode: Mode) -> Option<usize> {
    match mode {
        Mode::StyleGentle => Some(1),
        Mode::StyleBalanced => Some(2),
        Mode::StyleStrong => Some(4),
        _ => None,
    }
}

pub(crate) fn selected_examples(request: &Request) -> Option<Vec<String>> {
    let limit = sample_limit(request.config.mode)?;
    // Input is oldest to newest, like the sealed store's explicit export.
    // The newest example stays nearest the draft at every strength.
    Some(request.style_examples[request.style_examples.len().saturating_sub(limit)..].to_vec())
}

pub(crate) fn prefix(examples: &[String]) -> String {
    if examples.is_empty() {
        String::new()
    } else {
        format!("{}\n\n", examples.join("\n\n"))
    }
}

fn fact_word(word: &str) -> String {
    let lower = word.to_lowercase();
    // UAX #29 keeps a possessive suffix inside the word. Use the same name
    // key in examples, the draft and additions, with either apostrophe.
    lower
        .strip_suffix("'s")
        .or_else(|| lower.strip_suffix("’s"))
        .unwrap_or(&lower)
        .to_owned()
}

fn facts(text: &str, language: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    collect_fact_tokens(text, &mut found);
    // A conservative name fence, not entity recognition: capitalized words
    // include sentence starts and German nouns. Full-addition review still
    // judges semantic facts the mechanical fence cannot recognize.
    found.extend(
        text.unicode_words()
            .filter(|word| word.starts_with(char::is_uppercase))
            .map(fact_word),
    );
    if badi_broker::writing::WritingLanguage::from_tag(language)
        == Some(badi_broker::writing::WritingLanguage::Portuguese)
    {
        // Unicode word segmentation separates `sexta-feira` at the hyphen.
        // These weekday roots also catch the unhyphenated spelling.
        found.extend(text.unicode_words().map(str::to_lowercase).filter(|word| {
            ["segunda", "terça", "quarta", "quinta", "sexta"].contains(&word.as_str())
        }));
    }
    found
}

pub(crate) fn fact_conflict(candidate: &str, request: &Request, examples: &[String]) -> bool {
    let style = facts(&examples.join("\n"), &request.language);
    let mut current = facts(&request.before, &request.language);
    current.extend(facts(&request.after, &request.language));
    // Window context is ignored by this arm, so it cannot authorize a fact.
    // Include the seam: completing `Fri` as `day` introduces Friday too.
    let complete = format!("{}{candidate}", request.before);
    let stem = request
        .before
        .unicode_words()
        .next_back()
        .filter(|word| request.before.ends_with(word))
        .unwrap_or_default();
    let at_seam = format!("{stem}{candidate}");
    let introduced = at_seam.unicode_words().map(fact_word);
    introduced
        .into_iter()
        .any(|word| style.contains(&word) && !current.contains(&word))
        || {
            let mut numbers = BTreeSet::new();
            collect_fact_tokens(&complete, &mut numbers);
            numbers
                .iter()
                .any(|word| style.contains(word) && !current.contains(word))
        }
}

pub(crate) fn finish(
    result: &mut crate::ResultRecord,
    request: &Request,
    examples: &[String],
    prompts: Vec<PromptBudget>,
) {
    result.used_style = !examples.is_empty()
        && result.model_requests.iter().any(|payload| {
            payload
                .get("prompt")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|prompt| prompt.starts_with(&prefix(examples)))
        });
    result
        .warnings
        .push("experimental_style_not_qualified_for_production");
    if !request.context.is_empty() {
        result.warnings.push("window_context_ignored_by_style_arm");
    }
    if prompts.iter().any(|budget| !budget.fits()) {
        result.reason = "style_context_overflow";
    }
    result.style_budget = Some(StyleBudget {
        samples_supplied: request.style_examples.len(),
        samples_selected: examples.len(),
        draft_characters_dropped_for_style: 0,
        prompts,
    });
    if result.used_style
        && result.replace_before.is_none()
        && result
            .text
            .as_ref()
            .is_some_and(|text| fact_conflict(text, request, examples))
    {
        result.outcome = "abstention";
        result.reason = "style_fact_conflict";
        result.text = None;
        result.shape_valid = false;
        result.word_complete = false;
    }
}

#[cfg(test)]
mod tests;
