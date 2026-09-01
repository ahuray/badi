use std::cmp::Ordering;
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::unix::fs::{
    DirBuilderExt as _, MetadataExt as _, OpenOptionsExt as _, PermissionsExt as _,
};
use std::path::{Path, PathBuf};
use std::str::FromStr as _;
use std::sync::{Arc, Mutex};

use rustix::fs::{FlockOperation, flock};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::protocol::{
    MAX_SAFE_COUNTER, OriginScheme, TargetDescriptor, TargetKind, valid_linux_app_id,
};

pub const SETTINGS_SCHEMA_V1: &str = "badi.settings.v1";
pub const SETTINGS_SCHEMA: &str = "badi.settings.v2";
pub const MAX_SETTINGS_BYTES: usize = 65_536;
// Proven by a maximum-shape regression to fit both the 64 KiB wire frame and
// the private settings file, including full-length origin identities.
pub const MAX_SUBJECTS: usize = 64;
pub const MAX_RETENTION_DAYS: u16 = 90;

const SETTINGS_FILE_NAME: &str = "settings.json";
const PERSONALIZATION_FILE_NAME: &str = "personalization.json";
const STORE_LOCK_FILE_NAME: &str = "store.lock";
const PRIVATE_DIRECTORY_MODE: u32 = 0o700;
const PRIVATE_FILE_MODE: u32 = 0o600;
const TEMPORARY_NONCE_HEX_LENGTH: usize = 32;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAdapter {
    Chromium,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinuxAdapter {
    Fcitx,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WebScheme {
    Http,
    Https,
}

impl WebScheme {
    #[must_use]
    pub const fn default_port(self) -> u16 {
        match self {
            Self::Http => 80,
            Self::Https => 443,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum StableIdentity {
    BrowserOrigin {
        adapter: BrowserAdapter,
        scheme: WebScheme,
        host: String,
        port: u16,
    },
    LinuxApp {
        adapter: LinuxAdapter,
        app_id: String,
    },
}

impl StableIdentity {
    pub fn browser_origin(
        adapter: BrowserAdapter,
        scheme: WebScheme,
        host: &str,
        port: Option<u16>,
    ) -> Result<Self, IdentityError> {
        let host = canonical_host(host)?;
        let port = port.unwrap_or_else(|| scheme.default_port());
        if port == 0 {
            return Err(IdentityError::InvalidPort);
        }
        Ok(Self::BrowserOrigin {
            adapter,
            scheme,
            host,
            port,
        })
    }

    pub fn linux_app(adapter: LinuxAdapter, app_id: &str) -> Result<Self, IdentityError> {
        if !valid_linux_app_id(app_id) {
            return Err(IdentityError::InvalidAppId);
        }
        Ok(Self::LinuxApp {
            adapter,
            app_id: app_id.to_owned(),
        })
    }

    pub fn from_target(target: &TargetDescriptor) -> Result<Self, IdentityError> {
        match target.kind {
            TargetKind::Browser if target.app_id == "chromium" => {
                let origin = target.origin.as_ref().ok_or(IdentityError::MissingOrigin)?;
                let scheme = match origin.scheme {
                    OriginScheme::Http => WebScheme::Http,
                    OriginScheme::Https => WebScheme::Https,
                    OriginScheme::ChromeExtension
                    | OriginScheme::MozExtension
                    | OriginScheme::File => return Err(IdentityError::UnsupportedScheme),
                };
                Self::browser_origin(BrowserAdapter::Chromium, scheme, &origin.host, origin.port)
            }
            TargetKind::DesktopApplication if target.origin.is_none() => {
                Self::linux_app(LinuxAdapter::Fcitx, &target.app_id)
            }
            _ => Err(IdentityError::UnsupportedTarget),
        }
    }

    pub fn validate(&self) -> Result<(), IdentityError> {
        match self {
            Self::BrowserOrigin {
                adapter,
                scheme,
                host,
                port,
            } => {
                let canonical = Self::browser_origin(*adapter, *scheme, host, Some(*port))?;
                if &canonical == self {
                    Ok(())
                } else {
                    Err(IdentityError::NoncanonicalHost)
                }
            }
            Self::LinuxApp { adapter, app_id } => {
                let canonical = Self::linux_app(*adapter, app_id)?;
                if &canonical == self {
                    Ok(())
                } else {
                    Err(IdentityError::InvalidAppId)
                }
            }
        }
    }
}

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SubjectRule {
    pub identity: StableIdentity,
    pub permissions: SubjectPermissions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsV2 {
    pub schema: String,
    pub revision: u64,
    pub paused: bool,
    pub subjects: Vec<SubjectRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    schema: String,
    revision: u64,
    paused: bool,
    subjects: Vec<SubjectRule>,
}

impl<'de> Deserialize<'de> for SettingsV2 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let decoded = SettingsDocument::deserialize(deserializer)?;
        if decoded.schema != SETTINGS_SCHEMA && decoded.schema != SETTINGS_SCHEMA_V1 {
            return Err(serde::de::Error::custom("unsupported_settings_schema"));
        }
        if decoded.schema == SETTINGS_SCHEMA_V1
            && decoded
                .subjects
                .iter()
                .any(|rule| matches!(rule.identity, StableIdentity::LinuxApp { .. }))
        {
            return Err(serde::de::Error::custom("linux_app_requires_settings_v2"));
        }
        Ok(Self {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: decoded.revision,
            paused: decoded.paused,
            subjects: decoded.subjects,
        })
    }
}

impl SettingsV2 {
    #[must_use]
    pub fn deny_by_default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 0,
            paused: true,
            subjects: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), SettingsValidationError> {
        if self.schema != SETTINGS_SCHEMA {
            return Err(SettingsValidationError::UnsupportedSchema);
        }
        if self.revision > MAX_SAFE_COUNTER {
            return Err(SettingsValidationError::RevisionOutOfRange);
        }
        if self.subjects.len() > MAX_SUBJECTS {
            return Err(SettingsValidationError::TooManySubjects);
        }

        let mut previous: Option<&StableIdentity> = None;
        for rule in &self.subjects {
            rule.identity.validate()?;
            rule.permissions.validate()?;
            if matches!(rule.identity, StableIdentity::LinuxApp { .. })
                && rule.permissions.learn.is_allowed()
            {
                return Err(SettingsValidationError::LinuxAppLearningUnsupported);
            }
            if let Some(identity) = previous {
                match identity.cmp(&rule.identity) {
                    Ordering::Equal => return Err(SettingsValidationError::DuplicateIdentity),
                    Ordering::Greater => {
                        return Err(SettingsValidationError::SubjectsNotCanonical);
                    }
                    Ordering::Less => {}
                }
            }
            previous = Some(&rule.identity);
        }
        Ok(())
    }

    #[must_use]
    pub fn resolve_identity(&self, identity: &StableIdentity) -> PolicyResolution {
        if self.validate().is_err() || identity.validate().is_err() {
            return PolicyResolution::unknown(self.revision, self.paused);
        }
        self.resolve_identity_validated(identity)
    }

    #[must_use]
    pub(crate) fn resolve_identity_validated(&self, identity: &StableIdentity) -> PolicyResolution {
        match self
            .subjects
            .binary_search_by(|rule| rule.identity.cmp(identity))
        {
            Ok(index) => PolicyResolution {
                settings_revision: self.revision,
                paused: self.paused,
                identity_known: true,
                configured: true,
                permissions: self.subjects[index].permissions,
            },
            Err(_) => PolicyResolution {
                settings_revision: self.revision,
                paused: self.paused,
                identity_known: true,
                configured: false,
                permissions: SubjectPermissions::deny_all(),
            },
        }
    }

    #[must_use]
    pub fn resolve_target(&self, target: &TargetDescriptor) -> PolicyResolution {
        if self.validate().is_err() {
            return PolicyResolution::unknown(self.revision, self.paused);
        }
        self.resolve_target_validated(target)
    }

    #[must_use]
    pub(crate) fn resolve_target_validated(&self, target: &TargetDescriptor) -> PolicyResolution {
        StableIdentity::from_target(target).map_or_else(
            |_| PolicyResolution::unknown(self.revision, self.paused),
            |identity| self.resolve_identity_validated(&identity),
        )
    }

    pub fn wire_document(
        &self,
        protocol_version: u8,
    ) -> Result<serde_json::Value, serde_json::Error> {
        if protocol_version == crate::protocol::PROTOCOL_VERSION {
            let subjects = self
                .subjects
                .iter()
                .filter(|rule| matches!(rule.identity, StableIdentity::BrowserOrigin { .. }))
                .collect::<Vec<_>>();
            Ok(serde_json::json!({
                "schema": SETTINGS_SCHEMA_V1,
                "revision": self.revision,
                "paused": self.paused,
                "subjects": subjects,
            }))
        } else {
            serde_json::to_value(self)
        }
    }

    /// A legacy settings client can still manage its browser-origin slice, but
    /// cannot erase native policy that its schema is unable to represent.
    #[must_use]
    pub fn preserving_linux_rules_from(mut self, current: &Self) -> Self {
        self.subjects.extend(
            current
                .subjects
                .iter()
                .filter(|rule| matches!(rule.identity, StableIdentity::LinuxApp { .. }))
                .cloned(),
        );
        self.subjects
            .sort_by(|left, right| left.identity.cmp(&right.identity));
        self
    }
}

impl Default for SettingsV2 {
    fn default() -> Self {
        Self::deny_by_default()
    }
}

/// Compatibility name for existing broker and Chromium control-plane code.
/// Values deserialize legacy v1 documents but are always canonical settings v2
/// in memory and on disk.
pub type SettingsV1 = SettingsV2;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyResolution {
    pub settings_revision: u64,
    pub paused: bool,
    pub identity_known: bool,
    pub configured: bool,
    pub permissions: SubjectPermissions,
}

impl PolicyResolution {
    const fn unknown(settings_revision: u64, paused: bool) -> Self {
        Self {
            settings_revision,
            paused,
            identity_known: false,
            configured: false,
            permissions: SubjectPermissions::deny_all(),
        }
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoragePaths {
    config_dir: PathBuf,
    data_dir: PathBuf,
}

impl StoragePaths {
    pub fn from_environment() -> Result<Self, PrivateStorageError> {
        let home = env::var_os("HOME").map(PathBuf::from);
        let config_home = xdg_home("XDG_CONFIG_HOME", home.as_deref(), ".config")?;
        let data_home = xdg_home("XDG_DATA_HOME", home.as_deref(), ".local/share")?;
        Self::new(config_home.join("badi"), data_home.join("badi"))
    }

    pub fn new(
        config_dir: impl Into<PathBuf>,
        data_dir: impl Into<PathBuf>,
    ) -> Result<Self, PrivateStorageError> {
        let config_dir = config_dir.into();
        let data_dir = data_dir.into();
        if !config_dir.is_absolute() || !data_dir.is_absolute() {
            return Err(PrivateStorageError::PathNotAbsolute);
        }
        Ok(Self {
            config_dir,
            data_dir,
        })
    }

    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[must_use]
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    #[must_use]
    pub fn settings_path(&self) -> PathBuf {
        self.config_dir.join(SETTINGS_FILE_NAME)
    }

    #[must_use]
    pub fn personalization_path(&self) -> PathBuf {
        self.data_dir.join(PERSONALIZATION_FILE_NAME)
    }

    #[must_use]
    pub fn config_lock_path(&self) -> PathBuf {
        self.config_dir.join(STORE_LOCK_FILE_NAME)
    }

    #[must_use]
    pub fn data_lock_path(&self) -> PathBuf {
        self.data_dir.join(STORE_LOCK_FILE_NAME)
    }
}

#[derive(Debug)]
pub(crate) struct PrivateStorage {
    paths: StoragePaths,
    config_lock: Arc<StoreLock>,
    _data_lock: Arc<StoreLock>,
    mutations: Arc<Mutex<()>>,
}

impl PrivateStorage {
    pub(crate) fn open_from_environment() -> Result<Self, PrivateStorageError> {
        Self::open(StoragePaths::from_environment()?)
    }

    pub(crate) fn open(paths: StoragePaths) -> Result<Self, PrivateStorageError> {
        ensure_private_directory(paths.config_dir())?;
        ensure_private_directory(paths.data_dir())?;
        // Each independently mutable root needs its own lifetime lock. This
        // prevents custom path pairs from bypassing settings CAS by sharing a
        // config directory while selecting different data directories (and
        // prevents the inverse collision for personalization state).
        let config_lock = Arc::new(StoreLock::acquire(&paths.config_lock_path())?);
        let data_lock = if paths.config_lock_path() == paths.data_lock_path() {
            Arc::clone(&config_lock)
        } else {
            Arc::new(StoreLock::acquire(&paths.data_lock_path())?)
        };
        // A crash before the atomic rename can leave a complete private copy
        // beside the canonical file. Remove only names generated by Badi, and
        // do it while both lifetime locks exclude another Badi process. This
        // is part of the privacy boundary: canonical clear must not coexist
        // with a forgotten settings or personalization document.
        remove_private_temporary_files(&paths.settings_path())?;
        remove_private_temporary_files(&paths.personalization_path())?;
        // A previous process may have exited after a visible rename/unlink but
        // before its directory entry was durably synced. Startup is the only
        // recovery boundary for a settings commit-unknown state: while holding
        // both lifetime locks, durably adopt the namespace that is visible now
        // before loading it into live authority.
        sync_directory(paths.config_dir())?;
        if paths.data_dir() != paths.config_dir() {
            sync_directory(paths.data_dir())?;
        }
        Ok(Self {
            paths,
            config_lock,
            _data_lock: data_lock,
            mutations: Arc::new(Mutex::new(())),
        })
    }

    #[must_use]
    pub(crate) fn paths(&self) -> &StoragePaths {
        &self.paths
    }

    pub(crate) fn settings_store(&self) -> Result<SettingsStore, PrivateStorageError> {
        SettingsStore::new(
            self.paths.settings_path(),
            Arc::clone(&self.config_lock),
            Arc::clone(&self.mutations),
        )
    }
}

#[derive(Debug)]
pub(crate) struct StoreLock {
    _file: File,
}

impl StoreLock {
    fn acquire(path: &Path) -> Result<Self, PrivateStorageError> {
        let create = || {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(PRIVATE_FILE_MODE)
                .open(path)
        };
        let (file, created) = match create() {
            Ok(file) => (file, true),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let file = OpenOptions::new().read(true).write(true).open(path)?;
                (file, false)
            }
            Err(error) => return Err(error.into()),
        };
        if created {
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        }
        validate_open_private_file(path, &file, None)?;
        match flock(&file, FlockOperation::NonBlockingLockExclusive) {
            Ok(()) => Ok(Self { _file: file }),
            Err(error) if error == rustix::io::Errno::WOULDBLOCK => {
                Err(PrivateStorageError::LockUnavailable)
            }
            Err(error) => Err(io::Error::from_raw_os_error(error.raw_os_error()).into()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct SettingsStore {
    path: PathBuf,
    _lock: Arc<StoreLock>,
    mutation: Arc<Mutex<()>>,
}

impl SettingsStore {
    fn new(
        path: impl Into<PathBuf>,
        lock: Arc<StoreLock>,
        mutation: Arc<Mutex<()>>,
    ) -> Result<Self, PrivateStorageError> {
        let path = path.into();
        if !path.is_absolute() || path.parent().is_none() {
            return Err(PrivateStorageError::PathNotAbsolute);
        }
        Ok(Self {
            path,
            _lock: lock,
            mutation,
        })
    }

    #[must_use]
    #[cfg(test)]
    fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn load_or_initialize(&self) -> Result<SettingsV1, SettingsStoreError> {
        let _guard = self
            .mutation
            .lock()
            .map_err(|_| SettingsStoreError::LockPoisoned)?;
        if let Some(bytes) = read_private_limited(&self.path, MAX_SETTINGS_BYTES)? {
            let (settings, migrated) = decode_settings_with_source(&bytes)?;
            if migrated {
                // The existing lifetime and mutation locks make this a single
                // writer migration. atomic_write_private keeps either the old
                // complete v1 document or the new complete v2 document visible.
                write_settings(&self.path, &settings)?;
            }
            Ok(settings)
        } else {
            let settings = SettingsV1::deny_by_default();
            write_settings(&self.path, &settings)?;
            Ok(settings)
        }
    }

    #[cfg(test)]
    fn load(&self) -> Result<Option<SettingsV1>, SettingsStoreError> {
        read_private_limited(&self.path, MAX_SETTINGS_BYTES)?
            .map(|bytes| decode_settings(&bytes))
            .transpose()
    }

    pub(crate) fn compare_and_replace(
        &self,
        expected_revision: u64,
        next: SettingsV1,
    ) -> Result<SettingsV1, SettingsStoreError> {
        self.compare_and_replace_with_writer(expected_revision, &next, write_settings)?;
        Ok(next)
    }

    fn compare_and_replace_with_writer<F>(
        &self,
        expected_revision: u64,
        next: &SettingsV1,
        writer: F,
    ) -> Result<SettingsV1, SettingsStoreError>
    where
        F: FnOnce(&Path, &SettingsV1) -> Result<(), SettingsStoreError>,
    {
        next.validate()?;
        let required_revision = expected_revision
            .checked_add(1)
            .filter(|revision| *revision <= MAX_SAFE_COUNTER)
            .ok_or(SettingsValidationError::RevisionOutOfRange)?;
        if next.revision != required_revision {
            return Err(SettingsStoreError::NextRevision {
                expected: required_revision,
                actual: next.revision,
            });
        }

        let _guard = self
            .mutation
            .lock()
            .map_err(|_| SettingsStoreError::LockPoisoned)?;
        let current = match read_private_limited(&self.path, MAX_SETTINGS_BYTES)? {
            Some(bytes) => decode_settings(&bytes)?,
            None => SettingsV1::deny_by_default(),
        };
        if current.revision != expected_revision {
            return Err(SettingsStoreError::RevisionConflict {
                expected: expected_revision,
                actual: current.revision,
            });
        }
        match writer(&self.path, next) {
            Ok(()) => Ok(next.clone()),
            Err(write_error) => {
                if matches!(
                    &write_error,
                    SettingsStoreError::Storage(PrivateStorageError::CommitStateUnknown)
                ) {
                    return Err(SettingsStoreError::CommitStateUnknown);
                }
                // Atomic replacement can fail after rename (for example while
                // syncing the directory). Re-read under the same mutation lock
                // before deciding whether old authority may be restored.
                // Exact new bytes mean the replacement is authoritative now;
                // exact old bytes mean it was rejected. Anything else is
                // commit-unknown and must keep the broker fail-closed.
                let observed = read_private_limited(&self.path, MAX_SETTINGS_BYTES)
                    .map_err(SettingsStoreError::from)
                    .and_then(|bytes| bytes.map(|value| decode_settings(&value)).transpose());
                match observed {
                    Ok(Some(document)) if document == *next => Ok(next.clone()),
                    Ok(Some(document)) if document == current => Err(write_error),
                    Ok(None) if current == SettingsV1::deny_by_default() => Err(write_error),
                    _ => Err(SettingsStoreError::CommitStateUnknown),
                }
            }
        }
    }

    pub(crate) fn preflight_replace(next: &SettingsV1) -> Result<(), SettingsStoreError> {
        let _ = encode_settings(next)?;
        Ok(())
    }
}

fn decode_settings(bytes: &[u8]) -> Result<SettingsV1, SettingsStoreError> {
    decode_settings_with_source(bytes).map(|(settings, _)| settings)
}

fn decode_settings_with_source(bytes: &[u8]) -> Result<(SettingsV1, bool), SettingsStoreError> {
    #[derive(Deserialize)]
    struct SchemaProbe {
        schema: String,
    }

    let source = serde_json::from_slice::<SchemaProbe>(bytes)?;
    let settings: SettingsV1 = serde_json::from_slice(bytes)?;
    settings.validate()?;
    Ok((settings, source.schema == SETTINGS_SCHEMA_V1))
}

fn write_settings(path: &Path, settings: &SettingsV1) -> Result<(), SettingsStoreError> {
    let bytes = encode_settings(settings)?;
    atomic_write_private(path, &bytes)?;
    Ok(())
}

fn encode_settings(settings: &SettingsV1) -> Result<Vec<u8>, SettingsStoreError> {
    settings.validate()?;
    let mut bytes = serde_json::to_vec_pretty(settings)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_SETTINGS_BYTES {
        return Err(SettingsStoreError::EncodedTooLarge(bytes.len()));
    }
    Ok(bytes)
}

fn xdg_home(
    variable: &'static str,
    home: Option<&Path>,
    fallback: &str,
) -> Result<PathBuf, PrivateStorageError> {
    let candidate = env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|path| path.join(fallback)))
        .ok_or(PrivateStorageError::MissingHome(variable))?;
    if candidate.is_absolute() {
        Ok(candidate)
    } else {
        Err(PrivateStorageError::PathNotAbsolute)
    }
}

fn canonical_host(raw: &str) -> Result<String, IdentityError> {
    if raw.is_empty() || raw.len() > 253 || !raw.is_ascii() {
        return Err(IdentityError::InvalidHost);
    }
    let without_brackets = raw
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(raw);
    if let Ok(address) = Ipv4Addr::from_str(without_brackets) {
        return Ok(address.to_string());
    }
    if let Ok(address) = Ipv6Addr::from_str(without_brackets) {
        return Ok(address.to_string());
    }
    let normalized = without_brackets.to_ascii_lowercase();
    let ambiguous_hex_address = normalized.strip_prefix("0x").is_some_and(|digits| {
        !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_hexdigit())
    });
    if normalized
        .bytes()
        .all(|byte| byte.is_ascii_digit() || byte == b'.')
        || ambiguous_hex_address
    {
        return Err(IdentityError::InvalidHost);
    }
    if normalized.ends_with('.')
        || normalized.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        })
    {
        return Err(IdentityError::InvalidHost);
    }
    Ok(normalized)
}

pub(crate) fn ensure_private_directory(path: &Path) -> Result<(), PrivateStorageError> {
    if !path.is_absolute() {
        return Err(PrivateStorageError::PathNotAbsolute);
    }
    match fs::symlink_metadata(path) {
        Ok(metadata) => return validate_private_directory_metadata(&metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    fs::create_dir_all(parent)?;
    let mut builder = fs::DirBuilder::new();
    builder.mode(PRIVATE_DIRECTORY_MODE);
    match builder.create(path) {
        Ok(()) => {
            fs::set_permissions(path, fs::Permissions::from_mode(PRIVATE_DIRECTORY_MODE))?;
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error.into()),
    }
    let metadata = fs::symlink_metadata(path)?;
    validate_private_directory_metadata(&metadata)
}

fn validate_private_directory_metadata(metadata: &fs::Metadata) -> Result<(), PrivateStorageError> {
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o777 != PRIVATE_DIRECTORY_MODE
    {
        return Err(PrivateStorageError::UnsafeDirectory);
    }
    Ok(())
}

pub(crate) fn read_private_limited(
    path: &Path,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, PrivateStorageError> {
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    validate_private_file_metadata(&link_metadata, Some(max_bytes))?;
    let mut file = File::open(path)?;
    validate_open_private_file(path, &file, Some(max_bytes))?;

    let mut bytes = Vec::with_capacity(
        usize::try_from(file.metadata()?.len())
            .unwrap_or(max_bytes)
            .min(max_bytes),
    );
    Read::by_ref(&mut file)
        .take(
            u64::try_from(max_bytes)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(PrivateStorageError::FileTooLarge(bytes.len()));
    }
    Ok(Some(bytes))
}

fn validate_open_private_file(
    path: &Path,
    file: &File,
    max_bytes: Option<usize>,
) -> Result<(), PrivateStorageError> {
    let opened = file.metadata()?;
    validate_private_file_metadata(&opened, max_bytes)?;
    let current = fs::symlink_metadata(path)?;
    validate_private_file_metadata(&current, max_bytes)?;
    if opened.dev() != current.dev() || opened.ino() != current.ino() {
        return Err(PrivateStorageError::FileChanged);
    }
    Ok(())
}

fn validate_private_file_metadata(
    metadata: &fs::Metadata,
    max_bytes: Option<usize>,
) -> Result<(), PrivateStorageError> {
    if !metadata.is_file()
        || metadata.uid() != rustix::process::getuid().as_raw()
        || metadata.mode() & 0o777 != PRIVATE_FILE_MODE
        || metadata.nlink() != 1
    {
        return Err(PrivateStorageError::UnsafeFile);
    }
    if max_bytes.is_some_and(|limit| metadata.len() > u64::try_from(limit).unwrap_or(u64::MAX)) {
        return Err(PrivateStorageError::FileTooLarge(
            usize::try_from(metadata.len()).unwrap_or(usize::MAX),
        ));
    }
    Ok(())
}

pub(crate) fn atomic_write_private(path: &Path, bytes: &[u8]) -> Result<(), PrivateStorageError> {
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    ensure_private_directory(parent)?;
    match fs::symlink_metadata(path) {
        Ok(existing) => validate_private_file_metadata(&existing, None)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PrivateStorageError::InvalidFileName)?;
    let temporary = parent.join(format!(
        ".{file_name}.{}.tmp",
        uuid::Uuid::new_v4().as_simple()
    ));
    let mut temporary_created = false;
    let mut renamed = false;
    let mut directory_synced = false;
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(PRIVATE_FILE_MODE)
            .open(&temporary)?;
        temporary_created = true;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(PRIVATE_FILE_MODE))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        validate_open_private_file(&temporary, &file, None)?;
        fs::rename(&temporary, path)?;
        renamed = true;
        sync_directory(parent)?;
        directory_synced = true;
        let written = File::open(path)?;
        validate_open_private_file(path, &written, None)
    })();
    if let Err(error) = result {
        if !renamed {
            if temporary_created && remove_private_file(&temporary).is_err() {
                return Err(PrivateStorageError::CommitStateUnknown);
            }
            return Err(error);
        }
        // The rename is the commit point. If a later durability or validation
        // step fails, recover only when the safely re-opened destination is
        // byte-for-byte the intended document. Otherwise callers must assume
        // either version may be authoritative and remain fail-closed.
        return match read_private_limited(path, bytes.len()) {
            Ok(Some(observed))
                if observed == bytes && (directory_synced || sync_directory(parent).is_ok()) =>
            {
                Ok(())
            }
            _ => Err(PrivateStorageError::CommitStateUnknown),
        };
    }
    Ok(())
}

/// Removes only complete-document temporary files created for `path`.
///
/// The caller must hold the storage root's lifetime lock. An exact-looking but
/// unsafe entry is preserved and fails closed rather than being unlinked.
pub(crate) fn remove_private_temporary_files(path: &Path) -> Result<bool, PrivateStorageError> {
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    ensure_private_directory(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(PrivateStorageError::InvalidFileName)?;
    let prefix = format!(".{file_name}.");
    let suffix = ".tmp";
    let mut removed = false;
    let result = (|| {
        for entry in fs::read_dir(parent)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(nonce) = name
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(suffix))
            else {
                continue;
            };
            if nonce.len() != TEMPORARY_NONCE_HEX_LENGTH
                || !nonce
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                continue;
            }
            let temporary = entry.path();
            let metadata = fs::symlink_metadata(&temporary)?;
            validate_private_file_metadata(&metadata, None)?;
            fs::remove_file(&temporary)?;
            removed = true;
        }
        Ok(())
    })();
    if removed && sync_directory(parent).is_err() {
        return Err(PrivateStorageError::CommitStateUnknown);
    }
    result.map(|()| removed)
}

pub(crate) fn remove_private_file(path: &Path) -> Result<bool, PrivateStorageError> {
    remove_private_file_with_sync(path, sync_directory)
}

fn remove_private_file_with_sync<F>(
    path: &Path,
    mut sync_parent: F,
) -> Result<bool, PrivateStorageError>
where
    F: FnMut(&Path) -> Result<(), PrivateStorageError>,
{
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // Absence may be the visible result of an earlier unlink whose
            // directory sync failed. Require a successful namespace sync even
            // on retries so clear cannot acknowledge data that may resurrect
            // after a crash.
            let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
            return sync_parent(parent)
                .map(|()| false)
                .map_err(|_| PrivateStorageError::CommitStateUnknown);
        }
        Err(error) => return Err(error.into()),
    };
    validate_private_file_metadata(&metadata, None)?;
    fs::remove_file(path)?;
    let parent = path.parent().ok_or(PrivateStorageError::PathNotAbsolute)?;
    if sync_parent(parent).is_err() {
        return match fs::symlink_metadata(path) {
            Err(error)
                if error.kind() == io::ErrorKind::NotFound && sync_parent(parent).is_ok() =>
            {
                Ok(true)
            }
            _ => Err(PrivateStorageError::CommitStateUnknown),
        };
    }
    Ok(true)
}

fn sync_directory(path: &Path) -> Result<(), PrivateStorageError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum IdentityError {
    #[error("invalid_app_id")]
    InvalidAppId,
    #[error("invalid_host")]
    InvalidHost,
    #[error("invalid_port")]
    InvalidPort,
    #[error("missing_origin")]
    MissingOrigin,
    #[error("noncanonical_host")]
    NoncanonicalHost,
    #[error("unsupported_scheme")]
    UnsupportedScheme,
    #[error("unsupported_target")]
    UnsupportedTarget,
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SettingsValidationError {
    #[error("duplicate_identity")]
    DuplicateIdentity,
    #[error("identity")]
    Identity(#[from] IdentityError),
    #[error("invalid_retention")]
    InvalidRetention,
    #[error("linux_app_learning_unsupported")]
    LinuxAppLearningUnsupported,
    #[error("permission_dependency")]
    PermissionDependency,
    #[error("revision_out_of_range")]
    RevisionOutOfRange,
    #[error("subjects_not_canonical")]
    SubjectsNotCanonical,
    #[error("too_many_subjects")]
    TooManySubjects,
    #[error("unsupported_schema")]
    UnsupportedSchema,
}

#[derive(Debug, Error)]
pub enum PrivateStorageError {
    #[error("commit_state_unknown")]
    CommitStateUnknown,
    #[error("file_changed")]
    FileChanged,
    #[error("file_too_large:{0}")]
    FileTooLarge(usize),
    #[error("invalid_file_name")]
    InvalidFileName,
    #[error("io")]
    Io(#[from] io::Error),
    #[error("lock_unavailable")]
    LockUnavailable,
    #[error("missing_home:{0}")]
    MissingHome(&'static str),
    #[error("path_not_absolute")]
    PathNotAbsolute,
    #[error("unsafe_directory")]
    UnsafeDirectory,
    #[error("unsafe_file")]
    UnsafeFile,
}

#[derive(Debug, Error)]
pub enum SettingsStoreError {
    #[error("commit_state_unknown")]
    CommitStateUnknown,
    #[error("encoded_too_large:{0}")]
    EncodedTooLarge(usize),
    #[error("json")]
    Json(#[from] serde_json::Error),
    #[error("lock_poisoned")]
    LockPoisoned,
    #[error("next_revision:{expected}:{actual}")]
    NextRevision { expected: u64, actual: u64 },
    #[error("revision_conflict:{expected}:{actual}")]
    RevisionConflict { expected: u64, actual: u64 },
    #[error("storage")]
    Storage(#[from] PrivateStorageError),
    #[error("validation")]
    Validation(#[from] SettingsValidationError),
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt as _;

    use tempfile::tempdir;

    use super::{
        BrowserAdapter, LinuxAdapter, MAX_SETTINGS_BYTES, MAX_SUBJECTS, PRIVATE_FILE_MODE,
        PermissionDecision, PrivateStorage, PrivateStorageError, RetentionPermission,
        SETTINGS_SCHEMA, SETTINGS_SCHEMA_V1, SettingsStoreError, SettingsV1, StableIdentity,
        StoragePaths, SubjectPermissions, SubjectRule, WebScheme, remove_private_file_with_sync,
        write_settings,
    };
    use crate::protocol::{
        MAX_FRAME_BYTES, Origin, OriginScheme, TargetDescriptor, TargetKind, WireEnvelope,
    };

    fn allowed(days: Option<u16>) -> SubjectPermissions {
        SubjectPermissions {
            suggest: PermissionDecision::Allow,
            display: PermissionDecision::Allow,
            context_read: PermissionDecision::Allow,
            learn: PermissionDecision::Allow,
            retention: days.map_or(RetentionPermission::None, |days| {
                RetentionPermission::Bounded { days }
            }),
        }
    }

    fn identity(host: &str) -> StableIdentity {
        StableIdentity::browser_origin(BrowserAdapter::Chromium, WebScheme::Https, host, None)
            .expect("identity")
    }

    fn settings_with(rule: SubjectRule) -> SettingsV1 {
        SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 1,
            paused: false,
            subjects: vec![rule],
        }
    }

    #[test]
    fn missing_settings_are_paused_and_deny_everything() {
        let settings = SettingsV1::deny_by_default();
        settings.validate().expect("safe settings");
        let resolution = settings.resolve_identity(&identity("example.com"));
        assert!(resolution.identity_known);
        assert!(!resolution.configured);
        assert!(!resolution.allows_context_read());
        assert!(!resolution.allows_suggestion());
        assert!(!resolution.allows_display());
        assert!(!resolution.allows_learning());
    }

    #[test]
    fn browser_target_resolution_is_exact_and_canonical() {
        let target = TargetDescriptor {
            kind: TargetKind::Browser,
            app_id: "chromium".to_owned(),
            target_id: "opaque-session".to_owned(),
            origin: Some(Origin {
                scheme: OriginScheme::Https,
                host: "EXAMPLE.COM".to_owned(),
                port: None,
            }),
        };
        assert_eq!(
            StableIdentity::from_target(&target).expect("known browser origin"),
            identity("example.com")
        );

        for rejected in [
            TargetDescriptor {
                origin: None,
                ..target.clone()
            },
            TargetDescriptor {
                kind: TargetKind::Terminal,
                ..target.clone()
            },
            TargetDescriptor {
                app_id: "claimed-app".to_owned(),
                ..target.clone()
            },
        ] {
            assert!(StableIdentity::from_target(&rejected).is_err());
        }
    }

    #[test]
    fn desktop_target_resolution_uses_exact_canonical_fcitx_program_identity() {
        let target = TargetDescriptor {
            kind: TargetKind::DesktopApplication,
            app_id: "com.github.xournalpp.xournalpp".to_owned(),
            target_id: "ic:42".to_owned(),
            origin: None,
        };
        assert_eq!(
            StableIdentity::from_target(&target).expect("known Linux app"),
            StableIdentity::linux_app(LinuxAdapter::Fcitx, &target.app_id)
                .expect("canonical identity")
        );

        for app_id in [
            "",
            "Omawrite",
            "omawrite window",
            ".omawrite",
            "omawrite.",
            "omawrite.1editor",
            "omawrite._editor",
            "omawrite.-editor",
        ] {
            assert!(StableIdentity::linux_app(LinuxAdapter::Fcitx, app_id).is_err());
        }
        for app_id in [
            "omawrite",
            "com.github.xournalpp.xournalpp",
            "omawrite.editor_",
        ] {
            StableIdentity::linux_app(LinuxAdapter::Fcitx, app_id)
                .expect("schema-compatible Linux app identity");
        }
        let mut with_origin = target;
        with_origin.origin = Some(Origin {
            scheme: OriginScheme::Https,
            host: "example.com".to_owned(),
            port: None,
        });
        assert!(StableIdentity::from_target(&with_origin).is_err());
    }

    #[test]
    fn identity_rejects_ambiguous_hosts_and_normalizes_ip_literals() {
        for host in [
            "",
            ".example.com",
            "example.com.",
            "bad_label",
            "-bad.example",
        ] {
            assert!(
                StableIdentity::browser_origin(
                    BrowserAdapter::Chromium,
                    WebScheme::Https,
                    host,
                    None
                )
                .is_err()
            );
        }
        assert_eq!(
            StableIdentity::browser_origin(
                BrowserAdapter::Chromium,
                WebScheme::Http,
                "[2001:0db8::1]",
                None,
            )
            .expect("IPv6"),
            StableIdentity::BrowserOrigin {
                adapter: BrowserAdapter::Chromium,
                scheme: WebScheme::Http,
                host: "2001:db8::1".to_owned(),
                port: 80,
            }
        );
    }

    #[test]
    fn permission_dependencies_and_retention_bounds_are_strict() {
        let mut permissions = SubjectPermissions::deny_all();
        permissions.suggest = PermissionDecision::Allow;
        assert!(permissions.validate().is_err());

        let mut permissions = allowed(None);
        permissions.suggest = PermissionDecision::Block;
        assert!(permissions.validate().is_err());

        let mut permissions = allowed(Some(1));
        assert!(permissions.validate().is_ok());
        permissions.retention = RetentionPermission::Bounded { days: 91 };
        assert!(permissions.validate().is_err());
    }

    #[test]
    fn settings_require_sorted_unique_subjects() {
        let first = SubjectRule {
            identity: identity("a.example"),
            permissions: allowed(None),
        };
        let second = SubjectRule {
            identity: identity("b.example"),
            permissions: allowed(None),
        };
        let valid = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 1,
            paused: false,
            subjects: vec![first.clone(), second.clone()],
        };
        valid.validate().expect("canonical settings");
        let mut reversed = valid.clone();
        reversed.subjects.reverse();
        assert!(reversed.validate().is_err());
        let mut duplicate = valid;
        duplicate.subjects = vec![first.clone(), first];
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn maximum_shape_settings_fit_storage_and_wire_limits() {
        let subjects = (0..MAX_SUBJECTS)
            .map(|index| {
                let host = format!(
                    "h{index:03}.{}.{}.{}.{}",
                    "a".repeat(63),
                    "b".repeat(63),
                    "c".repeat(63),
                    "d".repeat(56)
                );
                assert_eq!(host.len(), 253);
                SubjectRule {
                    identity: identity(&host),
                    permissions: allowed(Some(90)),
                }
            })
            .collect();
        let settings = SettingsV1 {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: crate::protocol::MAX_SAFE_COUNTER,
            paused: true,
            subjects,
        };
        settings.validate().expect("maximum-shape settings");
        let stored = super::encode_settings(&settings).expect("bounded settings encoding");
        assert!(stored.len() <= MAX_SETTINGS_BYTES);

        let wire_value = serde_json::json!({
            "v": 1,
            "id": "x".repeat(128),
            "type": "settings.replace",
            "mono_ms": crate::protocol::MAX_SAFE_COUNTER,
            "payload": {
                "expected_revision": crate::protocol::MAX_SAFE_COUNTER,
                "document": settings
            }
        });
        let envelope: WireEnvelope =
            serde_json::from_value(wire_value).expect("maximum settings envelope");
        envelope.validate_shape().expect("valid envelope shape");
        assert!(serde_json::to_vec(&envelope).expect("wire encoding").len() <= MAX_FRAME_BYTES);
    }

    #[test]
    fn strict_json_rejects_unknown_fields_and_unsafe_combinations() {
        let unknown = br#"{
          "schema":"badi.settings.v1","revision":0,"paused":true,"subjects":[],
          "cloud":"allow"
        }"#;
        assert!(serde_json::from_slice::<SettingsV1>(unknown).is_err());

        let invalid = br#"{
          "schema":"badi.settings.v1","revision":1,"paused":false,
          "subjects":[{"identity":{"kind":"browser_origin","adapter":"chromium",
          "scheme":"https","host":"example.com","port":443},"permissions":{
          "suggest":"allow","display":"block","context_read":"allow","learn":"block",
          "retention":{"mode":"none"}}}]
        }"#;
        let decoded: SettingsV1 = serde_json::from_slice(invalid).expect("structurally valid");
        assert!(decoded.validate().is_err());
    }

    #[test]
    fn legacy_v1_document_migrates_atomically_to_canonical_v2_on_load() {
        let temporary = tempdir().expect("temporary directory");
        let paths = StoragePaths::new(
            temporary.path().join("config/badi"),
            temporary.path().join("data/badi"),
        )
        .expect("paths");
        let storage = PrivateStorage::open(paths.clone()).expect("private storage");
        let legacy = format!(
            r#"{{
              "schema":"{SETTINGS_SCHEMA_V1}","revision":1,"paused":false,
              "subjects":[{{"identity":{{"kind":"browser_origin","adapter":"chromium",
              "scheme":"https","host":"example.com","port":443}},"permissions":{{
              "suggest":"allow","display":"allow","context_read":"allow","learn":"block",
              "retention":{{"mode":"none"}}}}}}]
            }}"#
        );
        fs::write(paths.settings_path(), legacy).expect("seed legacy settings");
        fs::set_permissions(
            paths.settings_path(),
            fs::Permissions::from_mode(PRIVATE_FILE_MODE),
        )
        .expect("private legacy settings");

        let settings = storage
            .settings_store()
            .expect("settings store")
            .load_or_initialize()
            .expect("migrate settings");
        assert_eq!(settings.schema, SETTINGS_SCHEMA);
        assert_eq!(settings.revision, 1);
        assert!(
            settings
                .resolve_identity(&identity("example.com"))
                .allows_suggestion()
        );

        let persisted: serde_json::Value = serde_json::from_slice(
            &fs::read(paths.settings_path()).expect("read migrated settings"),
        )
        .expect("migrated JSON");
        assert_eq!(persisted["schema"], SETTINGS_SCHEMA);
        assert!(
            fs::read_dir(paths.config_dir())
                .expect("config directory")
                .all(|entry| !entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".tmp"))
        );
    }

    #[test]
    fn private_storage_locks_for_lifetime_and_uses_path_override() {
        let temporary = tempdir().expect("temporary directory");
        let paths = StoragePaths::new(
            temporary.path().join("config/badi"),
            temporary.path().join("data/badi"),
        )
        .expect("paths");
        let storage = PrivateStorage::open(paths.clone()).expect("first lock");
        assert!(matches!(
            PrivateStorage::open(paths),
            Err(PrivateStorageError::LockUnavailable)
        ));
        drop(storage);
    }

    #[test]
    fn startup_removes_only_safe_owned_complete_document_temporaries() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = StoragePaths::new(
            temporary.path().join("config/badi"),
            temporary.path().join("data/badi"),
        )
        .expect("paths");
        for directory in [storage_paths.config_dir(), storage_paths.data_dir()] {
            fs::create_dir_all(directory).expect("storage directory");
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("private directory");
        }
        let settings_temporary = storage_paths
            .config_dir()
            .join(".settings.json.00000000000000000000000000000001.tmp");
        let personalization_temporary = storage_paths
            .data_dir()
            .join(".personalization.json.00000000000000000000000000000002.tmp");
        let unrelated = storage_paths
            .data_dir()
            .join(".personalization.json.not-a-badi-nonce.tmp");
        for path in [&settings_temporary, &personalization_temporary, &unrelated] {
            fs::write(path, b"private complete document\n").expect("temporary file");
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("private temporary mode");
        }

        let _storage = PrivateStorage::open(storage_paths).expect("storage recovery");

        assert!(!settings_temporary.exists());
        assert!(!personalization_temporary.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn startup_preserves_and_rejects_an_unsafe_owned_temporary_file() {
        let temporary = tempdir().expect("temporary directory");
        let storage_paths = StoragePaths::new(
            temporary.path().join("config/badi"),
            temporary.path().join("data/badi"),
        )
        .expect("paths");
        fs::create_dir_all(storage_paths.config_dir()).expect("config directory");
        fs::create_dir_all(storage_paths.data_dir()).expect("data directory");
        fs::set_permissions(
            storage_paths.config_dir(),
            fs::Permissions::from_mode(0o700),
        )
        .expect("private config directory");
        fs::set_permissions(storage_paths.data_dir(), fs::Permissions::from_mode(0o700))
            .expect("private data directory");
        let unsafe_temporary = storage_paths
            .data_dir()
            .join(".personalization.json.00000000000000000000000000000003.tmp");
        fs::write(&unsafe_temporary, b"untrusted file\n").expect("unsafe temporary file");
        fs::set_permissions(&unsafe_temporary, fs::Permissions::from_mode(0o644))
            .expect("unsafe temporary mode");

        assert!(matches!(
            PrivateStorage::open(storage_paths),
            Err(PrivateStorageError::UnsafeFile)
        ));
        assert!(unsafe_temporary.exists());
    }

    #[test]
    fn either_shared_storage_root_uses_the_same_interprocess_lock() {
        let temporary = tempdir().expect("temporary directory");
        let shared_config = temporary.path().join("config/shared-badi");
        let shared_data = temporary.path().join("data/shared-badi");
        let first = PrivateStorage::open(
            StoragePaths::new(&shared_config, &shared_data).expect("first paths"),
        )
        .expect("first storage");

        assert!(matches!(
            PrivateStorage::open(
                StoragePaths::new(&shared_config, temporary.path().join("data/other-badi"),)
                    .expect("shared config paths"),
            ),
            Err(PrivateStorageError::LockUnavailable)
        ));
        assert!(matches!(
            PrivateStorage::open(
                StoragePaths::new(temporary.path().join("config/other-badi"), &shared_data,)
                    .expect("shared data paths"),
            ),
            Err(PrivateStorageError::LockUnavailable)
        ));
        drop(first);
    }

    #[test]
    fn settings_store_initializes_privately_and_enforces_cas() {
        let temporary = tempdir().expect("temporary directory");
        let config = temporary.path().join("config/badi");
        let data = temporary.path().join("data/badi");
        let storage =
            PrivateStorage::open(StoragePaths::new(config, data).expect("paths")).expect("storage");
        let store = storage.settings_store().expect("settings store");
        let initial = store.load_or_initialize().expect("initial settings");
        assert_eq!(initial, SettingsV1::deny_by_default());
        let metadata = fs::metadata(store.path()).expect("settings metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);

        let next = settings_with(SubjectRule {
            identity: identity("example.com"),
            permissions: allowed(Some(30)),
        });
        store
            .compare_and_replace(0, next.clone())
            .expect("first replacement");
        assert!(matches!(
            store.compare_and_replace(0, next.clone()),
            Err(SettingsStoreError::RevisionConflict {
                expected: 0,
                actual: 1
            })
        ));
        assert_eq!(store.load().expect("load"), Some(next));
    }

    #[test]
    fn post_rename_write_error_reloads_new_authority_instead_of_restoring_old() {
        let temporary = tempdir().expect("temporary directory");
        let storage = PrivateStorage::open(
            StoragePaths::new(
                temporary.path().join("config/badi"),
                temporary.path().join("data/badi"),
            )
            .expect("paths"),
        )
        .expect("storage");
        let store = storage.settings_store().expect("settings store");
        store.load_or_initialize().expect("initial settings");
        let next = settings_with(SubjectRule {
            identity: identity("example.com"),
            permissions: allowed(Some(30)),
        });

        let replaced = store
            .compare_and_replace_with_writer(0, &next, |path, document| {
                write_settings(path, document)?;
                Err(SettingsStoreError::Storage(PrivateStorageError::Io(
                    std::io::Error::other("injected post-rename failure"),
                )))
            })
            .expect("exact new document is authoritative");

        assert_eq!(replaced, next);
        assert_eq!(store.load().expect("load"), Some(replaced));
    }

    #[test]
    fn store_rejects_symlinks_hardlinks_and_public_files() {
        let temporary = tempdir().expect("temporary directory");
        let directory = temporary.path().join("config/badi");
        let storage = PrivateStorage::open(
            StoragePaths::new(&directory, temporary.path().join("data/badi")).expect("paths"),
        )
        .expect("storage");
        let real = directory.join("real.json");
        fs::write(&real, b"{}").expect("real file");
        fs::set_permissions(&real, fs::Permissions::from_mode(0o600)).expect("file mode");

        let symlink = directory.join("settings.json");
        std::os::unix::fs::symlink(&real, &symlink).expect("symlink");
        let store = storage.settings_store().expect("store");
        assert!(store.load().is_err());
        fs::remove_file(&symlink).expect("remove symlink");

        fs::hard_link(&real, &symlink).expect("hard link");
        assert!(store.load().is_err());
        fs::remove_file(&symlink).expect("remove hard link");
        fs::remove_file(&real).expect("remove real");

        fs::write(&symlink, b"{}").expect("public file");
        fs::set_permissions(&symlink, fs::Permissions::from_mode(0o644)).expect("public mode");
        assert!(store.load().is_err());
    }

    #[test]
    fn clear_retry_requires_a_successful_directory_sync_after_visible_absence() {
        fn fail_sync(_: &std::path::Path) -> Result<(), PrivateStorageError> {
            Err(PrivateStorageError::CommitStateUnknown)
        }

        let temporary = tempdir().expect("temporary directory");
        let directory = temporary.path().join("data/badi");
        fs::create_dir_all(&directory).expect("data directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("private directory");
        let path = directory.join("personalization.json");
        fs::write(&path, b"private state\n").expect("private file");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("private mode");
        assert!(matches!(
            remove_private_file_with_sync(&path, fail_sync),
            Err(PrivateStorageError::CommitStateUnknown)
        ));
        assert!(!path.exists());
        assert!(matches!(
            remove_private_file_with_sync(&path, fail_sync),
            Err(PrivateStorageError::CommitStateUnknown)
        ));
        assert!(!path.exists());
        assert!(
            !remove_private_file_with_sync(&path, super::sync_directory)
                .expect("durably acknowledge absence")
        );
    }
}
