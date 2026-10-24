//! Per-app quietness from the writer's own counts (suggestions shown and
//! accepted, never text): where Badi's suggestions are seldom taken, it shows
//! fewer and surer ones. The quiet floor only ever tightens the confidence
//! floor (`writing::CONFIDENCE_FLOOR`, a user decision), and an explicit
//! request is never quieted. `badi stats forget` resets it with the counts.

use std::time::Duration;

use crate::stats::StatsCounts;

/// Suggestions shown in an app or site over seven days before its
/// acceptance rate counts.
pub(super) const MIN_SHOWN: u64 = 50;
/// How long an app's level is reused before its counts are read again.
pub(super) const REFRESH: Duration = Duration::from_secs(600);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum QuietLevel {
    #[default]
    Normal,
    Quieter,
    Quietest,
}

impl QuietLevel {
    /// The stricter lowest token log-probability a shown suggestion needs.
    pub(super) const fn floor(self) -> Option<f64> {
        match self {
            Self::Normal => None,
            Self::Quieter => Some(-2.25),
            Self::Quietest => Some(-1.75),
        }
    }

    /// The level after `counts`. A level tightens below one acceptance rate
    /// and loosens only well above it, so it does not flip from week to week;
    /// with too few suggestions to judge it stays as it was.
    pub(super) fn next(self, counts: StatsCounts) -> Self {
        if counts.shown == 0 {
            return Self::Normal;
        }
        if counts.shown < MIN_SHOWN {
            return self;
        }
        let percent = counts.accepted.saturating_mul(100) / counts.shown;
        match self {
            _ if percent < 4 => Self::Quietest,
            Self::Normal if percent < 8 => Self::Quieter,
            Self::Quieter if percent >= 15 => Self::Normal,
            Self::Quietest if percent >= 8 => Self::Quieter,
            level => level,
        }
    }
}

#[cfg(test)]
mod tests;
