//! Conservative language selection from an already authorized field snapshot.
//! This module never acquires text, retains samples or grants editing authority.

const WINDOW_SCALARS: usize = 512;
const MIN_SCALARS: usize = 32;
const MIN_CONFIDENCE: f64 = 0.9;

/// Return a qualified writing language only when the bounded field suffix is
/// long and confident enough. Callers retain their existing hint on abstention;
/// an explicit user override must take precedence over this inference.
#[must_use]
pub fn detect_field_language(before: &str) -> Option<&'static str> {
    match confident_language(before)? {
        whatlang::Lang::Eng => Some("en"),
        whatlang::Lang::Deu => Some("de"),
        whatlang::Lang::Pes => Some("fa"),
        whatlang::Lang::Fra => Some("fr"),
        whatlang::Lang::Spa => Some("es"),
        whatlang::Lang::Por => Some("pt"),
        _ => None,
    }
}

/// The confidently detected language of the bounded field suffix, from the
/// detector's full inventory: limiting it to the languages Badi supports
/// would force French, Spanish and other prose into an incorrect language.
#[must_use]
pub fn confident_language(before: &str) -> Option<whatlang::Lang> {
    let start = before
        .char_indices()
        .rev()
        .nth(WINDOW_SCALARS - 1)
        .map_or(0, |(index, _)| index);
    let sample = &before[start..];
    if sample.chars().count() < MIN_SCALARS {
        return None;
    }
    let detected = whatlang::detect(sample)?;
    (detected.confidence() >= MIN_CONFIDENCE).then(|| detected.lang())
}

#[cfg(test)]
mod tests;
