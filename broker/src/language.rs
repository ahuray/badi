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
mod tests {
    use super::detect_field_language;

    #[test]
    fn writing_language_prose_is_detected_but_short_and_other_language_text_abstains() {
        for (text, language) in [
            (
                "Thank you for taking the time to read this message and for your help.",
                Some("en"),
            ),
            (
                "Vielen Dank für Ihre Nachricht. Ich freue mich auf unser nächstes Gespräch.",
                Some("de"),
            ),
            (
                "از شما برای خواندن این پیام و همکاری با ما بسیار سپاسگزارم.",
                Some("fa"),
            ),
            (
                "Merci pour votre message et pour le temps que vous nous avez consacré.",
                Some("fr"),
            ),
            (
                "Muchas gracias por su mensaje y por el tiempo que nos ha dedicado.",
                Some("es"),
            ),
            (
                "Muito obrigado pela sua mensagem e pelo tempo que nos dedicou hoje.",
                Some("pt"),
            ),
            (
                "Grazie mille per il tuo messaggio e per il tempo che ci hai dedicato.",
                None,
            ),
            ("Vielen Dank", None),
            ("1234567890 1234567890 1234567890 1234567890", None),
        ] {
            assert_eq!(detect_field_language(text), language);
        }
        let recent = "Vielen Dank für Ihre Nachricht. ".repeat(32);
        let mixed = format!(
            "{}{}",
            "English writing before a language switch. ".repeat(100),
            recent
        );
        assert_eq!(
            detect_field_language(&mixed),
            detect_field_language(&recent)
        );
    }
}
