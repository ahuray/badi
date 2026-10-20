//! Learned words: words of six or more letters the user wrote in an app or
//! site with context on (ADR 0010), offered to finish a word only after they
//! appeared in two separate fields.
//!
//! Each subject's words are one record sealed with the keyring key that
//! style learning uses. The subject and the day it last changed stay in the
//! clear, so turning context off for a subject, and retention, purge records
//! without the key. Without a delivered key nothing is read or written.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::settings::{
    SettingsV2, StableIdentity, atomic_write_private, read_private_limited, remove_private_file,
    remove_private_temporary_files,
};
use crate::style::{
    KEY_ID_BYTES, NONCE_BYTES, StyleError, StyleKey, StyleKeyState, decode_hex, open_text,
    seal_text,
};

pub const LEARNED_SCHEMA: &str = "badi.learned-words.v1";
pub const LEARNED_FILE_NAME: &str = "learned-words.json";
pub const MAX_LEARNED_BYTES: usize = 1_048_576;
pub const MAX_LEARNED_SUBJECTS: usize = 64;
pub const MAX_WORDS_PER_SUBJECT: usize = 512;
/// A word unused this many days is dropped; a subject unchanged as long, too.
pub const LEARNED_RETENTION_DAYS: u64 = 90;
/// Separate fields a word must appear in before it is offered, so one typo
/// or one passing word is never suggested back.
pub const OFFER_AFTER_FIELDS: u32 = 2;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SealedSubject {
    identity: StableIdentity,
    day: u64,
    nonce: String,
    ciphertext: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct LearnedDocument {
    schema: String,
    key_id: String,
    subjects: Vec<SealedSubject>,
}

/// One word, how many fields it appeared in and the day it last did.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct LearnedWord {
    word: String,
    fields: u32,
    day: u64,
}

/// Content-free counts for status and diagnostics. `words` needs the key.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LearnedStatus {
    pub key: StyleKeyState,
    pub subjects: u64,
    pub words: Option<u64>,
}

#[derive(Debug)]
pub struct LearnedStore {
    path: PathBuf,
    key: Option<StyleKey>,
    document: LearnedDocument,
}

impl LearnedStore {
    /// Opens the sealed records without a key; a corrupt file is an error the
    /// caller reports, and stays untouched until `forget`.
    pub fn open(path: PathBuf) -> Result<Self, StyleError> {
        let document = match read_private_limited(&path, MAX_LEARNED_BYTES)? {
            Some(bytes) => {
                let document: LearnedDocument =
                    serde_json::from_slice(&bytes).map_err(|_| StyleError::Corrupt)?;
                validate_document(&document)?;
                document
            }
            None => LearnedDocument::empty(),
        };
        Ok(Self {
            path,
            key: None,
            document,
        })
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
            Some(key) if self.document.subjects.is_empty() || self.document.key_id == key.id() => {
                StyleKeyState::Ready
            }
            Some(_) => StyleKeyState::Mismatch,
        }
    }

    #[must_use]
    pub fn status(&self) -> LearnedStatus {
        let words = (self.key_state() == StyleKeyState::Ready).then(|| {
            self.document
                .subjects
                .iter()
                .filter_map(|record| self.words_of(record).ok())
                .map(|words| u64::try_from(words.len()).unwrap_or(u64::MAX))
                .sum()
        });
        LearnedStatus {
            key: self.key_state(),
            subjects: u64::try_from(self.document.subjects.len()).unwrap_or(u64::MAX),
            words,
        }
    }

    /// Counts one more field for each of `words`, written in `identity`'s
    /// field today. Returns whether the store changed. Records sealed with
    /// another key are unreadable and give way to the new key's.
    pub fn record(
        &mut self,
        settings: &SettingsV2,
        today: u64,
        identity: &StableIdentity,
        words: &[String],
    ) -> Result<bool, StyleError> {
        if words.is_empty() || !settings.context_words_for(identity) {
            return Ok(false);
        }
        let key = self.key.as_ref().ok_or(StyleError::KeyUnavailable)?;
        let mut subjects = if self.key_state() == StyleKeyState::Mismatch {
            Vec::new()
        } else {
            self.document.subjects.clone()
        };
        let position = subjects
            .iter()
            .position(|record| &record.identity == identity);
        let mut known: BTreeMap<String, LearnedWord> = match position {
            Some(index) => self
                .words_of(&subjects[index])?
                .into_iter()
                .map(|word| (word.word.clone(), word))
                .collect(),
            None => BTreeMap::new(),
        };
        for word in words {
            if !crate::vocabulary::learnable(word) {
                continue;
            }
            let entry = known.entry(word.clone()).or_insert_with(|| LearnedWord {
                word: word.clone(),
                fields: 0,
                day: today,
            });
            entry.fields = entry.fields.saturating_add(1);
            entry.day = today;
        }
        let mut kept: Vec<LearnedWord> = known
            .into_values()
            .filter(|word| within_retention(word.day, today))
            .collect();
        // The rarest, then the oldest, make room.
        kept.sort_by(|left, right| {
            (right.fields, right.day, &left.word).cmp(&(left.fields, left.day, &right.word))
        });
        kept.truncate(MAX_WORDS_PER_SUBJECT);
        kept.sort_by(|left, right| left.word.cmp(&right.word));
        let plain = serde_json::to_string(&kept).map_err(|_| StyleError::Encryption)?;
        let (nonce, ciphertext) = seal_text(key, LEARNED_SCHEMA, identity, today, &plain)?;
        let sealed = SealedSubject {
            identity: identity.clone(),
            day: today,
            nonce,
            ciphertext,
        };
        if let Some(index) = position {
            subjects[index] = sealed;
        } else {
            if subjects.len() >= MAX_LEARNED_SUBJECTS {
                if let Some(oldest) = subjects
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, record)| record.day)
                    .map(|(index, _)| index)
                {
                    subjects.remove(oldest);
                }
            }
            subjects.push(sealed);
        }
        subjects.sort_by(|left, right| left.identity.cmp(&right.identity));
        let next = LearnedDocument {
            schema: LEARNED_SCHEMA.to_owned(),
            key_id: key.id().to_owned(),
            subjects,
        };
        self.persist(next)?;
        Ok(true)
    }

    /// The words `identity` offers today: seen in at least
    /// [`OFFER_AFTER_FIELDS`] fields within retention, while context is on
    /// there. Empty without a usable key.
    #[must_use]
    pub fn offered(
        &self,
        settings: &SettingsV2,
        today: u64,
        identity: &StableIdentity,
    ) -> Vec<String> {
        if !settings.context_words_for(identity) || self.key_state() != StyleKeyState::Ready {
            return Vec::new();
        }
        self.document
            .subjects
            .iter()
            .find(|record| &record.identity == identity)
            .and_then(|record| self.words_of(record).ok())
            .unwrap_or_default()
            .into_iter()
            .filter(|word| word.fields >= OFFER_AFTER_FIELDS && within_retention(word.day, today))
            .map(|word| word.word)
            .take(crate::vocabulary::MAX_LEARNED_WORDS)
            .collect()
    }

    /// Drops every subject where context is now off, and every subject
    /// unchanged past retention. Needs no key.
    pub fn reconcile(&mut self, settings: &SettingsV2, today: u64) -> Result<bool, StyleError> {
        let subjects: Vec<SealedSubject> = self
            .document
            .subjects
            .iter()
            .filter(|record| {
                !settings.context_words_off.contains(&record.identity)
                    && within_retention(record.day, today)
            })
            .cloned()
            .collect();
        if subjects.len() == self.document.subjects.len() {
            return Ok(false);
        }
        let next = LearnedDocument {
            schema: LEARNED_SCHEMA.to_owned(),
            key_id: self.document.key_id.clone(),
            subjects,
        };
        self.persist(next)?;
        Ok(true)
    }

    /// Removes every learned word. The key stays: style learning shares it.
    pub fn forget(&mut self) -> Result<bool, StyleError> {
        let removed = remove_private_temporary_files(&self.path)?;
        let changed = remove_private_file(&self.path)? || removed;
        self.document = LearnedDocument::empty();
        Ok(changed)
    }

    fn words_of(&self, record: &SealedSubject) -> Result<Vec<LearnedWord>, StyleError> {
        let key = self.key.as_ref().ok_or(StyleError::KeyUnavailable)?;
        let plain = open_text(
            key,
            LEARNED_SCHEMA,
            &record.identity,
            record.day,
            &record.nonce,
            &record.ciphertext,
        )?;
        let words: Vec<LearnedWord> =
            serde_json::from_str(&plain).map_err(|_| StyleError::Corrupt)?;
        if words.len() > MAX_WORDS_PER_SUBJECT
            || words
                .iter()
                .any(|word| !crate::vocabulary::learnable(&word.word))
        {
            return Err(StyleError::Corrupt);
        }
        Ok(words)
    }

    fn persist(&mut self, next: LearnedDocument) -> Result<(), StyleError> {
        if next.subjects.is_empty() {
            remove_private_file(&self.path)?;
        } else {
            let bytes = serde_json::to_vec(&next).map_err(|_| StyleError::Corrupt)?;
            if bytes.len() > MAX_LEARNED_BYTES {
                return Err(StyleError::Corrupt);
            }
            atomic_write_private(&self.path, &bytes)?;
        }
        self.document = next;
        Ok(())
    }
}

impl LearnedDocument {
    fn empty() -> Self {
        Self {
            schema: LEARNED_SCHEMA.to_owned(),
            key_id: String::new(),
            subjects: Vec::new(),
        }
    }
}

const fn within_retention(day: u64, today: u64) -> bool {
    day <= today && today - day < LEARNED_RETENTION_DAYS
}

fn validate_document(document: &LearnedDocument) -> Result<(), StyleError> {
    let key_ok = if document.subjects.is_empty() {
        document.key_id.is_empty() || decode_hex(&document.key_id).is_some()
    } else {
        decode_hex(&document.key_id).is_some_and(|bytes| bytes.len() == KEY_ID_BYTES)
    };
    if document.schema != LEARNED_SCHEMA
        || !key_ok
        || document.subjects.len() > MAX_LEARNED_SUBJECTS
        || document.subjects.iter().any(|record| {
            record.identity.validate().is_err()
                || decode_hex(&record.nonce).is_none_or(|bytes| bytes.len() != NONCE_BYTES)
                || decode_hex(&record.ciphertext).is_none()
        })
    {
        return Err(StyleError::Corrupt);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
