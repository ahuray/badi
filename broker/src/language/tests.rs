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
