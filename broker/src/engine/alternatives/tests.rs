use super::{Alternatives, MAX_ALTERNATIVES, Plan};
use crate::provider::AlternativeAsk;

fn generate(excluded: &[&str], fallback: &str) -> Plan {
    Plan::Generate {
        ask: AlternativeAsk {
            excluded: excluded.iter().map(|word| (*word).to_owned()).collect(),
            new_word: true,
        },
        fallback: fallback.to_owned(),
    }
}

#[test]
fn each_press_asks_for_a_new_first_word_then_cycles() {
    let mut alternatives = Alternatives::default();
    assert_eq!(
        alternatives.next("f1", " for your time"),
        generate(&["for"], " for your time"),
        "without another, the shown one stays"
    );
    assert_eq!(
        alternatives.next("f1", " so much."),
        generate(&["for", "so"], " for your time")
    );
    assert_eq!(
        alternatives.next("f1", " again"),
        generate(&["for", "so", "again"], " for your time")
    );
    assert_eq!(MAX_ALTERNATIVES, 3);
    assert_eq!(
        alternatives.next("f1", " Anyway,"),
        Plan::Show(" for your time".to_owned()),
        "after three alternatives presses cycle"
    );
    assert_eq!(
        alternatives.next("f1", " for your time"),
        Plan::Show(" so much.".to_owned())
    );
}

#[test]
fn a_model_without_alternatives_cycles_and_a_new_context_starts_over() {
    let mut alternatives = Alternatives::default();
    let _ = alternatives.next("f1", " for your time");
    let _ = alternatives.next("f1", " so much.");
    alternatives.exhaust();
    assert_eq!(
        alternatives.next("f1", " so much."),
        Plan::Show(" for your time".to_owned())
    );
    assert_eq!(
        alternatives.next("f2", " for you"),
        generate(&["for"], " for you")
    );
}

#[test]
fn the_first_suggestion_tells_whether_alternatives_start_a_new_word() {
    let mut after_space = Alternatives::default();
    let Plan::Generate { ask, .. } = after_space.next("f1", "interest in our products.") else {
        panic!("a first press asks the model");
    };
    assert_eq!(
        ask,
        AlternativeAsk {
            excluded: vec!["interest".to_owned()],
            new_word: false,
        },
        "after a space the word at the caret is the one to choose"
    );
    let mut after_word = Alternatives::default();
    let Plan::Generate { ask, .. } = after_word.next("f1", " latest update") else {
        panic!("a first press asks the model");
    };
    assert!(ask.new_word, "after a finished word the next one starts");
}
