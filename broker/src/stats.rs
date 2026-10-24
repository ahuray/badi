//! Usage statistics: how many suggestions were shown, how many acceptances
//! landed and how many words they saved, per local calendar day and per exact
//! app id or site origin.
//!
//! Only counts are kept, never text: no typed word, suggestion or field
//! detail. A day and its subjects stay at most [`STATS_RETENTION_DAYS`] days in
//! `stats.json` in the private data directory, written atomically with mode
//! 0600 by the outcome worker. Nothing is counted while settings pause Badi or
//! for an app or site that may not suggest. `stats.forget` deletes the file.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;

use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::protocol::MAX_SAFE_COUNTER;
use crate::settings::{
    PrivateStorageError, SettingsV2, StableIdentity, atomic_write_private, read_private_limited,
    remove_private_file, remove_private_temporary_files,
};

pub const STATS_SCHEMA: &str = "badi.stats.v1";
pub const STATS_FILE_NAME: &str = "stats.json";
/// Days kept, today included; older days are deleted.
pub const STATS_RETENTION_DAYS: u16 = 90;
/// Apps and sites counted on one day; later ones add to the day's `other`.
pub const MAX_SUBJECTS_PER_DAY: usize = 64;
/// Subjects a summary names for each of its windows.
pub const MAX_SUMMARY_SUBJECTS: usize = 16;
/// Every valid document fits: 91 days of 64 subjects with the longest host
/// and the largest counters encode to about 2.7 MB.
pub const MAX_STATS_BYTES: usize = 4 * 1024 * 1024;

/// One counted moment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatsEvent {
    /// A suggestion the broker showed. Type-through remainders of it are
    /// the same suggestion.
    Shown,
    /// An acceptance the adapter reported `applied` or
    /// `dispatched_unverified`, with the word parts it inserted (zero for a
    /// spelling replacement).
    Accepted { words: u64 },
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatsCounts {
    pub shown: u64,
    pub accepted: u64,
    pub words: u64,
}

impl StatsCounts {
    #[must_use]
    pub const fn is_zero(&self) -> bool {
        self.shown == 0 && self.accepted == 0 && self.words == 0
    }

    fn add(&mut self, other: Self) {
        self.shown = add_bounded(self.shown, other.shown);
        self.accepted = add_bounded(self.accepted, other.accepted);
        self.words = add_bounded(self.words, other.words);
    }

    fn record(&mut self, event: StatsEvent) {
        match event {
            StatsEvent::Shown => self.shown = add_bounded(self.shown, 1),
            StatsEvent::Accepted { words } => {
                self.accepted = add_bounded(self.accepted, 1);
                self.words = add_bounded(self.words, words);
            }
        }
    }

    const fn valid(&self) -> bool {
        self.shown <= MAX_SAFE_COUNTER
            && self.accepted <= MAX_SAFE_COUNTER
            && self.words <= MAX_SAFE_COUNTER
    }
}

/// Counters stay JSON-safe integers for the panel and scripts.
fn add_bounded(left: u64, right: u64) -> u64 {
    left.saturating_add(right).min(MAX_SAFE_COUNTER)
}

/// One day's totals, for a summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatsDay {
    /// The local calendar date, `YYYY-MM-DD`.
    pub day: String,
    pub shown: u64,
    pub accepted: u64,
    pub words: u64,
}

/// One app or site's counts over a summary's windows.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatsSubject {
    pub identity: StableIdentity,
    pub today: StatsCounts,
    pub last_7_days: StatsCounts,
    pub retained: StatsCounts,
}

/// Exact totals over each window, whichever subjects a summary names.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatsTotals {
    pub today: StatsCounts,
    pub last_7_days: StatsCounts,
    pub retained: StatsCounts,
}

/// What `stats.query` reports: every kept day's totals and the apps and
/// sites that lead any window. Bounded to fit one frame.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StatsSummary {
    /// The broker's local calendar date, `YYYY-MM-DD`.
    pub today: String,
    pub retention_days: u64,
    pub totals: StatsTotals,
    /// Days with counts, oldest first.
    pub days: Vec<StatsDay>,
    /// The union of the top [`MAX_SUMMARY_SUBJECTS`] of each window by words,
    /// then shown; ordered by retained words, then shown, then identity.
    pub subjects: Vec<StatsSubject>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct DayCounts {
    subjects: BTreeMap<StableIdentity, StatsCounts>,
    /// Subjects past [`MAX_SUBJECTS_PER_DAY`], so the day's totals stay exact.
    other: StatsCounts,
}

impl DayCounts {
    fn total(&self) -> StatsCounts {
        let mut total = self.other;
        for counts in self.subjects.values() {
            total.add(*counts);
        }
        total
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StatsDocument {
    schema: String,
    days: Vec<DayRecord>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DayRecord {
    day: String,
    subjects: Vec<SubjectRecord>,
    #[serde(default, skip_serializing_if = "StatsCounts::is_zero")]
    other: StatsCounts,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SubjectRecord {
    identity: StableIdentity,
    shown: u64,
    accepted: u64,
    words: u64,
}

#[derive(Debug)]
pub struct StatsStore {
    path: PathBuf,
    days: BTreeMap<Date, DayCounts>,
    /// Counts not yet on disk; the worker persists them on its sweep, before a
    /// pause is acknowledged and at shutdown.
    dirty: bool,
}

impl StatsStore {
    /// Reads the kept days; a corrupt file is an error the caller reports,
    /// and stays untouched until `forget`. A copy an interrupted write left
    /// beside it is removed.
    pub fn open(path: PathBuf) -> Result<Self, StatsError> {
        remove_private_temporary_files(&path)?;
        let days = match read_private_limited(&path, MAX_STATS_BYTES)? {
            Some(bytes) => decode(&bytes)?,
            None => BTreeMap::new(),
        };
        Ok(Self {
            path,
            days,
            dirty: false,
        })
    }

    /// Counts `event` for `identity` on `today`, unless the settings pause
    /// Badi or keep suggestions from that app or site. Returns whether a
    /// count changed.
    pub fn record(
        &mut self,
        settings: &SettingsV2,
        today: Date,
        identity: &StableIdentity,
        event: StatsEvent,
    ) -> bool {
        if !counts_for(settings, identity) || identity.validate().is_err() {
            return false;
        }
        self.reconcile(today);
        let day = self.days.entry(today).or_default();
        let counts =
            if day.subjects.contains_key(identity) || day.subjects.len() < MAX_SUBJECTS_PER_DAY {
                day.subjects.entry(identity.clone()).or_default()
            } else {
                &mut day.other
            };
        counts.record(event);
        self.dirty = true;
        true
    }

    /// Drops days outside the kept window: the last
    /// [`STATS_RETENTION_DAYS`] days, plus tomorrow, so moving west across a
    /// time zone keeps the day just counted.
    pub fn reconcile(&mut self, today: Date) -> bool {
        let before = self.days.len();
        self.days.retain(|day, _| within_retention(*day, today));
        let changed = self.days.len() != before;
        self.dirty |= changed;
        changed
    }

    /// Writes pending counts; an empty store removes the file.
    pub fn persist(&mut self) -> Result<bool, StatsError> {
        if !self.dirty {
            return Ok(false);
        }
        if self.days.is_empty() {
            remove_private_file(&self.path)?;
        } else {
            atomic_write_private(&self.path, &encode(&self.days)?)?;
        }
        self.dirty = false;
        Ok(true)
    }

    /// Deletes every count and the file.
    pub fn forget(&mut self) -> Result<(), StatsError> {
        self.days.clear();
        remove_private_temporary_files(&self.path)?;
        remove_private_file(&self.path)?;
        self.dirty = false;
        Ok(())
    }

    /// A summary with no counts, for a store that cannot be read.
    #[must_use]
    pub fn empty_summary(today: Date) -> StatsSummary {
        StatsSummary {
            today: today.to_string(),
            retention_days: u64::from(STATS_RETENTION_DAYS),
            totals: StatsTotals::default(),
            days: Vec::new(),
            subjects: Vec::new(),
        }
    }

    /// One app or site's counts over the seven days ending `today`.
    #[must_use]
    pub fn last_7_days(&self, identity: &StableIdentity, today: Date) -> StatsCounts {
        let week_start = today
            .checked_sub(jiff::Span::new().days(6))
            .unwrap_or(Date::MIN);
        let mut counts = StatsCounts::default();
        for day in self.days.range(week_start..=today).map(|(_, day)| day) {
            if let Some(subject) = day.subjects.get(identity) {
                counts.add(*subject);
            }
        }
        counts
    }

    #[must_use]
    pub fn summary(&self, today: Date) -> StatsSummary {
        let week_start = today
            .checked_sub(jiff::Span::new().days(6))
            .unwrap_or(Date::MIN);
        let mut totals = StatsTotals::default();
        let mut subjects: BTreeMap<&StableIdentity, StatsSubject> = BTreeMap::new();
        let mut days = Vec::new();
        for (day, counts) in self
            .days
            .iter()
            .filter(|(day, _)| within_retention(**day, today))
        {
            let in_today = *day == today;
            let in_week = *day >= week_start && *day <= today;
            let total = counts.total();
            totals.retained.add(total);
            if in_week {
                totals.last_7_days.add(total);
            }
            if in_today {
                totals.today.add(total);
            }
            days.push(StatsDay {
                day: day.to_string(),
                shown: total.shown,
                accepted: total.accepted,
                words: total.words,
            });
            for (identity, counts) in &counts.subjects {
                let subject = subjects.entry(identity).or_insert_with(|| StatsSubject {
                    identity: identity.clone(),
                    today: StatsCounts::default(),
                    last_7_days: StatsCounts::default(),
                    retained: StatsCounts::default(),
                });
                subject.retained.add(*counts);
                if in_week {
                    subject.last_7_days.add(*counts);
                }
                if in_today {
                    subject.today.add(*counts);
                }
            }
        }
        let subjects: Vec<StatsSubject> = subjects.into_values().collect();
        let windows: [fn(&StatsSubject) -> StatsCounts; 3] = [
            |subject| subject.today,
            |subject| subject.last_7_days,
            |subject| subject.retained,
        ];
        let mut named = vec![false; subjects.len()];
        for window in windows {
            let mut order: Vec<usize> = (0..subjects.len())
                .filter(|index| !window(&subjects[*index]).is_zero())
                .collect();
            order.sort_by(|left, right| {
                let (left, right) = (&subjects[*left], &subjects[*right]);
                rank(window(right))
                    .cmp(&rank(window(left)))
                    .then_with(|| left.identity.cmp(&right.identity))
            });
            for index in order.into_iter().take(MAX_SUMMARY_SUBJECTS) {
                named[index] = true;
            }
        }
        let mut subjects: Vec<StatsSubject> = subjects
            .into_iter()
            .zip(named)
            .filter_map(|(subject, named)| named.then_some(subject))
            .collect();
        subjects.sort_by(|left, right| {
            rank(right.retained)
                .cmp(&rank(left.retained))
                .then_with(|| left.identity.cmp(&right.identity))
        });
        StatsSummary {
            today: today.to_string(),
            retention_days: u64::from(STATS_RETENTION_DAYS),
            totals,
            days,
            subjects,
        }
    }
}

/// Words saved lead, then suggestions shown.
const fn rank(counts: StatsCounts) -> (u64, u64, u64) {
    (counts.words, counts.shown, counts.accepted)
}

/// Counted only where the settings let this app or site suggest, as the
/// engine admits data there, and never while they pause Badi.
fn counts_for(settings: &SettingsV2, identity: &StableIdentity) -> bool {
    let resolution = settings.resolve_identity_validated(identity);
    resolution.configured()
        && resolution.allows_context_read()
        && resolution.allows_display()
        && resolution.allows_suggestion()
}

fn within_retention(day: Date, today: Date) -> bool {
    let first = today
        .checked_sub(jiff::Span::new().days(i64::from(STATS_RETENTION_DAYS) - 1))
        .unwrap_or(Date::MIN);
    let last = today.tomorrow().unwrap_or(Date::MAX);
    day >= first && day <= last
}

/// The local calendar date now, in the system time zone (UTC when it cannot
/// be determined).
#[must_use]
pub fn local_today() -> Date {
    jiff::Zoned::now().date()
}

fn parse_day(text: &str) -> Result<Date, StatsError> {
    let day = Date::from_str(text).map_err(|_| StatsError::Corrupt)?;
    if day.to_string() == text {
        Ok(day)
    } else {
        Err(StatsError::Corrupt)
    }
}

fn decode(bytes: &[u8]) -> Result<BTreeMap<Date, DayCounts>, StatsError> {
    let document: StatsDocument = serde_json::from_slice(bytes).map_err(|_| StatsError::Corrupt)?;
    if document.schema != STATS_SCHEMA {
        return Err(StatsError::Corrupt);
    }
    let mut days = BTreeMap::new();
    let mut previous_day = None;
    for record in document.days {
        let day = parse_day(&record.day)?;
        if previous_day.is_some_and(|previous| previous >= day)
            || record.subjects.len() > MAX_SUBJECTS_PER_DAY
            || !record.other.valid()
        {
            return Err(StatsError::Corrupt);
        }
        previous_day = Some(day);
        let mut subjects = BTreeMap::new();
        let mut previous_identity: Option<&StableIdentity> = None;
        for subject in &record.subjects {
            let counts = StatsCounts {
                shown: subject.shown,
                accepted: subject.accepted,
                words: subject.words,
            };
            if subject.identity.validate().is_err()
                || previous_identity.is_some_and(|previous| previous >= &subject.identity)
                || counts.is_zero()
                || !counts.valid()
            {
                return Err(StatsError::Corrupt);
            }
            previous_identity = Some(&subject.identity);
            subjects.insert(subject.identity.clone(), counts);
        }
        if subjects.is_empty() && record.other.is_zero() {
            return Err(StatsError::Corrupt);
        }
        days.insert(
            day,
            DayCounts {
                subjects,
                other: record.other,
            },
        );
    }
    Ok(days)
}

fn encode(days: &BTreeMap<Date, DayCounts>) -> Result<Vec<u8>, StatsError> {
    let document = StatsDocument {
        schema: STATS_SCHEMA.to_owned(),
        days: days
            .iter()
            .map(|(day, counts)| DayRecord {
                day: day.to_string(),
                subjects: counts
                    .subjects
                    .iter()
                    .map(|(identity, counts)| SubjectRecord {
                        identity: identity.clone(),
                        shown: counts.shown,
                        accepted: counts.accepted,
                        words: counts.words,
                    })
                    .collect(),
                other: counts.other,
            })
            .collect(),
    };
    // Machine-owned state; compact so the largest valid document stays far
    // below the read limit.
    let mut bytes = serde_json::to_vec(&document).map_err(|_| StatsError::Corrupt)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_STATS_BYTES {
        return Err(StatsError::TooLarge);
    }
    Ok(bytes)
}

#[derive(Debug, Error)]
pub enum StatsError {
    #[error("stats_corrupt")]
    Corrupt,
    #[error("stats_too_large")]
    TooLarge,
    #[error("stats_storage: {0}")]
    Storage(#[from] PrivateStorageError),
}

#[cfg(test)]
mod tests;
