//! Private settings: the strict `badi.settings.v2` document, subject identities
//! and permissions, policy resolution, and owner-only storage with
//! compare-and-swap replacement.

mod document;
mod identity;
mod paths;
mod permissions;
mod policy;
mod private_file;
mod store;
#[cfg(test)]
mod tests;

pub use document::{
    BatteryMode, DEFAULT_WRITING_LANGUAGES, PredictionLanguage, SettingsV2, SettingsValidationError,
};
pub use identity::{BrowserAdapter, IdentityError, LinuxAdapter, StableIdentity, WebScheme};
pub(crate) use paths::PrivateStorage;
pub use paths::StoragePaths;
pub use permissions::{
    ALL_LINUX_APPS_PERMISSIONS, ALL_WEB_ORIGINS_PERMISSIONS, PermissionDecision,
    RetentionPermission, SubjectPermissions, SubjectRule,
};
pub use policy::{PolicyResolution, PolicySource};
pub use private_file::PrivateStorageError;
pub(crate) use private_file::{
    atomic_write_private, ensure_private_directory, read_private_limited, remove_private_file,
    remove_private_temporary_files,
};
pub(crate) use store::SettingsStore;
pub use store::SettingsStoreError;

pub const SETTINGS_SCHEMA_V1: &str = "badi.settings.v1";
pub const SETTINGS_SCHEMA: &str = "badi.settings.v2";
pub const MAX_SETTINGS_BYTES: usize = 65_536;
// Proven by a maximum-shape regression to fit both the 64 KiB wire frame and
// the private settings file, including full-length origin identities.
pub const MAX_SUBJECTS: usize = 64;
pub const MAX_RETENTION_DAYS: u16 = 90;
/// The longest timed Escape quiet a settings document may name.
pub const MAX_ESCAPE_QUIET_SECONDS: u64 = 3_600;
