//! Type-through (ADR 0004): typing the next characters of a shown suggestion
//! keeps its remainder.
//!
//! The carry is text, never authority. Observed fields open a new session for
//! every edit, so the remainder can only be offered to the next request that
//! passed every admission check, as a new suggestion bound to that request's
//! context. It keeps the shown suggestion's deadline, so typing never extends
//! how long derived text stays on screen, and nothing is kept past it.

use std::time::{Duration, Instant};

use super::SessionState;
use crate::protocol::{AdapterKind, ContextChangedPayload, Origin, TargetKind};
use crate::segment::{sanitize_suggestion, validate_completion_shape};

/// A carried remainder is shown only with at least this much reading time left.
const MIN_CARRIED_TTL: Duration = Duration::from_millis(500);
/// Adapters bound `before`; once its start slides with typing, at least this
/// much of the earlier text must still match exactly.
const MIN_SLID_SHARED_BYTES: usize = 128;

/// The app or site a carry belongs to. Observed fields get a new target id
/// for every edit, so the id itself cannot scope it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct CarryScope {
    adapter_kind: AdapterKind,
    kind: TargetKind,
    app_id: String,
    origin: Option<Origin>,
}

impl CarryScope {
    pub(super) fn of(session: &SessionState) -> Self {
        let target = &session.target.target;
        Self {
            adapter_kind: session.authority.adapter_kind,
            kind: target.kind,
            app_id: target.app_id.clone(),
            origin: target.origin.clone(),
        }
    }
}

/// The most recently shown continuation, while its deadline lasts.
pub(super) struct TypeThrough {
    pub(super) scope: CarryScope,
    pub(super) before: String,
    pub(super) after: String,
    pub(super) language: Option<String>,
    pub(super) text: String,
    pub(super) expires_at: Instant,
    /// The day of the `Shown` aggregate the original suggestion recorded.
    pub(super) aggregate_day: Option<u64>,
}

impl TypeThrough {
    /// The rest of the suggestion when `context` is this carry's context plus
    /// a typed, non-empty proper prefix of its text, in the same app or site,
    /// with enough reading time left and a valid continuation shape.
    pub(super) fn remainder(
        &self,
        scope: &CarryScope,
        context: &ContextChangedPayload,
        now: Instant,
    ) -> Option<String> {
        if *scope != self.scope
            || context.after != self.after
            || context.language != self.language
            || self.expires_at < now + MIN_CARRIED_TTL
        {
            return None;
        }
        let typed = typed_prefix(&self.before, &context.before, &self.text)?;
        let remainder = &self.text[typed..];
        let remainder = sanitize_suggestion(remainder).ok()?;
        // The user typed the start of the word themselves, so the remainder
        // may continue it; every other spacing and overlap check applies.
        validate_completion_shape(&context.before, &context.after, &remainder, true).ok()?;
        Some(remainder)
    }
}

/// The byte length of the suggestion's prefix that `now_before` adds to
/// `then_before`. The adapter's window may have dropped leading text since.
fn typed_prefix(then_before: &str, now_before: &str, text: &str) -> Option<usize> {
    (1..text.len())
        .filter(|&end| text.is_char_boundary(end))
        .find(|&end| {
            now_before.strip_suffix(&text[..end]).is_some_and(|base| {
                base == then_before
                    || (base.len() >= MIN_SLID_SHARED_BYTES && then_before.ends_with(base))
            })
        })
}

#[cfg(test)]
mod tests {
    use super::{CarryScope, MIN_SLID_SHARED_BYTES, TypeThrough, typed_prefix};
    use crate::protocol::{
        Activation, AdapterKind, ContextChangedPayload, FieldDescriptor, FieldPurpose, OffsetUnit,
        Selection, TargetKind,
    };
    use std::time::{Duration, Instant};

    fn scope(app_id: &str) -> CarryScope {
        CarryScope {
            adapter_kind: AdapterKind::Fcitx,
            kind: TargetKind::DesktopApplication,
            app_id: app_id.to_owned(),
            origin: None,
        }
    }

    fn carry(before: &str, text: &str, expires_in: Duration) -> TypeThrough {
        TypeThrough {
            scope: scope("org.telegram.desktop"),
            before: before.to_owned(),
            after: String::new(),
            language: Some("en".to_owned()),
            text: text.to_owned(),
            expires_at: Instant::now() + expires_in,
            aggregate_day: None,
        }
    }

    fn context(before: &str) -> ContextChangedPayload {
        ContextChangedPayload {
            fingerprint: "fingerprint_0000000000000001".to_owned(),
            before: before.to_owned(),
            after: String::new(),
            selection: Selection {
                anchor: 0,
                head: 0,
                unit: OffsetUnit::Utf16CodeUnits,
            },
            field: FieldDescriptor {
                purpose: FieldPurpose::Normal,
                editable: true,
                multiline: true,
                composing: false,
                sensitive: false,
                identity_known: true,
                focused: true,
                lock_screen: false,
            },
            activation: Activation::Always,
            explicit: false,
            language: Some("en".to_owned()),
        }
    }

    #[test]
    fn typed_characters_of_the_suggestion_carry_its_remainder() {
        let shown = carry(
            "Please find attached the",
            " report for",
            Duration::from_secs(5),
        );
        let here = scope("org.telegram.desktop");
        let now = Instant::now();
        for (typed, rest) in [
            (" ", "report for"),
            (" re", "port for"),
            (" report", " for"),
            (" report f", "or"),
        ] {
            let context = context(&format!("Please find attached the{typed}"));
            assert_eq!(shown.remainder(&here, &context, now).as_deref(), Some(rest));
        }
    }

    #[test]
    fn anything_but_an_exact_typed_prefix_carries_nothing() {
        let shown = carry(
            "Please find attached the",
            " report for",
            Duration::from_secs(5),
        );
        let here = scope("org.telegram.desktop");
        let now = Instant::now();
        for before in [
            "Please find attached the",            // nothing typed
            "Please find attached the report for", // all of it typed
            "Please find attached the rep0",       // a different character
            "Please find attached the  ",          // an extra space
            "please find attached the re",         // an edit before the caret
            "Please find attached them re",        // an edit at the old caret
        ] {
            assert_eq!(
                shown.remainder(&here, &context(before), now),
                None,
                "{before:?}"
            );
        }

        let mut after_changed = context("Please find attached the re");
        after_changed.after = "x".to_owned();
        assert_eq!(shown.remainder(&here, &after_changed, now), None);
        let mut language_changed = context("Please find attached the re");
        language_changed.language = Some("de".to_owned());
        assert_eq!(shown.remainder(&here, &language_changed, now), None);
        assert_eq!(
            shown.remainder(
                &scope("org.other.app"),
                &context("Please find attached the re"),
                now
            ),
            None
        );
    }

    #[test]
    fn a_nearly_expired_suggestion_carries_nothing() {
        let shown = carry("Thanks for the", " update", Duration::from_millis(300));
        assert_eq!(
            shown.remainder(
                &scope("org.telegram.desktop"),
                &context("Thanks for the up"),
                Instant::now()
            ),
            None
        );
    }

    #[test]
    fn a_sliding_window_needs_a_long_exact_shared_tail() {
        let long = "x".repeat(MIN_SLID_SHARED_BYTES + 10);
        assert_eq!(
            typed_prefix(&format!("ab{long}"), &format!("b{long} n"), " next"),
            Some(2)
        );
        assert_eq!(typed_prefix("ab short", "b short n", " next"), None);
        assert_eq!(typed_prefix("Hello", "Hello wor", " world"), Some(4));
        // Lengths are bytes of whole characters.
        assert_eq!(typed_prefix("سلام", "سلام د", " دوست"), Some(3));
    }
}
