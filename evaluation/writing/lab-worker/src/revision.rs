//! The revision lane (ADR 0012): one selection per request, sent through the
//! broker's own revision request ([`complete`]) to the revision runtime the
//! broker would start, then judged by the broker's gate ([`accept`]). Expected
//! corrections and facts stay with the Node runner: a request carries only
//! what an adapter sends the broker.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use badi_broker::engine::REVISION_TIMEOUT;
use badi_broker::model_selection::current_memory;
use badi_broker::protocol::{MAX_AFTER_CHARS, MAX_BEFORE_CHARS};
use badi_broker::provider::CompletionProvider as _;
use badi_broker::revision::runtime::{PROFILE, complete, revision_model, token_budget};
use badi_broker::revision::service::{RevisionCall, RevisionError, RevisionReply};
use badi_broker::revision::{
    MAX_SELECTED_SCALARS, RevisionLanguage, RevisionMode, accept, changed_word_share,
    safe_revision_text,
};
use badi_broker::semantic::runtime::OwnedRuntime;
use badi_broker::writing;
use serde::{Deserialize, Serialize};

use crate::{LabError, valid_text};

pub const REQUEST_SCHEMA: &str = "badi.prediction-lab.revision-request.v1";

/// One selection to revise, with the text around it on its line.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevisionRequest {
    pub schema: String,
    pub id: String,
    pub language: String,
    pub mode: RevisionMode,
    #[serde(default)]
    pub before: String,
    pub selected: String,
    #[serde(default)]
    pub after: String,
}

impl RevisionRequest {
    /// The revision language of a request the broker would admit: the
    /// protocol's bounds on the selection and its surroundings.
    pub fn validate(&self) -> Result<&'static RevisionLanguage, LabError> {
        let selected = self.selected.chars().count();
        if self.schema != REQUEST_SCHEMA
            || self.id.is_empty()
            || self.id.len() > 128
            || self.id.chars().any(char::is_control)
            || !(1..=MAX_SELECTED_SCALARS).contains(&selected)
            || self.selected.trim().is_empty()
            || !safe_revision_text(&self.selected)
            || !valid_text(&self.selected, MAX_SELECTED_SCALARS)
            || !valid_text(&self.before, MAX_BEFORE_CHARS)
            || !valid_text(&self.after, MAX_AFTER_CHARS)
        {
            return Err(LabError::InvalidRequest);
        }
        badi_broker::revision::language(&self.language).ok_or(LabError::UnsupportedLanguage)
    }
}

/// What one revision request produced, as the broker would have decided it.
#[derive(Debug, Serialize)]
pub struct RevisionRecord {
    pub r#type: &'static str,
    pub id: String,
    pub mode: RevisionMode,
    pub language: &'static str,
    /// `shown`, the gate's abstention class, or the engine's name for a
    /// truncated answer or a failed call.
    pub outcome: &'static str,
    /// The replacement the adapter would show.
    pub text: Option<String>,
    /// The model's answer before the gate.
    pub output: Option<String>,
    /// Against the shown text, or the answer when nothing is shown.
    pub changed_word_share: Option<f64>,
    pub latency_ms: f64,
    pub generated_tokens: Option<u32>,
    pub token_budget: u16,
    pub truncated: bool,
    /// ISO 639-3 code of the answer's language when the broker's detector is
    /// confident, whatever language it is.
    pub output_language: Option<&'static str>,
    /// The broker's field-language choice for the text before and in the
    /// selection; when present, it decides the prompt's language in
    /// production instead of the field's own.
    pub field_language: Option<&'static str>,
}

/// The installed revision model on the pinned runtime with the broker's
/// revision launch. Contained, so it cannot outlive this worker; await it
/// on the thread that lives as long as the process.
pub async fn activate(directory: PathBuf) -> Result<OwnedRuntime, LabError> {
    let model = revision_model().ok_or(LabError::RuntimeUnavailable)?;
    // The Lab reports a host short of memory right now instead of waiting.
    if !writing::memory_fits(model, current_memory().available_mib) {
        return Err(LabError::RuntimeUnavailable);
    }
    let threads = writing::writing_threads().map_err(|_| LabError::RuntimeUnavailable)?;
    writing::verify_launch(directory, model, threads)
        .await
        .map_err(|_| LabError::RuntimeUnavailable)?
        .with_writing_profile(PROFILE)
        .map_err(|_| LabError::RuntimeUnavailable)?
        .for_revision()
        .contained()
        .spawn()
        .await
        .map_err(|_| LabError::RuntimeUnavailable)
}

/// Revises one selection within the broker's revision timeout.
pub async fn run(
    runtime: &OwnedRuntime,
    request: &RevisionRequest,
) -> Result<RevisionRecord, LabError> {
    let language = request.validate()?;
    if !runtime.is_alive() {
        return Err(LabError::RuntimeUnavailable);
    }
    let call = RevisionCall {
        mode: request.mode,
        language,
        before: &request.before,
        selected: &request.selected,
        timeout: REVISION_TIMEOUT,
    };
    let started = Instant::now();
    let reply = complete(runtime, call, REVISION_TIMEOUT).await;
    Ok(record(request, language, reply, started.elapsed()))
}

/// The engine's decision on `reply`: a truncated answer is never shown,
/// and any other answer goes through the gate with the selection's
/// surroundings, so their case and final-mark rules are measured too.
fn record(
    request: &RevisionRequest,
    language: &'static RevisionLanguage,
    reply: Result<RevisionReply, RevisionError>,
    elapsed: Duration,
) -> RevisionRecord {
    let (outcome, text) = match &reply {
        Err(RevisionError::Timeout) => ("timeout", None),
        Err(RevisionError::Unavailable) => ("unavailable", None),
        Err(RevisionError::Failed | RevisionError::Cancelled) => ("failed", None),
        Ok(reply) if reply.truncated => ("truncated", None),
        Ok(reply) => match accept(
            request.mode,
            language,
            &request.before,
            &request.selected,
            &request.after,
            &reply.output,
        ) {
            Ok(text) => ("shown", Some(text)),
            Err(abstain) => (abstain.name(), None),
        },
    };
    let reply = reply.ok();
    let output = reply.as_ref().map(|reply| reply.output.clone());
    let selected = request.selected.trim();
    let compared = text
        .as_deref()
        .or_else(|| output.as_deref().map(str::trim))
        .map(str::trim);
    RevisionRecord {
        r#type: "revision_result",
        id: request.id.clone(),
        mode: request.mode,
        language: language.tag,
        outcome,
        changed_word_share: compared.map(|revised| changed_word_share(selected, revised)),
        output_language: output
            .as_deref()
            .and_then(badi_broker::language::confident_language)
            .map(|detected| detected.code()),
        field_language: badi_broker::language::detect_field_language(&format!(
            "{}{}",
            request.before, request.selected
        )),
        text,
        output,
        latency_ms: elapsed.as_secs_f64() * 1000.0,
        generated_tokens: reply.as_ref().and_then(|reply| reply.generated_tokens),
        token_budget: token_budget(selected.chars().count()),
        truncated: reply.is_some_and(|reply| reply.truncated),
    }
}

#[cfg(test)]
mod tests;
