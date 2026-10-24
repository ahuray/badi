//! Per-subject permissions and the defaults of the list modes.

use serde::{Deserialize, Serialize};

use super::MAX_RETENTION_DAYS;
use super::document::SettingsValidationError;
use super::identity::StableIdentity;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionDecision {
    Allow,
    Block,
}

impl PermissionDecision {
    #[must_use]
    pub const fn is_allowed(self) -> bool {
        matches!(self, Self::Allow)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum RetentionPermission {
    None,
    Bounded { days: u16 },
}

impl RetentionPermission {
    #[must_use]
    pub const fn days(self) -> Option<u16> {
        match self {
            Self::None => None,
            Self::Bounded { days } => Some(days),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectPermissions {
    pub suggest: PermissionDecision,
    pub display: PermissionDecision,
    pub context_read: PermissionDecision,
    pub learn: PermissionDecision,
    pub retention: RetentionPermission,
}

impl SubjectPermissions {
    #[must_use]
    pub const fn deny_all() -> Self {
        Self {
            suggest: PermissionDecision::Block,
            display: PermissionDecision::Block,
            context_read: PermissionDecision::Block,
            learn: PermissionDecision::Block,
            retention: RetentionPermission::None,
        }
    }

    pub fn validate(self) -> Result<(), SettingsValidationError> {
        if self.suggest.is_allowed()
            && (!self.display.is_allowed() || !self.context_read.is_allowed())
        {
            return Err(SettingsValidationError::PermissionDependency);
        }
        if self.learn.is_allowed()
            && (!self.suggest.is_allowed()
                || !self.display.is_allowed()
                || !self.context_read.is_allowed())
        {
            return Err(SettingsValidationError::PermissionDependency);
        }
        if let Some(days) = self.retention.days() {
            if !self.learn.is_allowed() || !(1..=MAX_RETENTION_DAYS).contains(&days) {
                return Err(SettingsValidationError::InvalidRetention);
            }
        }
        Ok(())
    }
}

impl Default for SubjectPermissions {
    fn default() -> Self {
        Self::deny_all()
    }
}

/// Permissions of an http(s) origin, or a Linux app, that has no exact rule
/// while `all_web_origins`, or `all_linux_apps`, is on: prediction only, never
/// learning or retention.
pub const ALL_WEB_ORIGINS_PERMISSIONS: SubjectPermissions = SubjectPermissions {
    suggest: PermissionDecision::Allow,
    display: PermissionDecision::Allow,
    context_read: PermissionDecision::Allow,
    learn: PermissionDecision::Block,
    retention: RetentionPermission::None,
};
pub const ALL_LINUX_APPS_PERMISSIONS: SubjectPermissions = ALL_WEB_ORIGINS_PERMISSIONS;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectRule {
    pub identity: StableIdentity,
    pub permissions: SubjectPermissions,
}
