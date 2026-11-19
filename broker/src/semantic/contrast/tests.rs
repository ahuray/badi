use super::{ScoredToken, contrastive_choice};

const FLOOR: f64 = -2.75;

fn token(id: u64, text: &str, logprob: f64) -> ScoredToken {
    ScoredToken {
        id: Some(id),
        text: text.to_owned(),
        logprob,
    }
}

#[test]
fn a_token_the_small_model_finds_obvious_gives_way() {
    // The writer prefers ` the`; the small model is even surer of it, so the
    // contrastive score picks the more specific ` Caribbean`.
    let writer = [token(1, " the", -0.9), token(2, " Caribbean", -1.4)];
    let contrast = [token(1, " the", -0.2), token(2, " Caribbean", -4.0)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        Some(" Caribbean")
    );
}

#[test]
fn the_writers_own_best_is_no_change() {
    let writer = [token(1, " to", -0.3), token(2, " for", -1.9)];
    let contrast = [token(1, " to", -2.0), token(2, " for", -0.5)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        None
    );
}

#[test]
fn a_candidate_below_the_floor_or_a_tenth_of_the_best_never_wins() {
    // ` Paris` scores best contrastively but is below the confidence floor.
    let writer = [token(1, " the", -1.0), token(2, " Paris", -2.9)];
    let contrast = [token(1, " the", -0.1), token(2, " Paris", -9.0)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        None
    );
    // Above the floor, but less than a tenth as likely as the best.
    let writer = [token(1, " the", -0.1), token(2, " Paris", -2.5)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        None
    );
}

#[test]
fn a_token_the_small_model_did_not_list_counts_as_least_likely() {
    let writer = [token(1, " the", -0.8), token(2, " Miwok", -1.5)];
    let contrast = [token(1, " the", -0.3), token(3, " a", -6.0)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        Some(" Miwok")
    );
}

#[test]
fn only_tokens_the_grammar_and_echo_allow_are_chosen() {
    let contrast = [token(9, " x", -1.0)];
    // A healed trailing space: the token must reproduce it and start a word.
    let writer = [
        token(1, "the", -0.5),
        token(2, " ", -0.6),
        token(3, " team", -1.2),
    ];
    assert_eq!(
        contrastive_choice(&writer, &contrast, " ", false, FLOOR),
        None,
        "the only admissible token is the writer's best"
    );
    let writer = [
        token(1, " the", -0.5),
        token(2, " <b>", -0.6),
        token(3, " 2024", -0.7),
        token(4, " team", -1.2),
    ];
    let contrast = [
        token(1, " the", -0.1),
        token(2, " <b>", -9.0),
        token(3, " 2024", -8.0),
        token(4, " team", -5.0),
    ];
    assert_eq!(
        contrastive_choice(&writer, &contrast, " ", false, FLOOR),
        Some(" team")
    );
    assert_eq!(
        contrastive_choice(&writer, &contrast, " ", true, FLOOR),
        Some(" team"),
        "a number is never forced, even where the typed text has digits"
    );
    let numbers_first = [token(3, " 2024", -0.4), token(4, " team", -1.2)];
    assert_eq!(
        contrastive_choice(&numbers_first, &contrast, " ", true, FLOOR),
        None,
        "the writer's own number stays its greedy pick"
    );
}

#[test]
fn without_contrast_scores_nothing_is_forced() {
    let writer = [token(1, " the", -0.9), token(2, " Caribbean", -1.4)];
    assert_eq!(contrastive_choice(&writer, &[], "", false, FLOOR), None);
    assert_eq!(contrastive_choice(&[], &writer, "", false, FLOOR), None);
}

#[test]
fn tokens_match_by_text_when_an_id_is_missing() {
    let writer = [
        ScoredToken {
            id: None,
            text: " the".to_owned(),
            logprob: -0.9,
        },
        token(2, " Caribbean", -1.4),
    ];
    let contrast = [token(1, " the", -0.2), token(2, " Caribbean", -4.0)];
    assert_eq!(
        contrastive_choice(&writer, &contrast, "", false, FLOOR),
        Some(" Caribbean")
    );
}
