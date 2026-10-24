use super::{MIN_SHOWN, QuietLevel};
use crate::stats::StatsCounts;

fn counts(shown: u64, accepted: u64) -> StatsCounts {
    StatsCounts {
        shown,
        accepted,
        words: accepted,
    }
}

#[test]
fn seldom_taken_suggestions_raise_the_floor_and_only_ever_tighten_it() {
    assert_eq!(QuietLevel::Normal.floor(), None);
    assert!(QuietLevel::Quieter.floor() > Some(crate::writing::CONFIDENCE_FLOOR));
    assert!(QuietLevel::Quietest.floor() > QuietLevel::Quieter.floor());
    assert_eq!(
        QuietLevel::Normal.next(counts(352, 20)),
        QuietLevel::Quieter,
        "5.7%"
    );
    assert_eq!(
        QuietLevel::Normal.next(counts(100, 3)),
        QuietLevel::Quietest
    );
    assert_eq!(QuietLevel::Normal.next(counts(100, 14)), QuietLevel::Normal);
}

#[test]
fn a_level_loosens_only_well_above_where_it_tightened() {
    assert_eq!(
        QuietLevel::Quieter.next(counts(100, 10)),
        QuietLevel::Quieter
    );
    assert_eq!(
        QuietLevel::Quieter.next(counts(100, 15)),
        QuietLevel::Normal
    );
    assert_eq!(
        QuietLevel::Quietest.next(counts(100, 6)),
        QuietLevel::Quietest
    );
    assert_eq!(
        QuietLevel::Quietest.next(counts(100, 8)),
        QuietLevel::Quieter
    );
    assert_eq!(
        QuietLevel::Quieter.next(counts(100, 2)),
        QuietLevel::Quietest
    );
}

#[test]
fn too_few_suggestions_keep_the_level_and_forgotten_counts_reset_it() {
    assert_eq!(
        QuietLevel::Quieter.next(counts(MIN_SHOWN - 1, 0)),
        QuietLevel::Quieter
    );
    assert_eq!(
        QuietLevel::Normal.next(counts(MIN_SHOWN - 1, 0)),
        QuietLevel::Normal
    );
    assert_eq!(QuietLevel::Quietest.next(counts(0, 0)), QuietLevel::Normal);
}
