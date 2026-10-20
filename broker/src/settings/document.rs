//! The `badi.settings.v2` document, its decoding from v1 or v2, and validation.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

use crate::protocol::{MAX_SAFE_COUNTER, TargetDescriptor, valid_linux_app_id};
use crate::spelling::SpellingLanguage;

use super::identity::{IdentityError, StableIdentity};
use super::permissions::{
    ALL_LINUX_APPS_PERMISSIONS, ALL_WEB_ORIGINS_PERMISSIONS, SubjectPermissions, SubjectRule,
};
use super::policy::{PolicyResolution, PolicySource};
use super::{MAX_ESCAPE_QUIET_SECONDS, MAX_SUBJECTS, SETTINGS_SCHEMA, SETTINGS_SCHEMA_V1};

/// The languages the writing provider can suggest in, in canonical order.
/// English, German and Persian are on by default; French, Spanish and
/// Portuguese are opt-in in [`SettingsV2::writing_languages`] and not yet qualified.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum PredictionLanguage {
    #[serde(rename = "en")]
    English,
    #[serde(rename = "de")]
    German,
    #[serde(rename = "fa")]
    Persian,
    #[serde(rename = "fr")]
    French,
    #[serde(rename = "es")]
    Spanish,
    #[serde(rename = "pt")]
    Portuguese,
}

/// The writing languages when settings name none.
pub const DEFAULT_WRITING_LANGUAGES: [PredictionLanguage; 3] = [
    PredictionLanguage::English,
    PredictionLanguage::German,
    PredictionLanguage::Persian,
];

impl PredictionLanguage {
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::German => "de",
            Self::Persian => "fa",
            Self::French => "fr",
            Self::Spanish => "es",
            Self::Portuguese => "pt",
        }
    }

    /// The writing language of a BCP 47 `tag`, by its primary subtag.
    #[must_use]
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag.split('-').next()?.to_ascii_lowercase().as_str() {
            "en" => Some(Self::English),
            "de" => Some(Self::German),
            "fa" => Some(Self::Persian),
            "fr" => Some(Self::French),
            "es" => Some(Self::Spanish),
            "pt" => Some(Self::Portuguese),
            _ => None,
        }
    }
}

/// How automatic suggestions behave while the desktop helper reports battery
/// power or the power-saver profile. Explicit requests are never changed, and
/// the model stays resident in every mode.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum BatteryMode {
    /// The same scheduling as on mains power.
    #[serde(rename = "normal")]
    Normal,
    /// The default, omitted from documents: the model starts only after a
    /// typing pause, so keys typed in quick succession never start a
    /// generation that the next key cancels.
    #[default]
    #[serde(rename = "quiet")]
    Quiet,
    /// Automatic suggestions abstain; explicit requests still ask the model.
    #[serde(rename = "explicit")]
    ExplicitOnly,
}

impl BatteryMode {
    #[must_use]
    pub const fn is_quiet(&self) -> bool {
        matches!(self, Self::Quiet)
    }
}

/// How many words a model continuation may show, within the same token and
/// time budget. Corrections, vocabulary completions and the deterministic
/// fixture ignore it.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum SuggestionLength {
    /// One or two words.
    #[serde(rename = "short")]
    Short,
    /// Up to four words, the default, omitted from documents.
    #[default]
    #[serde(rename = "normal")]
    Normal,
}

impl SuggestionLength {
    #[must_use]
    pub const fn is_normal(&self) -> bool {
        matches!(self, Self::Normal)
    }

    /// The most words a continuation of this length shows.
    #[must_use]
    pub const fn max_words(self) -> usize {
        match self {
            Self::Short => 2,
            Self::Normal => 4,
        }
    }
}

/// Settings v1 documents deserialize into this type, but it is always
/// canonical settings v2 in memory and on disk.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
// Independent persisted flags, not a state machine.
#[allow(clippy::struct_excessive_bools)]
pub struct SettingsV2 {
    pub schema: String,
    pub revision: u64,
    pub paused: bool,
    /// Opt-in browser default: an http(s) origin without an exact rule gets
    /// [`ALL_WEB_ORIGINS_PERMISSIONS`]. Exact rules win and field denial still
    /// applies. Omitted when off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_web_origins: bool,
    /// Opt-in app default (blocklist mode): a Linux app without an exact rule
    /// gets [`ALL_LINUX_APPS_PERMISSIONS`], reported as a default so adapters
    /// open only fields they can corroborate. Omitted when off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub all_linux_apps: bool,
    /// Mid-sentence suggestions. Off by default: a word after the caret
    /// abstains before any model call. Punctuation after the caret can still
    /// complete the word being typed. Omitted when off.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mid_line: bool,
    /// Seconds a dismissal (Escape) keeps its field quiet for automatic
    /// suggestions, across edits, until the focus epoch ends or an explicit
    /// request asks again. Zero keeps the adapters' own until-edit decline
    /// only. Omitted when zero.
    #[serde(default, skip_serializing_if = "escape_quiet_seconds_is_zero")]
    pub escape_quiet_seconds: u64,
    /// Automatic scheduling on battery or in the power-saver profile.
    /// Omitted when quiet, the default.
    #[serde(default, skip_serializing_if = "BatteryMode::is_quiet")]
    pub battery_mode: BatteryMode,
    /// How many words a continuation shows. Omitted when normal, the default.
    #[serde(default, skip_serializing_if = "SuggestionLength::is_normal")]
    pub suggestion_length: SuggestionLength,
    /// Opt-in style learning: encrypted samples of the user's own sentences
    /// from subjects that allow learning with bounded retention. Omitted when
    /// off. Turning it off stops collection; forgetting is a separate step.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub style_learning: bool,
    /// Opt-in context (`badi context on`): finish the word being typed with
    /// words in the same window and words the user wrote before in the app,
    /// which are kept sealed. `None` until the user chooses, when the panel
    /// asks once; `Some(false)` keeps it off.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_words: Option<bool>,
    /// Apps and sites where context stays off while it is on: no window words
    /// are read there and no words are learned or used. Canonical order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context_words_off: Vec<StableIdentity>,
    /// Per-app quality hints, independent of app/site acquisition permission.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub language_overrides: BTreeMap<String, PredictionLanguage>,
    /// Languages whose spelling is corrected, in canonical order. Omitted
    /// when only English, the default; empty turns spelling off.
    #[serde(skip_serializing_if = "only_english")]
    pub spelling_languages: Vec<SpellingLanguage>,
    /// The writing model the user chose (`badi model use ID`), by catalog
    /// id. The broker uses it at its next start when it is installed and
    /// fits; omitted, the installed model the recommendation prefers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub writing_model: Option<String>,
    /// Languages Badi suggests words in, in canonical order; a request in
    /// any other language abstains. Omitted when English, German and
    /// Persian, the default; empty turns suggestions off.
    #[serde(skip_serializing_if = "default_writing_languages")]
    pub writing_languages: Vec<PredictionLanguage>,
    /// Per-language choice to heal a trailing space (true) or keep it in the
    /// prompt (false), over the running model's reviewed default. A language
    /// without an entry follows that default. Omitted when empty.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub trailing_space_healing: BTreeMap<PredictionLanguage, bool>,
    pub subjects: Vec<SubjectRule>,
}

fn only_english(languages: &[SpellingLanguage]) -> bool {
    languages == [SpellingLanguage::English]
}

fn default_writing_languages(languages: &[PredictionLanguage]) -> bool {
    languages == DEFAULT_WRITING_LANGUAGES
}

// Serde's skip_serializing_if dictates the by-reference signature.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn escape_quiet_seconds_is_zero(value: &u64) -> bool {
    *value == 0
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SettingsDocument {
    schema: String,
    revision: u64,
    paused: bool,
    #[serde(default)]
    all_web_origins: Option<bool>,
    #[serde(default)]
    all_linux_apps: Option<bool>,
    #[serde(default)]
    mid_line: Option<bool>,
    #[serde(default)]
    escape_quiet_seconds: Option<u64>,
    #[serde(default)]
    battery_mode: Option<BatteryMode>,
    #[serde(default)]
    suggestion_length: Option<SuggestionLength>,
    #[serde(default)]
    style_learning: Option<bool>,
    #[serde(default)]
    context_words: Option<bool>,
    #[serde(default)]
    context_words_off: Option<Vec<StableIdentity>>,
    #[serde(default, deserialize_with = "present_language_overrides")]
    language_overrides: Option<BTreeMap<String, PredictionLanguage>>,
    #[serde(default)]
    spelling_languages: Option<Vec<SpellingLanguage>>,
    #[serde(default)]
    writing_model: Option<String>,
    #[serde(default)]
    writing_languages: Option<Vec<PredictionLanguage>>,
    #[serde(default)]
    trailing_space_healing: Option<BTreeMap<PredictionLanguage, bool>>,
    subjects: Vec<SubjectRule>,
}

impl SettingsDocument {
    /// The error for the first field a settings v1 document cannot name,
    /// in a fixed order.
    fn first_v2_only_field(&self) -> Option<&'static str> {
        [
            (
                self.subjects
                    .iter()
                    .any(|rule| matches!(rule.identity, StableIdentity::LinuxApp { .. })),
                "linux_app_requires_settings_v2",
            ),
            (
                self.all_web_origins.is_some(),
                "all_web_origins_requires_settings_v2",
            ),
            (
                self.all_linux_apps.is_some(),
                "all_linux_apps_requires_settings_v2",
            ),
            (self.mid_line.is_some(), "mid_line_requires_settings_v2"),
            (
                self.escape_quiet_seconds.is_some(),
                "escape_quiet_seconds_requires_settings_v2",
            ),
            (
                self.battery_mode.is_some(),
                "battery_mode_requires_settings_v2",
            ),
            (
                self.suggestion_length.is_some(),
                "suggestion_length_requires_settings_v2",
            ),
            (
                self.style_learning.is_some(),
                "style_learning_requires_settings_v2",
            ),
            (
                self.context_words.is_some() || self.context_words_off.is_some(),
                "context_words_requires_settings_v2",
            ),
            (
                self.language_overrides.is_some(),
                "language_overrides_requires_settings_v2",
            ),
            (
                self.spelling_languages.is_some(),
                "spelling_languages_requires_settings_v2",
            ),
            (
                self.writing_model.is_some(),
                "writing_model_requires_settings_v2",
            ),
            (
                self.writing_languages.is_some(),
                "writing_languages_requires_settings_v2",
            ),
            (
                self.trailing_space_healing.is_some(),
                "trailing_space_healing_requires_settings_v2",
            ),
        ]
        .into_iter()
        .find_map(|(present, error)| present.then_some(error))
    }
}

fn present_language_overrides<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<BTreeMap<String, PredictionLanguage>>, D::Error> {
    BTreeMap::deserialize(deserializer).map(Some)
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
        if decoded.schema == SETTINGS_SCHEMA_V1 {
            if let Some(error) = decoded.first_v2_only_field() {
                return Err(serde::de::Error::custom(error));
            }
        }
        Ok(Self {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: decoded.revision,
            paused: decoded.paused,
            all_web_origins: decoded.all_web_origins.unwrap_or(false),
            all_linux_apps: decoded.all_linux_apps.unwrap_or(false),
            mid_line: decoded.mid_line.unwrap_or(false),
            escape_quiet_seconds: decoded.escape_quiet_seconds.unwrap_or(0),
            battery_mode: decoded.battery_mode.unwrap_or_default(),
            suggestion_length: decoded.suggestion_length.unwrap_or_default(),
            style_learning: decoded.style_learning.unwrap_or(false),
            context_words: decoded.context_words,
            context_words_off: decoded.context_words_off.unwrap_or_default(),
            language_overrides: decoded.language_overrides.unwrap_or_default(),
            spelling_languages: decoded
                .spelling_languages
                .unwrap_or_else(|| vec![SpellingLanguage::English]),
            writing_model: decoded.writing_model,
            writing_languages: decoded
                .writing_languages
                .unwrap_or_else(|| DEFAULT_WRITING_LANGUAGES.to_vec()),
            trailing_space_healing: decoded.trailing_space_healing.unwrap_or_default(),
            subjects: decoded.subjects,
        })
    }
}

impl SettingsV2 {
    /// The writing languages and healing choices, as the provider applies them.
    #[must_use]
    pub fn language_preferences(&self) -> crate::writing::LanguagePreferences {
        let writing = |language: &PredictionLanguage| {
            crate::writing::WritingLanguage::from_tag(language.tag())
        };
        crate::writing::LanguagePreferences {
            enabled: self.writing_languages.iter().filter_map(writing).collect(),
            healing: self
                .trailing_space_healing
                .iter()
                .filter_map(|(language, heal)| Some((writing(language)?, *heal)))
                .collect(),
        }
    }

    #[must_use]
    pub fn language_override(&self, target: &TargetDescriptor) -> Option<PredictionLanguage> {
        let app = match target.app_id.as_str() {
            "chromium-browser" => "chromium",
            app => app,
        };
        self.language_overrides
            .get(&target.app_id)
            .or_else(|| self.language_overrides.get(app))
            .copied()
    }

    /// Whether context is on for `identity`: chosen globally and not turned
    /// off for it. Permission to suggest there is checked separately.
    #[must_use]
    pub fn context_words_for(&self, identity: &StableIdentity) -> bool {
        self.context_words == Some(true)
            && !self.paused
            && self.context_words_off.binary_search(identity).is_err()
    }

    #[must_use]
    pub fn deny_by_default() -> Self {
        Self {
            schema: SETTINGS_SCHEMA.to_owned(),
            revision: 0,
            paused: true,
            all_web_origins: false,
            all_linux_apps: false,
            mid_line: false,
            escape_quiet_seconds: 0,
            battery_mode: BatteryMode::Quiet,
            suggestion_length: SuggestionLength::Normal,
            style_learning: false,
            context_words: None,
            context_words_off: Vec::new(),
            language_overrides: BTreeMap::new(),
            spelling_languages: vec![SpellingLanguage::English],
            writing_model: None,
            writing_languages: DEFAULT_WRITING_LANGUAGES.to_vec(),
            trailing_space_healing: BTreeMap::new(),
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
        if self.escape_quiet_seconds > MAX_ESCAPE_QUIET_SECONDS {
            return Err(SettingsValidationError::EscapeQuietOutOfRange);
        }
        if self.language_overrides.len() > MAX_SUBJECTS
            || self
                .language_overrides
                .keys()
                .any(|app| !valid_linux_app_id(app))
        {
            return Err(SettingsValidationError::InvalidLanguageOverride);
        }
        if !self.spelling_languages.is_sorted_by(|a, b| a < b) {
            return Err(SettingsValidationError::SpellingLanguagesNotCanonical);
        }
        if !self.writing_languages.is_sorted_by(|a, b| a < b) {
            return Err(SettingsValidationError::WritingLanguagesNotCanonical);
        }
        // A catalog id; one the broker does not know is ignored at start.
        if self.writing_model.as_deref().is_some_and(|id| {
            id.is_empty()
                || id.len() > 64
                || !id.bytes().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte)
                })
        }) {
            return Err(SettingsValidationError::InvalidWritingModel);
        }
        if self.subjects.len() > MAX_SUBJECTS {
            return Err(SettingsValidationError::TooManySubjects);
        }
        if self.context_words_off.len() > MAX_SUBJECTS {
            return Err(SettingsValidationError::TooManySubjects);
        }
        for identity in &self.context_words_off {
            identity.validate()?;
        }
        if !self.context_words_off.is_sorted_by(|a, b| a < b) {
            return Err(SettingsValidationError::ContextWordsOffNotCanonical);
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
                source: PolicySource::Rule,
                permissions: self.subjects[index].permissions,
            },
            Err(_)
                if self.all_web_origins
                    && matches!(identity, StableIdentity::BrowserOrigin { .. }) =>
            {
                PolicyResolution {
                    settings_revision: self.revision,
                    paused: self.paused,
                    identity_known: true,
                    source: PolicySource::ListDefault,
                    permissions: ALL_WEB_ORIGINS_PERMISSIONS,
                }
            }
            Err(_)
                if self.all_linux_apps && matches!(identity, StableIdentity::LinuxApp { .. }) =>
            {
                PolicyResolution {
                    settings_revision: self.revision,
                    paused: self.paused,
                    identity_known: true,
                    source: PolicySource::ListDefault,
                    permissions: ALL_LINUX_APPS_PERMISSIONS,
                }
            }
            Err(_) => PolicyResolution {
                settings_revision: self.revision,
                paused: self.paused,
                identity_known: true,
                source: PolicySource::Unconfigured,
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

    /// A protocol v1 settings client can still manage its browser-origin
    /// slice, but cannot erase native rules or the all-web and all-app
    /// defaults that its schema is unable to represent.
    #[must_use]
    pub fn preserving_v2_policy_from(mut self, current: &Self) -> Self {
        self.all_web_origins = current.all_web_origins;
        self.all_linux_apps = current.all_linux_apps;
        self.mid_line = current.mid_line;
        self.escape_quiet_seconds = current.escape_quiet_seconds;
        self.battery_mode = current.battery_mode;
        self.suggestion_length = current.suggestion_length;
        self.style_learning = current.style_learning;
        self.context_words = current.context_words;
        self.context_words_off
            .clone_from(&current.context_words_off);
        self.language_overrides
            .clone_from(&current.language_overrides);
        self.spelling_languages
            .clone_from(&current.spelling_languages);
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

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
pub enum SettingsValidationError {
    #[error("invalid_language_override")]
    InvalidLanguageOverride,
    #[error("invalid_writing_model")]
    InvalidWritingModel,
    #[error("duplicate_identity")]
    DuplicateIdentity,
    #[error("escape_quiet_out_of_range")]
    EscapeQuietOutOfRange,
    #[error("context_words_off_not_canonical")]
    ContextWordsOffNotCanonical,
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
    #[error("spelling_languages_not_canonical")]
    SpellingLanguagesNotCanonical,
    #[error("writing_languages_not_canonical")]
    WritingLanguagesNotCanonical,
    #[error("subjects_not_canonical")]
    SubjectsNotCanonical,
    #[error("too_many_subjects")]
    TooManySubjects,
    #[error("unsupported_schema")]
    UnsupportedSchema,
}
