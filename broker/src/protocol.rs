use std::fmt;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::DeserializeOwned};
use serde_json::Value;
use thiserror::Error;
use uuid::{Uuid, Variant};

use crate::metrics::MetricsSnapshot;

pub const PROTOCOL_VERSION: u8 = 1;
pub const MAX_SAFE_COUNTER: u64 = 9_007_199_254_740_991;
pub const MAX_FRAME_BYTES: usize = 65_536;
pub const MAX_BEFORE_CHARS: usize = 512;
pub const MAX_AFTER_CHARS: usize = 128;
pub const MAX_SUGGESTION_CHARS: usize = 64;
pub const MAX_SUGGESTION_WORDS: usize = 8;
pub const MAX_ID_CHARS: usize = 128;
pub const DEFAULT_SUGGESTION_TTL_MS: u64 = 600;

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
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    Browser,
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
        if self.v != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion(self.v));
        }
        if self.mono_ms > MAX_SAFE_COUNTER {
            return Err(ProtocolError::CounterOutOfRange("mono_ms"));
        }
        if self.id.as_deref().is_some_and(|id| !valid_opaque_id(id)) {
            return Err(ProtocolError::InvalidId);
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
        if self.min_v != PROTOCOL_VERSION || self.max_v != PROTOCOL_VERSION {
            return Err(ProtocolError::VersionNegotiationFailed);
        }
        if self.adapter.name.is_empty()
            || self.adapter.name.chars().count() > 64
            || self.adapter.version.is_empty()
            || self.adapter.version.chars().count() > 32
            || self.capabilities.is_empty()
            || self.capabilities.len() > 6
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
            || self.language.as_ref().is_some_and(|language| {
                !(2..=35).contains(&language.chars().count())
                    || !language
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
        {
            return Err(ProtocolError::InvalidPayload);
        }
        Ok(())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_revision: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_fingerprint: Option<String>,
}

impl CommitResultPayload {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        validate_fingerprint(&self.fingerprint)?;
        if !valid_opaque_id(&self.suggestion_id)
            || self
                .new_revision
                .is_some_and(|value| value > MAX_SAFE_COUNTER)
            || self.new_revision.is_some() != self.new_fingerprint.is_some()
        {
            return Err(ProtocolError::InvalidPayload);
        }
        if let Some(fingerprint) = &self.new_fingerprint {
            validate_fingerprint(fingerprint)?;
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
    pub sessions: u64,
    pub socket_mode: String,
    pub max_frame_bytes: usize,
    pub metrics: MetricsSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active: Option<ActiveLocator>,
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

    use super::{MAX_SAFE_COUNTER, MessageType, ProtocolError, SessionId, WireEnvelope};

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
}
