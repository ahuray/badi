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
use crate::metrics::CarryMiss;
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
    /// A desktop app's scope, for tests of what is kept per app.
    #[cfg(test)]
    pub(super) fn desktop_app(app_id: &str) -> Self {
        Self {
            adapter_kind: AdapterKind::Fcitx,
            kind: TargetKind::DesktopApplication,
            app_id: app_id.to_owned(),
            origin: None,
        }
    }

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
    /// Typing other letters at it already counted it as turned down once.
    pub(super) turned_down: bool,
}

impl TypeThrough {
    /// The rest of the suggestion when `context` is this carry's context plus
    /// a typed, non-empty proper prefix of its text, in the same app or site,
    /// with enough reading time left and a valid continuation shape. The
    /// error names the first gate that declined, for content-free metrics.
    ///
    /// An automatic request for exactly this carry's context keeps the whole
    /// text: an observed field is re-admitted under a new session when its
    /// epoch changes, with nothing typed since, and the suggestion on screen
    /// must not be replaced by another model call. An explicit request for
    /// that context asks the model.
    pub(super) fn remainder(
        &self,
        scope: &CarryScope,
        context: &ContextChangedPayload,
        now: Instant,
        explicit: bool,
    ) -> Result<String, CarryMiss> {
        if *scope != self.scope {
            return Err(CarryMiss::Scope);
        }
        if context.after != self.after {
            return Err(CarryMiss::AfterChanged);
        }
        if context.language != self.language {
            return Err(CarryMiss::LanguageChanged);
        }
        if self.expires_at < now + MIN_CARRIED_TTL {
            return Err(CarryMiss::DeadlineTight);
        }
        let typed = if !explicit && context.before == self.before {
            0
        } else {
            typed_prefix(&self.before, &context.before, &self.text)
                .ok_or(CarryMiss::PrefixMismatch)?
        };
        let remainder = &self.text[typed..];
        let remainder = sanitize_suggestion(remainder).map_err(|_| CarryMiss::ShapeRejected)?;
        // The user typed the start of the word themselves, so the remainder
        // may continue it; every other spacing and overlap check applies.
        validate_completion_shape(&context.before, &context.after, &remainder, true)
            .map_err(|_| CarryMiss::ShapeRejected)?;
        Ok(remainder)
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
mod tests;
