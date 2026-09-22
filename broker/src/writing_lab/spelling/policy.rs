use crate::writing::WritingLanguage;

pub(super) fn word_valid(word: &str, language: &str) -> bool {
    !word.is_empty() && word.chars().all(|c| c.is_alphabetic()
        || matches!(c, '\u{200c}' | '\u{0300}'..='\u{036f}' | '\u{064b}'..='\u{065f}' | '\u{0670}'))
        && WritingLanguage::from_tag(language).is_some_and(|value| value.accepts_output(word))
        && crate::segment::valid_orthographic_joiners(word)
}

pub(super) fn target<'a>(before: &'a str, language: &str) -> Option<&'a str> {
    let prefix = before.strip_suffix(' ')?;
    if prefix.ends_with(char::is_whitespace) {
        return None;
    }
    let word = prefix.split_whitespace().next_back()?;
    ((3..=24).contains(&word.chars().count()) && word_valid(word, language)).then_some(word)
}

#[derive(Eq, PartialEq)]
enum Case {
    Lower,
    Title,
    Upper,
    Uncased,
}
fn case(word: &str) -> Option<Case> {
    let letters: Vec<_> = word
        .chars()
        .filter(|c| c.is_lowercase() || c.is_uppercase())
        .collect();
    if letters.is_empty() {
        Some(Case::Uncased)
    } else if letters.iter().all(|c| c.is_lowercase()) {
        Some(Case::Lower)
    } else if letters.iter().all(|c| c.is_uppercase()) {
        Some(Case::Upper)
    } else if letters[0].is_uppercase() && letters[1..].iter().all(|c| c.is_lowercase()) {
        Some(Case::Title)
    } else {
        None
    }
}

pub(super) fn one_edit(original: &str, corrected: &str) -> bool {
    let a: Vec<_> = original.chars().collect();
    let b: Vec<_> = corrected.chars().collect();
    if a == b || a.first() != b.first() || a.is_empty() || b.is_empty() {
        return false;
    }
    let mismatch = a
        .iter()
        .zip(&b)
        .position(|(a, b)| a != b)
        .unwrap_or(a.len().min(b.len()));
    match a.len().cmp(&b.len()) {
        std::cmp::Ordering::Equal => {
            (a[mismatch] != '\u{200c}'
                && b[mismatch] != '\u{200c}'
                && a[mismatch + 1..] == b[mismatch + 1..])
                || (mismatch + 1 < a.len()
                    && a[mismatch] != '\u{200c}'
                    && a[mismatch + 1] != '\u{200c}'
                    && a[mismatch] == b[mismatch + 1]
                    && a[mismatch + 1] == b[mismatch]
                    && a[mismatch + 2..] == b[mismatch + 2..])
        }
        std::cmp::Ordering::Greater => {
            a.len() == b.len() + 1
                && a[mismatch] != '\u{200c}'
                && a[mismatch + 1..] == b[mismatch..]
        }
        std::cmp::Ordering::Less => {
            b.len() == a.len() + 1
                && b[mismatch] != '\u{200c}'
                && a[mismatch..] == b[mismatch + 1..]
        }
    }
}

pub(super) fn filter(original: &str, candidates: Vec<String>, language: &str) -> Vec<String> {
    let mut accepted = std::collections::BTreeSet::new();
    for candidate in candidates {
        if (3..=24).contains(&candidate.chars().count())
            && word_valid(&candidate, language)
            && case(original).is_some()
            && case(original) == case(&candidate)
            && one_edit(original, &candidate)
        {
            accepted.insert(candidate);
        }
    }
    accepted.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_completed_exact_single_space_words_are_targets() {
        assert_eq!(target("Das Fahrad ", "de"), Some("Fahrad"));
        assert_eq!(target("این گزاررش ", "fa"), Some("گزاررش"));
        for before in [
            "Das Fahrad",
            "Das Fahrad  ",
            "Das Fahrad\t ",
            "Fahrad\u{a0}",
            "Fahrad- ",
            "hi ",
            "می‌ ",
        ] {
            assert_eq!(target(before, "de"), None);
        }
    }
    #[test]
    fn scalar_edits_preserve_first_letter_case_and_joiners() {
        for (a, b) in [
            ("Fahrad", "Fahrrad"),
            ("Fahrrrad", "Fahrrad"),
            ("Fahrxad", "Fahrrad"),
            ("Fahrrda", "Fahrrad"),
            ("گزاررش", "گزارش"),
        ] {
            assert!(one_edit(a, b));
        }
        for (a, b) in [
            ("Fahrad", "fahrrad"),
            ("Fahrad", "Fahrad"),
            ("Fard", "Fahrrad"),
            ("میروم", "می‌روم"),
            ("می‌روم", "میروم"),
            ("می‌روم", "میر‌وم"),
        ] {
            assert!(!one_edit(a, b));
        }
        assert_eq!(
            filter(
                "Fahrad",
                vec![
                    "Fahrrad".into(),
                    "Fahrrad".into(),
                    "fahrrad".into(),
                    "Fahr-rad".into()
                ],
                "de"
            ),
            vec!["Fahrrad"]
        );
    }
    #[test]
    fn ambiguous_returned_candidates_stay_distinct() {
        assert_eq!(
            filter(
                "Hase",
                vec!["Hose".into(), "Haxe".into(), "Hase".into()],
                "de"
            ),
            vec!["Haxe", "Hose"]
        );
        assert!(filter("bAdI", vec!["bAdE".into()], "de").is_empty());
    }
}
