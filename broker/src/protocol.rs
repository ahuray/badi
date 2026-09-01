use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;
use uuid::{Uuid, Variant};

use crate::metrics::MetricsSnapshot;

/// The legacy browser wire version. Constructors continue to default to this
/// version so existing Chromium call sites cannot change behavior accidentally.
pub const PROTOCOL_VERSION: u8 = 1;
pub const MIN_PROTOCOL_VERSION: u8 = PROTOCOL_VERSION;
pub const CURRENT_PROTOCOL_VERSION: u8 = 2;
pub const MAX_SAFE_COUNTER: u64 = 9_007_199_254_740_991;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const MAX_BEFORE_CHARS: usize = 512;
pub const MAX_AFTER_CHARS: usize = 128;
pub const MAX_SUGGESTION_CHARS: usize = 64;
pub const MAX_SUGGESTION_WORDS: usize = 8;
pub const MAX_ID_CHARS: usize = 128;
pub const DEFAULT_SUGGESTION_TTL_MS: u64 = 600;
pub const CAPABILITY_COUNT: usize = 8;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SessionId(Uuid);

impl SessionId {
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Serialize for SessionId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for SessionId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let parsed = Uuid::parse_str(&raw).map_err(D::Error::custom)?;
        if parsed.to_string() != raw {
            return Err(D::Error::custom("noncanonical_session_id"));
        }
        let version = parsed.get_version_num();
        if !(1..=8).contains(&version) {
            return Err(D::Error::custom("unsupported_session_id_version"));
        }
        if parsed.get_variant() != Variant::RFC4122 {
            return Err(D::Error::custom("unsupported_session_id_variant"));
        }
        Ok(Self(parsed))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AdapterKind {
    Browser,
    Obsidian,
    Fcitx,
    Cli,
    Test,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum Capability {
    #[serde(rename = "context")]
    Context,
    #[serde(rename = "suggestion")]
    Suggestion,
    #[serde(rename = "commit.applied")]
    CommitApplied,
    #[serde(rename = "commit.dispatched_unverified")]
    CommitDispatchedUnverified,
    #[serde(rename = "control")]
    Control,
    #[serde(rename = "health")]
    Health,
    #[serde(rename = "policy")]
    Policy,
    #[serde(rename = "settings")]
    Settings,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Browser,
    DesktopApplication,
    Obsidian,
    Terminal,
    Fixture,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Always,
    Manual,
    Never,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldPurpose {
    Normal,
    Password,
    Pin,
    Otp,
    PaymentSecret,
    Terminal,
    Email,
    Url,
    Search,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    PhraseV1,
    LocalModel,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    Request,
    AcceptWord,
    AcceptAll,
    Dismiss,
    Pause,
    Resume,
    PauseToggle,
}

impl ControlAction {
    #[must_use]
    pub const fn is_global(self) -> bool {
        matches!(self, Self::Pause | Self::Resume | Self::PauseToggle)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Acceptance {
    Word,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum CommitStatus {
    #[serde(rename = "applied")]
    Applied,
    #[serde(rename = "dispatched-unverified")]
    DispatchedUnverified,
    #[serde(rename = "stale")]
    Stale,
    #[serde(rename = "blocked")]
    Blocked,
    #[serde(rename = "failed")]
    Failed,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    Accepted,
    AmbiguousSession,
    Cancelled,
    Dismissed,
    Expired,
    FieldAmbiguous,
    FieldNotEditable,
    FieldSensitive,
    FocusChanged,
    InvalidCapability,
    InvalidFrame,
    InvalidMessage,
    InvalidOutput,
    ManualRequired,
    NoContext,
    NoSuggestion,
    Paused,
    PolicyNever,
    ProviderError,
    ProviderTimeout,
    SessionClosed,
    SettingsCommitUnknown,
    SettingsCommittedDegraded,
    SettingsConflict,
    SettingsRejected,
    Stale,
    Superseded,
    UnknownSession,
    UnsupportedVersion,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum MessageType {
    #[serde(rename = "hello")]
    Hello,
    #[serde(rename = "hello.ack")]
    HelloAck,
    #[serde(rename = "session.open")]
    SessionOpen,
    #[serde(rename = "session.close")]
    SessionClose,
    #[serde(rename = "context.changed")]
    ContextChanged,
    #[serde(rename = "suggest.request")]
    SuggestRequest,
    #[serde(rename = "suggest.cancel")]
    SuggestCancel,
    #[serde(rename = "suggestion.show")]
    SuggestionShow,
    #[serde(rename = "suggestion.clear")]
    SuggestionClear,
    #[serde(rename = "control.request")]
    ControlRequest,
    #[serde(rename = "control.result")]
    ControlResult,
    #[serde(rename = "commit.prepare")]
    CommitPrepare,
    #[serde(rename = "commit.result")]
    CommitResult,
    #[serde(rename = "health.request")]
    HealthRequest,
    #[serde(rename = "health.status")]
    HealthStatus,
    #[serde(rename = "policy.query")]
    PolicyQuery,
    #[serde(rename = "policy.status")]
    PolicyStatus,
    #[serde(rename = "authority.changed")]
    AuthorityChanged,
    #[serde(rename = "authority.ack")]
    AuthorityAck,
    #[serde(rename = "settings.get")]
    SettingsGet,
    #[serde(rename = "settings.replace")]
    SettingsReplace,
    #[serde(rename = "settings.status")]
    SettingsStatus,
    #[serde(rename = "memory.clear")]
    MemoryClear,
    #[serde(rename = "memory.status")]
    MemoryStatus,
    #[serde(rename = "error")]
    Error,
}

impl MessageType {
    #[must_use]
    pub const fn requires_coordinates(self) -> bool {
        matches!(
            self,
            Self::SessionOpen
                | Self::SessionClose
                | Self::ContextChanged
                | Self::SuggestRequest
                | Self::SuggestCancel
                | Self::SuggestionShow
                | Self::SuggestionClear
                | Self::CommitPrepare
                | Self::CommitResult
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Coordinates {
    pub session_id: SessionId,
    pub focus_epoch: u64,
    pub revision: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WireEnvelope {
    pub v: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type")]
    pub message_type: MessageType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub focus_epoch: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<u64>,
    pub mono_ms: u64,
    pub payload: Value,
}

impl WireEnvelope {
    pub fn session<P: Serialize>(
        message_type: MessageType,
        coordinates: Coordinates,
        mono_ms: u64,
        payload: &P,
    ) -> Result<Self, ProtocolError> {
        let envelope = Self {
            v: PROTOCOL_VERSION,
            id: None,
            message_type,
            session_id: Some(coordinates.session_id),
            focus_epoch: Some(coordinates.focus_epoch),
            revision: Some(coordinates.revision),
            mono_ms,
            payload: serde_json::to_value(payload)?,
        };
        envelope.validate_shape()?;
        Ok(envelope)
    }

    pub fn global<P: Serialize>(
        message_type: MessageType,
        mono_ms: u64,
        payload: &P,
    ) -> Result<Self, ProtocolError> {
        let envelope = Self {
            v: PROTOCOL_VERSION,
            id: None,
            message_type,
            session_id: None,
            focus_epoch: None,
            revision: None,
            mono_ms,
            payload: serde_json::to_value(payload)?,
        };
        envelope.validate_shape()?;
        Ok(envelope)
    }

    pub fn validate_shape(&self) -> Result<(), ProtocolError> {
        if !(MIN_PROTOCOL_VERSION..=CURRENT_PROTOCOL_VERSION).contains(&self.v) {
            return Err(ProtocolError::UnsupportedVersion(self.v));
        }
        if self.mono_ms > MAX_SAFE_COUNTER {
            return Err(ProtocolError::CounterOutOfRange("mono_ms"));
        }
        if self.id.as_deref().is_some_and(|id| !valid_opaque_id(id)) {
            return Err(ProtocolError::InvalidId);
        }
        if matches!(
            self.message_type,
            MessageType::AuthorityChanged | MessageType::AuthorityAck
        ) && self.id.is_some()
        {
            return Err(ProtocolError::UnexpectedId);
        }
        if !self.payload.is_object() {
            return Err(ProtocolError::PayloadNotObject);
        }

        let coordinate_count = usize::from(self.session_id.is_some())
            + usize::from(self.focus_epoch.is_some())
            + usize::from(self.revision.is_some());
        if self.message_type.requires_coordinates() {
            if coordinate_count != 3 {
                return Err(ProtocolError::MissingCoordinates);
            }
            validate_coordinate_bounds(
                self.focus_epoch.ok_or(ProtocolError::MissingCoordinates)?,
                self.revision.ok_or(ProtocolError::MissingCoordinates)?,
            )?;
        } else if self.message_type == MessageType::ControlRequest {
            if coordinate_count != 0 && coordinate_count != 3 {
                return Err(ProtocolError::MissingCoordinates);
            }
            let action = self
                .payload
                .get("action")
                .cloned()
                .ok_or(ProtocolError::InvalidPayload)
                .and_then(|value| {
                    serde_json::from_value::<ControlAction>(value)
                        .map_err(|_| ProtocolError::InvalidPayload)
                })?;
            if action.is_global() && coordinate_count != 0 {
                return Err(ProtocolError::UnexpectedCoordinates);
            }
            if !action.is_global() && coordinate_count != 3 {
                return Err(ProtocolError::MissingCoordinates);
            }
            if !action.is_global() {
                validate_coordinate_bounds(
                    self.focus_epoch.ok_or(ProtocolError::MissingCoordinates)?,
                    self.revision.ok_or(ProtocolError::MissingCoordinates)?,
                )?;
            }
        } else if coordinate_count != 0 {
            return Err(ProtocolError::UnexpectedCoordinates);
        }
        Ok(())
    }

    pub fn at_version(mut self, version: u8) -> Result<Self, ProtocolError> {
        self.v = version;
        self.validate_shape()?;
        Ok(self)
    }

    pub fn coordinates(&self) -> Result<Coordinates, ProtocolError> {
        self.validate_shape()?;
        Ok(Coordinates {
            session_id: self.session_id.ok_or(ProtocolError::MissingCoordinates)?,
            focus_epoch: self.focus_epoch.ok_or(ProtocolError::MissingCoordinates)?,
            revision: self.revision.ok_or(ProtocolError::MissingCoordinates)?,
        })
    }

    pub fn decode_payload<T: DeserializeOwned>(&self) -> Result<T, ProtocolError> {
        Ok(serde_json::from_value(self.payload.clone())?)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterDescriptor {
    pub kind: AdapterKind,
    pub name: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelloPayload {
    pub min_v: u8,
    pub max_v: u8,
    pub adapter: AdapterDescriptor,
    pub capabilities: Vec<Capability>,
}

impl HelloPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.select_version().is_none() {
            return Err(ProtocolError::VersionNegotiationFailed);
        }
        if self.adapter.name.is_empty()
            || self.adapter.name.chars().count() > 64
            || self.adapter.version.is_empty()
            || self.adapter.version.chars().count() > 32
            || self.capabilities.is_empty()
            || self.capabilities.len() > CAPABILITY_COUNT
        {
            return Err(ProtocolError::InvalidPayload);
        }
        let mut unique = self.capabilities.clone();
        unique.sort_by_key(|capability| *capability as u8);
        unique.dedup();
        if unique.len() != self.capabilities.len() {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }

    #[must_use]
    pub fn select_version(&self) -> Option<u8> {
        if self.min_v > self.max_v {
            return None;
        }
        let selected = self.max_v.min(CURRENT_PROTOCOL_VERSION);
        (selected >= self.min_v.max(MIN_PROTOCOL_VERSION)).then_some(selected)
    }

    pub fn validate_for_frame(&self, frame_version: u8) -> Result<(), ProtocolError> {
        self.validate()?;
        if self.min_v != frame_version || self.max_v != frame_version {
            return Err(ProtocolError::VersionNegotiationFailed);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HelloAckPayload {
    pub selected_v: u8,
    pub connection_id: String,
    pub enabled_capabilities: Vec<Capability>,
    pub max_frame_bytes: usize,
    pub max_before_chars: usize,
    pub max_after_chars: usize,
    pub max_suggestion_chars: usize,
    pub max_suggestion_words: usize,
    pub paused: bool,
}

impl HelloAckPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !(MIN_PROTOCOL_VERSION..=CURRENT_PROTOCOL_VERSION).contains(&self.selected_v)
            || !valid_opaque_id(&self.connection_id)
            || self.enabled_capabilities.is_empty()
            || self.enabled_capabilities.len() > CAPABILITY_COUNT
            || self.max_frame_bytes != MAX_FRAME_BYTES
            || self.max_before_chars != MAX_BEFORE_CHARS
            || self.max_after_chars != MAX_AFTER_CHARS
            || self.max_suggestion_chars != MAX_SUGGESTION_CHARS
            || self.max_suggestion_words != MAX_SUGGESTION_WORDS
        {
            return Err(ProtocolError::InvalidPayload);
        }
        let mut unique = self.enabled_capabilities.clone();
        unique.sort_by_key(|capability| *capability as u8);
        unique.dedup();
        if unique.len() != self.enabled_capabilities.len() {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub scheme: OriginScheme,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OriginScheme {
    #[serde(rename = "http")]
    Http,
    #[serde(rename = "https")]
    Https,
    #[serde(rename = "chrome-extension")]
    ChromeExtension,
    #[serde(rename = "moz-extension")]
    MozExtension,
    #[serde(rename = "file")]
    File,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TargetDescriptor {
    pub kind: TargetKind,
    pub app_id: String,
    pub target_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<Origin>,
}

impl TargetDescriptor {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.app_id.is_empty()
            || self.app_id.chars().count() > 128
            || !valid_opaque_id(&self.target_id)
            || self.origin.as_ref().is_some_and(|origin| {
                origin.host.is_empty()
                    || origin.host.chars().count() > 253
                    || origin.port == Some(0)
            })
        {
            return Err(ProtocolError::InvalidPayload);
        }
        match self.kind {
            TargetKind::DesktopApplication => {
                if self.origin.is_some() || !valid_linux_app_id(&self.app_id) {
                    return Err(ProtocolError::InvalidPayload);
                }
            }
            TargetKind::Browser
            | TargetKind::Obsidian
            | TargetKind::Terminal
            | TargetKind::Fixture => {}
        }
        Ok(())
    }

    pub fn validate_for_version(&self, version: u8) -> Result<(), ProtocolError> {
        self.validate()?;
        if version == PROTOCOL_VERSION && self.kind == TargetKind::DesktopApplication {
            return Err(ProtocolError::InvalidPayload);
        }
        if version == CURRENT_PROTOCOL_VERSION
            && self.kind == TargetKind::Browser
            && self.origin.is_none()
        {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyResolutionReason {
    DefaultPolicy,
    GlobalDisabled,
    ContextDisabled,
    MatchedRule,
    SuggestionsDisabled,
    UnknownIdentity,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyQueryPayload {
    pub target: TargetDescriptor,
}

impl PolicyQueryPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        self.target.validate()
    }

    pub fn validate_for_version(&self, version: u8) -> Result<(), ProtocolError> {
        self.target.validate_for_version(version)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
// These are independent wire permissions; collapsing them into one enum would
// hide partial externally-authored policies from strict clients.
#[allow(clippy::struct_excessive_bools)]
pub struct PolicyStatusPayload {
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub paused: bool,
    pub activation: Activation,
    pub context_allowed: bool,
    pub display_allowed: bool,
    pub suggestions_allowed: bool,
    pub learning_allowed: bool,
    pub reason: PolicyResolutionReason,
}

impl PolicyStatusPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.authority_epoch > MAX_SAFE_COUNTER
            || self.settings_revision > MAX_SAFE_COUNTER
            || self.suggestions_allowed && (!self.context_allowed || !self.display_allowed)
            || self.learning_allowed
                && (!self.context_allowed || !self.display_allowed || !self.suggestions_allowed)
            || self.paused
                && (self.context_allowed
                    || self.suggestions_allowed
                    || self.display_allowed
                    || self.learning_allowed
                    || self.activation != Activation::Never)
            || !self.context_allowed && self.activation == Activation::Always
        {
            Err(ProtocolError::InvalidPayload)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityChangedPayload {
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub paused: bool,
}

impl AuthorityChangedPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_authority_counter(self.authority_epoch, self.settings_revision)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityAckPayload {
    pub authority_epoch: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsReplacePayload {
    pub expected_revision: u64,
    pub document: Value,
}

impl SettingsReplacePayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.expected_revision > MAX_SAFE_COUNTER || !self.document.is_object() {
            Err(ProtocolError::InvalidPayload)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsStatusPayload {
    pub document: Value,
    pub personalization_revision: u64,
    pub personalization_records: u64,
    pub personalization_bytes: u64,
    pub personalization_store_available: bool,
    pub personalization_recorder_available: bool,
    pub personalization_write_failures: u64,
    pub personalization_dropped_signals: u64,
}

impl SettingsStatusPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if !self.document.is_object()
            || self.personalization_revision > MAX_SAFE_COUNTER
            || self.personalization_records > MAX_SAFE_COUNTER
            || self.personalization_bytes > MAX_SAFE_COUNTER
            || self.personalization_write_failures > MAX_SAFE_COUNTER
            || self.personalization_dropped_signals > MAX_SAFE_COUNTER
        {
            Err(ProtocolError::InvalidPayload)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryStatusPayload {
    pub revision: u64,
    pub records: u64,
    pub bytes: u64,
    pub changed: bool,
}

impl MemoryStatusPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_authority_counter(self.revision, self.records)?;
        if self.bytes > MAX_SAFE_COUNTER {
            Err(ProtocolError::CounterOutOfRange("memory_bytes"))
        } else {
            Ok(())
        }
    }
}

impl AuthorityAckPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_authority_counter(self.authority_epoch, 0)
    }
}

fn validate_authority_counter(
    authority_epoch: u64,
    settings_revision: u64,
) -> Result<(), ProtocolError> {
    if authority_epoch > MAX_SAFE_COUNTER || settings_revision > MAX_SAFE_COUNTER {
        Err(ProtocolError::CounterOutOfRange("authority"))
    } else {
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionOpenPayload {
    pub target: TargetDescriptor,
    pub activation: Activation,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionClosePayload {
    pub reason: ReasonCode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub anchor: u64,
    pub head: u64,
    pub unit: OffsetUnit,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum OffsetUnit {
    #[serde(rename = "utf16_code_units")]
    Utf16CodeUnits,
    #[serde(rename = "unicode_scalar_values")]
    UnicodeScalarValues,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct FieldDescriptor {
    pub purpose: FieldPurpose,
    pub editable: bool,
    pub multiline: bool,
    pub composing: bool,
    pub sensitive: bool,
    pub identity_known: bool,
    pub focused: bool,
    pub lock_screen: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextChangedPayload {
    pub fingerprint: String,
    pub before: String,
    pub after: String,
    pub selection: Selection,
    pub field: FieldDescriptor,
    pub activation: Activation,
    pub explicit: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl ContextChangedPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_fingerprint(&self.fingerprint)?;
        let hard_denied = self.activation == Activation::Never
            || self.field.sensitive
            || self.field.lock_screen
            || matches!(
                self.field.purpose,
                FieldPurpose::Password
                    | FieldPurpose::Pin
                    | FieldPurpose::Otp
                    | FieldPurpose::PaymentSecret
            );
        if self.before.chars().count() > MAX_BEFORE_CHARS
            || self.after.chars().count() > MAX_AFTER_CHARS
            || (hard_denied && (!self.before.is_empty() || !self.after.is_empty()))
            || self.selection.anchor > MAX_SAFE_COUNTER
            || self.selection.head > MAX_SAFE_COUNTER
            || self
                .language
                .as_ref()
                .is_some_and(|language| !valid_language_tag(language))
        {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }

    pub fn validate_for_version(&self, version: u8) -> Result<(), ProtocolError> {
        self.validate()?;
        match (version, self.selection.unit) {
            (PROTOCOL_VERSION, OffsetUnit::Utf16CodeUnits)
            | (CURRENT_PROTOCOL_VERSION, OffsetUnit::UnicodeScalarValues) => Ok(()),
            _ => Err(ProtocolError::InvalidPayload),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestRequestPayload {
    pub fingerprint: String,
    pub explicit: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestCancelPayload {
    pub fingerprint: String,
    pub reason: ReasonCode,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionShowPayload {
    pub fingerprint: String,
    pub suggestion_id: String,
    pub text: String,
    pub accept_word: String,
    pub ttl_ms: u64,
    pub provider: ProviderKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SuggestionClearPayload {
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion_id: Option<String>,
    pub reason: ReasonCode,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalControlRequestPayload {
    pub action: ControlAction,
}

impl GlobalControlRequestPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.action.is_global() {
            Ok(())
        } else {
            Err(ProtocolError::InvalidPayload)
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionControlRequestPayload {
    pub action: ControlAction,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion_id: Option<String>,
}

impl SessionControlRequestPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_fingerprint(&self.fingerprint)?;
        if self.action.is_global()
            || (self.action == ControlAction::Request && self.suggestion_id.is_some())
            || (self.action != ControlAction::Request
                && self
                    .suggestion_id
                    .as_deref()
                    .is_none_or(|id| !valid_opaque_id(id)))
        {
            Err(ProtocolError::InvalidPayload)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ControlResultPayload {
    pub action: ControlAction,
    pub accepted: bool,
    pub reason: ReasonCode,
    pub paused: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitPreparePayload {
    pub fingerprint: String,
    pub suggestion_id: String,
    pub text: String,
    pub acceptance: Acceptance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommitResultPayload {
    pub fingerprint: String,
    pub suggestion_id: String,
    pub status: CommitStatus,
}

impl CommitResultPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_fingerprint(&self.fingerprint)?;
        if !valid_opaque_id(&self.suggestion_id) {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyPayload {}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthStatusPayload {
    pub provider: ProviderKind,
    pub paused: bool,
    pub authority_epoch: u64,
    pub settings_revision: u64,
    pub control_plane_degraded: bool,
    pub sessions: u64,
    pub socket_mode: String,
    pub max_frame_bytes: usize,
    pub metrics: MetricsSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<ActiveLocator>,
}

impl HealthStatusPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_authority_counter(self.authority_epoch, self.settings_revision)?;
        if self.sessions > MAX_SAFE_COUNTER
            || self.socket_mode != "0600"
            || self.max_frame_bytes != MAX_FRAME_BYTES
            || (self.control_plane_degraded && !self.paused)
        {
            Err(ProtocolError::InvalidPayload)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveLocator {
    pub session_id: SessionId,
    pub focus_epoch: u64,
    pub revision: u64,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggestion_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorPayload {
    pub code: ReasonCode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub committed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settings_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub control_plane_degraded: Option<bool>,
}

impl ErrorPayload {
    #[must_use]
    pub const fn simple(code: ReasonCode) -> Self {
        Self {
            code,
            committed: None,
            settings_revision: None,
            control_plane_degraded: None,
        }
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        let has_settings_context = self.committed.is_some()
            || self.settings_revision.is_some()
            || self.control_plane_degraded.is_some();
        if has_settings_context
            && (self.settings_revision.is_none() || self.control_plane_degraded.is_none())
        {
            return Err(ProtocolError::InvalidPayload);
        }
        if self
            .settings_revision
            .is_some_and(|revision| revision > MAX_SAFE_COUNTER)
        {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ProtocolError {
    #[error("counter_out_of_range:{0}")]
    CounterOutOfRange(&'static str),
    #[error("invalid_id")]
    InvalidId,
    #[error("invalid_payload")]
    InvalidPayload,
    #[error("missing_coordinates")]
    MissingCoordinates,
    #[error("payload_not_object")]
    PayloadNotObject,
    #[error("serde")]
    Serde(#[from] serde_json::Error),
    #[error("unexpected_coordinates")]
    UnexpectedCoordinates,
    #[error("unexpected_id")]
    UnexpectedId,
    #[error("unsupported_version:{0}")]
    UnsupportedVersion(u8),
    #[error("version_negotiation_failed")]
    VersionNegotiationFailed,
}

#[must_use]
pub fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().count() <= MAX_ID_CHARS
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
}

/// Canonical Fcitx program identity used for both the session target and its
/// settings subject. Restricting this to a stable, comparison-safe ASCII form
/// avoids accidentally granting policy to display names or window titles.
#[must_use]
pub fn valid_linux_app_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_ID_CHARS
        && value.split('.').all(|segment| {
            let mut bytes = segment.bytes();
            bytes.next().is_some_and(|byte| byte.is_ascii_lowercase())
                && bytes.all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'_' | b'-')
                })
        })
}

pub(crate) fn valid_language_tag(value: &str) -> bool {
    (2..=35).contains(&value.chars().count())
        && value.split('-').all(|subtag| {
            !subtag.is_empty() && subtag.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

pub fn validate_fingerprint(value: &str) -> Result<(), ProtocolError> {
    if (16..=128).contains(&value.chars().count())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(ProtocolError::InvalidPayload)
    }
}

pub fn validate_coordinate_bounds(focus_epoch: u64, revision: u64) -> Result<(), ProtocolError> {
    if focus_epoch > MAX_SAFE_COUNTER {
        Err(ProtocolError::CounterOutOfRange("focus_epoch"))
    } else if revision > MAX_SAFE_COUNTER {
        Err(ProtocolError::CounterOutOfRange("revision"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        AdapterDescriptor, AdapterKind, CURRENT_PROTOCOL_VERSION, Capability, HelloPayload,
        MAX_SAFE_COUNTER, MessageType, PROTOCOL_VERSION, PolicyQueryPayload, ProtocolError,
        SessionId, TargetDescriptor, TargetKind, WireEnvelope, valid_language_tag,
    };

    #[test]
    fn hello_accepts_each_capability_declared_by_the_v1_schema() {
        let hello = HelloPayload {
            min_v: PROTOCOL_VERSION,
            max_v: PROTOCOL_VERSION,
            adapter: AdapterDescriptor {
                kind: AdapterKind::Test,
                name: "all-capabilities".to_owned(),
                version: "1".to_owned(),
            },
            capabilities: vec![
                Capability::Context,
                Capability::Suggestion,
                Capability::CommitApplied,
                Capability::CommitDispatchedUnverified,
                Capability::Control,
                Capability::Health,
                Capability::Policy,
                Capability::Settings,
            ],
        };
        hello.validate().expect("all schema capabilities");
    }

    #[test]
    fn rejects_coordinates_on_global_message() {
        let envelope: WireEnvelope = serde_json::from_value(json!({
            "v": 1,
            "type": "health.request",
            "session_id": SessionId::new(),
            "focus_epoch": 0,
            "revision": 0,
            "mono_ms": 4,
            "payload": {}
        }))
        .expect("wire decoding should succeed before shape validation");

        assert!(matches!(
            envelope.validate_shape(),
            Err(ProtocolError::UnexpectedCoordinates)
        ));
        assert_eq!(envelope.message_type, MessageType::HealthRequest);
    }

    #[test]
    fn authority_messages_reject_correlation_ids_for_schema_parity() {
        for message_type in ["authority.changed", "authority.ack"] {
            let envelope: WireEnvelope = serde_json::from_value(json!({
                "v": 1,
                "id": "unexpected",
                "type": message_type,
                "mono_ms": 4,
                "payload": if message_type == "authority.changed" {
                    json!({"authority_epoch": 1, "settings_revision": 1, "paused": true})
                } else {
                    json!({"authority_epoch": 1})
                }
            }))
            .expect("wire decoding should succeed before shape validation");
            assert!(matches!(
                envelope.validate_shape(),
                Err(ProtocolError::UnexpectedId)
            ));
        }
    }

    #[test]
    fn rejects_noncanonical_session_id() {
        let result = serde_json::from_value::<WireEnvelope>(json!({
            "v": 1,
            "type": "session.open",
            "session_id": "550E8400-E29B-41D4-A716-446655440000",
            "focus_epoch": 0,
            "revision": 0,
            "mono_ms": 4,
            "payload": {}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn rejects_canonical_non_rfc_session_id_variant() {
        let result = serde_json::from_value::<WireEnvelope>(json!({
            "v": 1,
            "type": "session.open",
            "session_id": "550e8400-e29b-41d4-7716-446655440000",
            "focus_epoch": 0,
            "revision": 0,
            "mono_ms": 4,
            "payload": {}
        }));
        assert!(result.is_err());
    }

    #[test]
    fn addressed_control_rejects_unsafe_javascript_counter() {
        let base = json!({
            "v": 1,
            "type": "control.request",
            "session_id": SessionId::new(),
            "focus_epoch": 0,
            "revision": 0,
            "mono_ms": 4,
            "payload": {
                "action": "request",
                "fingerprint": "fingerprint_000000000001"
            }
        });
        let mut unsafe_focus = base.clone();
        unsafe_focus["focus_epoch"] = json!(MAX_SAFE_COUNTER + 1);
        let envelope: WireEnvelope = serde_json::from_value(unsafe_focus)
            .expect("wire decoding should succeed before shape validation");
        assert!(matches!(
            envelope.validate_shape(),
            Err(ProtocolError::CounterOutOfRange("focus_epoch"))
        ));

        let mut unsafe_revision = base;
        unsafe_revision["revision"] = json!(MAX_SAFE_COUNTER + 1);
        let envelope: WireEnvelope = serde_json::from_value(unsafe_revision)
            .expect("wire decoding should succeed before shape validation");
        assert!(matches!(
            envelope.validate_shape(),
            Err(ProtocolError::CounterOutOfRange("revision"))
        ));
    }

    #[test]
    fn language_tags_require_nonempty_ascii_alphanumeric_subtags() {
        for valid in ["en", "en-US", "zh-Hant-TW", "x-private"] {
            assert!(valid_language_tag(valid), "{valid}");
        }
        for invalid in ["en-", "-en", "en--x", "en_US", "e", "fr-ça"] {
            assert!(!valid_language_tag(invalid), "{invalid}");
        }
    }

    #[test]
    fn desktop_policy_queries_require_protocol_v2() {
        let query = PolicyQueryPayload {
            target: TargetDescriptor {
                kind: TargetKind::DesktopApplication,
                app_id: "omawrite".to_owned(),
                target_id: "ic:42".to_owned(),
                origin: None,
            },
        };

        assert!(query.validate_for_version(PROTOCOL_VERSION).is_err());
        query
            .validate_for_version(CURRENT_PROTOCOL_VERSION)
            .expect("desktop policy query is a v2 contract");
    }
}
