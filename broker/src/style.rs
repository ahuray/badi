//! Opt-in style samples: sentences the user wrote in fields whose subject
//! allows learning, sealed at rest with a key kept in the desktop keyring.
//!
//! Only the sample text is encrypted. Each record keeps its subject identity
//! and day in the clear, like the text-free personalization aggregates, so
//! revocation and retention can purge records without the key. The key never
//! touches the disk here: the session helper or `badi style on` delivers it
//! from the Secret Service, and without it nothing is read or written.
//!
//! Production prompts do not use these samples. The Lab can read an explicit
//! export to measure whether they help before any prompt uses them. This
//! protects stored text; it is not isolation from a compromised desktop.

use std::path::{Path, PathBuf};

use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::settings::{
    PrivateStorageError, RetentionPermission, SettingsV2, StableIdentity, atomic_write_private,
    read_private_limited, remove_private_file, remove_private_temporary_files,
};

pub const STYLE_SCHEMA: &str = "badi.style.v1";
pub const STYLE_FILE_NAME: &str = "style.json";
pub const MAX_STYLE_BYTES: usize = 262_144;
// Small enough that an export of the longest samples fits one 64 KiB frame.
pub const MAX_STYLE_SAMPLES: usize = 64;
pub const MAX_SAMPLES_PER_SUBJECT: usize = 16;
pub const MAX_SAMPLE_CHARS: usize = 240;
pub const MIN_SAMPLE_CHARS: usize = 24;
const KEY_ID_BYTES: usize = 16;
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 24;
/// Sentence ends that close an authored sample (Latin and Persian).
const SENTENCE_ENDS: [char; 5] = ['.', '!', '?', '؟', '…'];

/// The key the keyring holds: a random id that names it in the store, and 32
/// random bytes. Both arrive as lowercase hex.
pub struct StyleKey {
    id: String,
    cipher: XChaCha20Poly1305,
}

impl std::fmt::Debug for StyleKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StyleKey")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl StyleKey {
    pub fn from_hex(id: &str, key: &str) -> Result<Self, StyleError> {
        let id_bytes = decode_hex(id).filter(|bytes| bytes.len() == KEY_ID_BYTES);
        let key_bytes = decode_hex(key).filter(|bytes| bytes.len() == KEY_BYTES);
        let (Some(_), Some(key_bytes)) = (id_bytes, key_bytes) else {
            return Err(StyleError::InvalidKey);
        };
        let cipher =
            XChaCha20Poly1305::new_from_slice(&key_bytes).map_err(|_| StyleError::InvalidKey)?;
        Ok(Self {
            id: id.to_owned(),
            cipher,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Whether the store can be read and extended with the delivered key.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StyleKeyState {
    /// No key delivered since the broker started: nothing is read or written.
    Absent,
    Ready,
    /// The store was sealed with another key (lost or replaced). Its records
    /// stay unreadable until `forget` removes them.
    Mismatch,
}

/// One sealed record. The associated data binds the ciphertext to its key id,
/// subject and day, so no record can be moved to another subject or day.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SealedSample {
    identity: StableIdentity,
    day: u64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StyleDocument {
    schema: String,
    key_id: String,
    records: Vec<SealedSample>,
}

/// A decrypted sample, only ever returned to an explicit export.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleSample {
    pub identity: StableIdentity,
    pub day: u64,
    pub text: String,
}

/// Content-free counts for status and diagnostics.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StyleStatus {
    pub key: StyleKeyState,
    pub samples: u64,
    pub subjects: u64,
}

#[derive(Debug, Error)]
pub enum StyleError {
    #[error("invalid_style_key")]
    InvalidKey,
    #[error("style_key_unavailable")]
    KeyUnavailable,
    #[error("style_key_mismatch")]
    KeyMismatch,
    #[error("style_store_corrupt")]
    Corrupt,
    #[error("style_encryption_failed")]
    Encryption,
    #[error("style_storage: {0}")]
    Storage(#[from] PrivateStorageError),
}

#[derive(Debug)]
pub struct StyleStore {
    path: PathBuf,
    key: Option<StyleKey>,
    document: StyleDocument,
}

impl StyleStore {
    /// Opens the sealed records without a key; a corrupt file is an error the
    /// caller reports, and stays untouched until `forget`.
    pub fn open(path: PathBuf) -> Result<Self, StyleError> {
        let document = match read_private_limited(&path, MAX_STYLE_BYTES)? {
            Some(bytes) => {
                let document: StyleDocument =
                    serde_json::from_slice(&bytes).map_err(|_| StyleError::Corrupt)?;
                validate_document(&document)?;
                document
            }
            None => StyleDocument::empty(),
        };
        Ok(Self {
            path,
            key: None,
            document,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Holds the delivered key in memory for this broker's lifetime.
    pub fn set_key(&mut self, key: StyleKey) -> StyleKeyState {
        self.key = Some(key);
        self.key_state()
    }

    #[must_use]
    pub fn key_state(&self) -> StyleKeyState {
        match &self.key {
            None => StyleKeyState::Absent,
            Some(key) if self.document.records.is_empty() || self.document.key_id == key.id => {
                StyleKeyState::Ready
            }
            Some(_) => StyleKeyState::Mismatch,
        }
    }

    #[must_use]
    pub fn status(&self) -> StyleStatus {
        let mut subjects = self
            .document
            .records
            .iter()
            .map(|record| &record.identity)
            .collect::<Vec<_>>();
        subjects.sort();
        subjects.dedup();
        StyleStatus {
            key: self.key_state(),
            samples: u64::try_from(self.document.records.len()).unwrap_or(u64::MAX),
            subjects: u64::try_from(subjects.len()).unwrap_or(u64::MAX),
        }
    }

    /// Seals and appends one sample when learning and retention allow it.
    /// Returns whether the store changed. A duplicate of an existing sample
    /// of the same subject changes nothing; the oldest sample of a full
    /// subject, or of the full store, makes room.
    pub fn record(
        &mut self,
        settings: &SettingsV2,
        today: u64,
        identity: &StableIdentity,
        text: &str,
    ) -> Result<bool, StyleError> {
        if !settings.style_learning || !retains(settings, identity, today, today) {
            return Ok(false);
        }
        let key = self.key.as_ref().ok_or(StyleError::KeyUnavailable)?;
        if !self.document.records.is_empty() && self.document.key_id != key.id {
            return Err(StyleError::KeyMismatch);
        }
        for record in self
            .document
            .records
            .iter()
            .filter(|record| &record.identity == identity)
        {
            if open_sample(key, record)? == text {
                return Ok(false);
            }
        }
        let sealed = seal_sample(key, identity, today, text)?;
        let mut next = StyleDocument {
            schema: STYLE_SCHEMA.to_owned(),
            key_id: key.id.clone(),
            records: self.document.records.clone(),
        };
        let subject = next
            .records
            .iter()
            .filter(|record| &record.identity == identity)
            .count();
        if subject >= MAX_SAMPLES_PER_SUBJECT {
            remove_oldest(&mut next.records, Some(identity));
        }
        if next.records.len() >= MAX_STYLE_SAMPLES {
            remove_oldest(&mut next.records, None);
        }
        next.records.push(sealed);
        self.persist(next)?;
        Ok(true)
    }

    /// Drops every record whose subject no longer allows learning with
    /// bounded retention, or whose retention has run out. Needs no key.
    pub fn reconcile(&mut self, settings: &SettingsV2, today: u64) -> Result<bool, StyleError> {
        let records = self
            .document
            .records
            .iter()
            .filter(|record| retains(settings, &record.identity, record.day, today))
            .cloned()
            .collect::<Vec<_>>();
        if records.len() == self.document.records.len() {
            return Ok(false);
        }
        let next = StyleDocument {
            schema: STYLE_SCHEMA.to_owned(),
            key_id: self.document.key_id.clone(),
            records,
        };
        self.persist(next)?;
        Ok(true)
    }

    /// Every decrypted sample, for an explicit export only.
    pub fn export(&self) -> Result<Vec<StyleSample>, StyleError> {
        let key = self.key.as_ref().ok_or(StyleError::KeyUnavailable)?;
        if self.key_state() == StyleKeyState::Mismatch {
            return Err(StyleError::KeyMismatch);
        }
        self.document
            .records
            .iter()
            .map(|record| {
                Ok(StyleSample {
                    identity: record.identity.clone(),
                    day: record.day,
                    text: open_sample(key, record)?,
                })
            })
            .collect()
    }

    /// Removes every record and the key held in memory. The keyring item is
    /// the caller's to delete.
    pub fn forget(&mut self) -> Result<bool, StyleError> {
        let removed = remove_private_temporary_files(&self.path)?;
        let changed = remove_private_file(&self.path)? || removed;
        self.document = StyleDocument::empty();
        self.key = None;
        Ok(changed)
    }

    fn persist(&mut self, next: StyleDocument) -> Result<(), StyleError> {
        if next.records.is_empty() {
            remove_private_file(&self.path)?;
        } else {
            let bytes = serde_json::to_vec(&next).map_err(|_| StyleError::Corrupt)?;
            if bytes.len() > MAX_STYLE_BYTES {
                return Err(StyleError::Corrupt);
            }
            atomic_write_private(&self.path, &bytes)?;
        }
        self.document = next;
        Ok(())
    }
}

impl StyleDocument {
    fn empty() -> Self {
        Self {
            schema: STYLE_SCHEMA.to_owned(),
            key_id: String::new(),
            records: Vec::new(),
        }
    }
}

/// A record may exist only while its subject allows learning with bounded
/// retention, and only within that many days.
fn retains(settings: &SettingsV2, identity: &StableIdentity, day: u64, today: u64) -> bool {
    let resolution = settings.resolve_identity_validated(identity);
    let RetentionPermission::Bounded { days } = resolution.permissions.retention else {
        return false;
    };
    resolution.permissions.learn.is_allowed()
        && day <= today
        && today.saturating_sub(day) < u64::from(days)
}

fn remove_oldest(records: &mut Vec<SealedSample>, identity: Option<&StableIdentity>) {
    if let Some(index) = records
        .iter()
        .enumerate()
        .filter(|(_, record)| identity.is_none_or(|identity| &record.identity == identity))
        .min_by_key(|(_, record)| record.day)
        .map(|(index, _)| index)
    {
        records.remove(index);
    }
}

fn associated_data(
    key_id: &str,
    identity: &StableIdentity,
    day: u64,
) -> Result<Vec<u8>, StyleError> {
    serde_json::to_vec(&serde_json::json!({
        "schema": STYLE_SCHEMA, "key_id": key_id, "identity": identity, "day": day,
    }))
    .map_err(|_| StyleError::Encryption)
}

fn seal_sample(
    key: &StyleKey,
    identity: &StableIdentity,
    day: u64,
    text: &str,
) -> Result<SealedSample, StyleError> {
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let aad = associated_data(&key.id, identity, day)?;
    let ciphertext = key
        .cipher
        .encrypt(
            &nonce,
            Payload {
                msg: text.as_bytes(),
                aad: &aad,
            },
        )
        .map_err(|_| StyleError::Encryption)?;
    Ok(SealedSample {
        identity: identity.clone(),
        day,
        nonce: encode_hex(&nonce),
        ciphertext: encode_hex(&ciphertext),
    })
}

fn open_sample(key: &StyleKey, record: &SealedSample) -> Result<String, StyleError> {
    let nonce = decode_hex(&record.nonce)
        .filter(|bytes| bytes.len() == NONCE_BYTES)
        .ok_or(StyleError::Corrupt)?;
    let ciphertext = decode_hex(&record.ciphertext).ok_or(StyleError::Corrupt)?;
    let aad = associated_data(&key.id, &record.identity, record.day)?;
    let plaintext = key
        .cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| StyleError::KeyMismatch)?;
    String::from_utf8(plaintext).map_err(|_| StyleError::Corrupt)
}

fn validate_document(document: &StyleDocument) -> Result<(), StyleError> {
    let key_ok = if document.records.is_empty() {
        document.key_id.is_empty() || decode_hex(&document.key_id).is_some()
    } else {
        decode_hex(&document.key_id).is_some_and(|bytes| bytes.len() == KEY_ID_BYTES)
    };
    if document.schema != STYLE_SCHEMA
        || !key_ok
        || document.records.len() > MAX_STYLE_SAMPLES
        || document.records.iter().any(|record| {
            record.identity.validate().is_err()
                || decode_hex(&record.nonce).is_none_or(|bytes| bytes.len() != NONCE_BYTES)
                || decode_hex(&record.ciphertext).is_none()
        })
    {
        return Err(StyleError::Corrupt);
    }
    Ok(())
}

/// The most recent complete sentence the user wrote before the caret, with
/// earlier sentences of the same line only while it is shorter than
/// [`MIN_SAMPLE_CHARS`], never longer than [`MAX_SAMPLE_CHARS`]. A sample with
/// a digit, an at sign or a web address is dropped: numbers, accounts and
/// links are facts, not style. `None` when nothing complete remains.
#[must_use]
pub fn authored_sample(before: &str) -> Option<String> {
    let characters = before.char_indices().collect::<Vec<_>>();
    let mut sentences = Vec::new();
    let mut start = 0;
    for (position, &(index, character)) in characters.iter().enumerate() {
        if character == '\n' {
            sentences.clear();
            start = index + 1;
            continue;
        }
        let closes = characters
            .get(position + 1)
            .is_none_or(|(_, next)| next.is_whitespace());
        if SENTENCE_ENDS.contains(&character) && closes {
            let stop = index + character.len_utf8();
            let sentence = before[start..stop]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if !sentence.is_empty() {
                sentences.push(sentence);
            }
            start = stop;
        }
    }
    let mut sample = String::new();
    for sentence in sentences.iter().rev() {
        let candidate = if sample.is_empty() {
            sentence.clone()
        } else {
            format!("{sentence} {sample}")
        };
        if candidate.chars().count() > MAX_SAMPLE_CHARS {
            break;
        }
        sample = candidate;
        if sample.chars().count() >= MIN_SAMPLE_CHARS {
            break;
        }
    }
    let lower = sample.to_lowercase();
    let factual = sample
        .chars()
        .any(|character| character.is_ascii_digit() || character == '@')
        || lower.contains("http")
        || lower.contains("www.");
    (sample.chars().count() >= MIN_SAMPLE_CHARS && !factual).then_some(sample)
}

fn encode_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 || text.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(text.get(index..index + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests;
