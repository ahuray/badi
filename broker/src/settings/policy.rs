//! How a document resolves one target or identity to permissions.

use super::permissions::SubjectPermissions;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyResolution {
    pub settings_revision: u64,
    pub paused: bool,
    pub identity_known: bool,
    pub source: PolicySource,
    pub permissions: SubjectPermissions,
}

/// Where a resolution's permissions come from.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicySource {
    /// No rule and no list-mode default: deny.
    Unconfigured,
    /// An exact subject rule.
    Rule,
    /// The site or app blocklist mode's default for an unlisted subject.
    ListDefault,
}

impl PolicyResolution {
    pub(super) const fn unknown(settings_revision: u64, paused: bool) -> Self {
        Self {
            settings_revision,
            paused,
            identity_known: false,
            source: PolicySource::Unconfigured,
            permissions: SubjectPermissions::deny_all(),
        }
    }

    /// An exact rule or a list-mode default decided the permissions.
    #[must_use]
    pub const fn configured(self) -> bool {
        !matches!(self.source, PolicySource::Unconfigured)
    }

    /// Allowed by a list-mode default, not by an exact rule.
    #[must_use]
    pub const fn by_default(self) -> bool {
        matches!(self.source, PolicySource::ListDefault)
    }

    #[must_use]
    pub const fn allows_context_read(self) -> bool {
        !self.paused && self.permissions.context_read.is_allowed()
    }

    #[must_use]
    pub const fn allows_suggestion(self) -> bool {
        !self.paused && self.permissions.suggest.is_allowed()
    }

    #[must_use]
    pub const fn allows_display(self) -> bool {
        !self.paused && self.permissions.display.is_allowed()
    }

    #[must_use]
    pub const fn allows_learning(self) -> bool {
        !self.paused && self.permissions.learn.is_allowed()
    }
}
