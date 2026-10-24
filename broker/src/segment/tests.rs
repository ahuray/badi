use serde::Deserialize;
use unicode_segmentation::UnicodeSegmentation;

use super::{OutputError, accept_word, sanitize_suggestion, validate_suggestion_shape};

#[derive(Deserialize)]
struct FixtureSet {
    v: u8,
    algorithm: String,
    cases: Vec<FixtureCase>,
}

#[derive(Deserialize)]
struct FixtureCase {
    name: String,
    input: String,
    accepted: String,
    remainder: String,
}

#[test]
fn accept_word_fixtures_match() {
    let fixtures: FixtureSet = serde_json::from_str(include_str!("accept-word-fixtures.json"))
        .expect("shared accept-word fixtures");
    assert_eq!(fixtures.v, 1);
    assert_eq!(
        fixtures.algorithm,
        "leading_whitespace_plus_first_unicode_word_or_grapheme_then_terminal_space_v2"
    );
    assert!(fixtures.cases.len() >= 7);
    for fixture in fixtures.cases {
        let parts = accept_word(&fixture.input);
        assert_eq!(
            parts.accepted, fixture.accepted,
            "{} accepted",
            fixture.name
        );
        assert_eq!(
            parts.remainder, fixture.remainder,
            "{} remainder",
            fixture.name
        );
    }
}

#[test]
fn attaches_leading_space_to_unicode_word() {
    let parts = accept_word("  café κόσμε");
    assert_eq!(parts.accepted, "  café");
    assert_eq!(parts.remainder, " κόσμε");

    let combining = accept_word(" e\u{301}lan vital");
    assert_eq!(combining.accepted, " e\u{301}lan");
    assert_eq!(combining.remainder, " vital");
}

#[test]
fn handles_emoji_without_splitting_a_grapheme() {
    let parts = accept_word(" 👩🏽‍💻 next");
    assert_eq!(parts.accepted, " 👩🏽‍💻");
    assert_eq!(parts.remainder, " next");
    assert!(parts.accepted.graphemes(true).any(|part| part == "👩🏽‍💻"));
}

#[test]
fn output_rejects_over_limit_values_instead_of_rewriting_them() {
    let long = " one two three four five six seven eight nine ten";
    assert_eq!(sanitize_suggestion(long), Err(OutputError::TooLong));
    assert_eq!(
        sanitize_suggestion(&"x".repeat(65)),
        Err(OutputError::TooLong)
    );
    assert_eq!(
        sanitize_suggestion(" one two three four five six seven eight"),
        Ok(" one two three four five six seven eight".to_owned())
    );
}

#[test]
fn output_keeps_one_trailing_space_and_rejects_any_other_trailing_whitespace() {
    assert_eq!(sanitize_suggestion(" valid "), Ok(" valid ".to_owned()));
    assert_eq!(
        sanitize_suggestion(" valid  "),
        Err(OutputError::InvalidShape)
    );
    assert_eq!(sanitize_suggestion("   "), Err(OutputError::Empty));
    assert_eq!(
        validate_suggestion_shape("Please find the ", "report today", "final "),
        Ok(())
    );
    for (before, after, suggestion) in [
        ("Please find the ", "", "final "),
        ("Thanks ", ", John", "a lot "),
        ("hello", "world", "final "),
        ("hello ", " report", "final "),
    ] {
        assert_eq!(
            validate_suggestion_shape(before, after, suggestion),
            Err(OutputError::InvalidShape),
            "{before:?} | {suggestion:?} | {after:?}"
        );
    }
}

#[test]
fn output_rejects_every_non_ascii_unicode_whitespace_scalar() {
    for whitespace in [
        '\u{0085}', '\u{00a0}', '\u{1680}', '\u{2000}', '\u{2001}', '\u{2002}', '\u{2003}',
        '\u{2004}', '\u{2005}', '\u{2006}', '\u{2007}', '\u{2008}', '\u{2009}', '\u{200a}',
        '\u{2028}', '\u{2029}', '\u{202f}', '\u{205f}', '\u{3000}',
    ] {
        assert!(matches!(
            sanitize_suggestion(&format!(" bad{whitespace}space")),
            Err(OutputError::ForbiddenControl | OutputError::InvalidShape)
        ));
    }
}

#[test]
fn context_shape_rejects_spacing_and_exact_overlap_failures() {
    for (before, after, suggestion) in [
        ("hello", "", "world"),
        ("hello ", "", " world"),
        ("hello", "world", " brave"),
        ("hello", "", " hello"),
        ("hello", " world", " world"),
        ("wait for", "", " for your time"),
        ("hello", "", "  world"),
        ("hello", "", " world  again"),
        ("hello", "", " \u{00a0}world"),
        ("café", "", "noir"),
        ("café", "", "éclair"),
        ("hello", " time remains", " for your time"),
    ] {
        assert_eq!(
            validate_suggestion_shape(before, after, suggestion),
            Err(OutputError::InvalidShape)
        );
    }
    assert_eq!(validate_suggestion_shape("hello", "", " world"), Ok(()));
    assert_eq!(
        validate_suggestion_shape("hello.", "", " Hello again"),
        Ok(())
    );
    assert_eq!(validate_suggestion_shape("class", "", " as"), Ok(()));
    assert_eq!(
        validate_suggestion_shape("hello", " world", " brave"),
        Ok(())
    );
    assert_eq!(validate_suggestion_shape("你", "", "好"), Ok(()));
}

#[test]
fn rejects_controls_instead_of_escaping_them() {
    assert_eq!(
        sanitize_suggestion(" do it\nnow"),
        Err(OutputError::ForbiddenControl)
    );
    assert_eq!(
        sanitize_suggestion(" press\tTab"),
        Err(OutputError::ForbiddenControl)
    );
    assert_eq!(
        sanitize_suggestion(" escape\u{1b}"),
        Err(OutputError::ForbiddenControl)
    );
    assert_eq!(
        sanitize_suggestion(" two\u{2028}lines"),
        Err(OutputError::ForbiddenControl)
    );
}

#[test]
fn output_rejects_ambiguous_unicode_or_repeated_spacing() {
    for suggestion in ["\u{00a0}world", " world\u{00a0}again", " world  again"] {
        assert_eq!(
            sanitize_suggestion(suggestion),
            Err(OutputError::InvalidShape)
        );
    }
}

#[test]
fn rejects_unicode_format_controls() {
    for hostile in [
        "safe\u{202e}hostile",
        "safe\u{2066}hostile",
        "safe\u{200b}hostile",
        "safe\u{00ad}hostile",
        "safe\u{13430}hostile",
        "safe\u{e007f}hostile",
    ] {
        assert_eq!(
            sanitize_suggestion(hostile),
            Err(OutputError::ForbiddenControl)
        );
    }
}
