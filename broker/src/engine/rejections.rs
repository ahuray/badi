//! Next words the writer turned down: dismissed with Escape, or typed over
//! with other letters. An automatic suggestion whose next word was turned
//! down at the same spot in the same app or site is withheld, so Badi does
//! not offer it again unless the writer takes it there. Only keyed hashes are
//! kept, never text, in memory, and the broker forgets them when it restarts.

use std::collections::VecDeque;
use std::hash::BuildHasher as _;
use std::hash::RandomState;

use super::type_through::CarryScope;

/// Turned-down spots remembered, oldest forgotten first.
const CAPACITY: usize = 256;
/// Escape is a clear no: one dismissal withholds that word there.
pub(super) const DISMISSED: u8 = 2;
/// Typing other letters may just be a typo: it takes two.
pub(super) const TYPED_OVER: u8 = 1;
/// The weight at which a next word is withheld at its spot.
const WITHHELD_AT: u8 = 2;
/// How many words before the caret make a spot.
const SPOT_WORDS: usize = 3;

pub(super) struct Rejections {
    hasher: RandomState,
    entries: VecDeque<(u64, u8)>,
}

impl Default for Rejections {
    fn default() -> Self {
        Self {
            hasher: RandomState::new(),
            entries: VecDeque::with_capacity(CAPACITY),
        }
    }
}

impl Rejections {
    /// Counts `weight` against the next word of `text` after `before`.
    pub(super) fn record(&mut self, scope: &CarryScope, before: &str, text: &str, weight: u8) {
        let Some(key) = self.key(scope, before, text) else {
            return;
        };
        let weight = match self.entries.iter().position(|(entry, _)| *entry == key) {
            Some(index) => self
                .entries
                .remove(index)
                .map_or(weight, |(_, earlier)| earlier.saturating_add(weight)),
            None => weight,
        };
        if self.entries.len() == CAPACITY {
            self.entries.pop_front();
        }
        self.entries.push_back((key, weight));
    }

    /// Records the word a carried suggestion lost when the writer typed other
    /// letters at it: `typed` follows `before`, where `text` was shown.
    pub(super) fn record_typed_over(
        &mut self,
        scope: &CarryScope,
        before: &str,
        text: &str,
        typed: &str,
    ) {
        let matched: usize = text
            .chars()
            .zip(typed.chars())
            .take_while(|(shown, written)| shown == written)
            .map(|(shown, _)| shown.len_utf8())
            .sum();
        if matched >= typed.len() {
            return;
        }
        // The words typed in full were taken; the word being typed was not.
        let word_start = text[..matched]
            .rfind(char::is_whitespace)
            .map_or(0, |index| index + 1);
        let spot = format!("{before}{}", &text[..word_start]);
        self.record(scope, &spot, &text[word_start..], TYPED_OVER);
    }

    /// Forgets any turn-down of the next word of `text` at this spot: the
    /// writer took it there.
    pub(super) fn forget(&mut self, scope: &CarryScope, before: &str, text: &str) {
        if let Some(key) = self.key(scope, before, text) {
            self.entries.retain(|(entry, _)| *entry != key);
        }
    }

    /// Whether the next word of `text` was turned down at this spot.
    pub(super) fn withholds(&self, scope: &CarryScope, before: &str, text: &str) -> bool {
        self.key(scope, before, text).is_some_and(|key| {
            self.entries
                .iter()
                .any(|(entry, weight)| *entry == key && *weight >= WITHHELD_AT)
        })
    }

    fn key(&self, scope: &CarryScope, before: &str, text: &str) -> Option<u64> {
        let word = next_word(text)?;
        // A suggestion's leading space belongs to its spot: after `Hello`,
        // ` revision` is at the same spot as `revision` after `Hello `.
        let spaces = text.len() - text.trim_start().len();
        let spot = spot(&format!("{before}{}", &text[..spaces]));
        Some(self.hasher.hash_one((format!("{scope:?}"), spot, word)))
    }
}

/// The last words before the caret, case- and spacing-insensitive, and
/// whether a space follows them (a word started, or one to start).
fn spot(before: &str) -> (String, bool) {
    let mut words: Vec<&str> = before.split_whitespace().rev().take(SPOT_WORDS).collect();
    words.reverse();
    (
        words.join(" ").to_lowercase(),
        before.ends_with(char::is_whitespace),
    )
}

/// The first word of `text`, lowercased and without closing punctuation.
pub(super) fn next_word(text: &str) -> Option<String> {
    let word = text
        .split_whitespace()
        .next()?
        .trim_end_matches(|c: char| !c.is_alphanumeric());
    (!word.is_empty()).then(|| word.to_lowercase())
}

#[cfg(test)]
mod tests;
