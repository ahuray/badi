//! Explicit, memory-only spelling diagnostics. No broker or editor authority.

pub mod artifact;
pub mod engine;
mod identity;
mod policy;
mod worker;

pub use worker::run;

use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

pub const REQUEST_SCHEMA: &str = "badi.spelling-lab.request.v1";
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const BUDGET_MS: u64 = 550;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub schema: String,
    pub id: String,
    pub before: String,
    pub language: String,
    #[serde(default)]
    pub protected_words: Vec<String>,
}

impl Request {
    pub fn validate(&self, language: &str) -> Result<(), &'static str> {
        if self.schema != REQUEST_SCHEMA
            || self.id.is_empty()
            || self.id.len() > 128
            || self.id.chars().any(char::is_control)
            || !super::valid_text(&self.before, 2048)
            || self.protected_words.len() > 32
            || self.protected_words.iter().any(|word| {
                word.is_empty() || word.chars().count() > 24 || !policy::word_valid(word, language)
            })
            || self.language.len() > 35
            || !self.language.split('-').all(|part| {
                !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_alphanumeric())
            })
            || !self
                .language
                .split('-')
                .next()
                .is_some_and(|tag| tag.eq_ignore_ascii_case(language))
        {
            return Err("invalid_request");
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct ResultRecord {
    pub r#type: &'static str,
    pub schema: &'static str,
    pub id: String,
    pub outcome: &'static str,
    pub reason: &'static str,
    pub text: Option<String>,
    pub replace_before: Option<String>,
    pub filtered_candidate_count: usize,
    pub query_count: u32,
    pub query_ms: f64,
    pub latency_ms: f64,
    pub identity: artifact::Identity,
    pub identity_sha256: String,
    pub warnings: [&'static str; 2],
}

pub async fn check(
    engine: &mut engine::OwnedEngine,
    request: Request,
    cancellation: CancellationToken,
) -> Result<ResultRecord, &'static str> {
    let started = std::time::Instant::now();
    let deadline = started + std::time::Duration::from_millis(BUDGET_MS);
    request.validate(&engine.identity().language)?;
    let mut record = ResultRecord {
        r#type: "result",
        schema: "badi.spelling-lab.result.v1",
        id: request.id.clone(),
        outcome: "abstention",
        reason: "no_completed_word",
        text: None,
        replace_before: None,
        filtered_candidate_count: 0,
        query_count: 0,
        query_ms: 0.0,
        latency_ms: 0.0,
        identity: engine.identity().clone(),
        identity_sha256: engine.identity().sha256(),
        warnings: [
            "returned_candidates_not_exhaustive",
            "unknown_proper_names_not_detected",
        ],
    };
    let decision = async {
        if cancellation.is_cancelled() {
            return Err("cancelled");
        }
        let Some(word) = policy::target(&request.before, &engine.identity().language) else {
            return Ok(());
        };
        if request
            .protected_words
            .iter()
            .any(|protected| protected == word)
        {
            record.reason = "protected_word";
            return Ok(());
        }
        record.query_count += 1;
        let response = engine.query(word, deadline, cancellation.clone()).await?;
        let candidates = match response {
            engine::Response::Accepted => {
                record.reason = "dictionary_accepts_original";
                return Ok(());
            }
            engine::Response::Suggestions(candidates) => candidates,
        };
        let filtered = policy::filter(word, candidates, &engine.identity().language);
        record.filtered_candidate_count = filtered.len();
        record.reason = if filtered.is_empty() {
            "no_admissible_candidate"
        } else {
            "ambiguous_candidates"
        };
        if filtered.len() != 1 {
            return Ok(());
        }
        let corrected = &filtered[0];
        record.query_count += 1;
        if engine
            .query(corrected, deadline, cancellation.clone())
            .await?
            != engine::Response::Accepted
        {
            record.reason = "candidate_not_accepted";
            return Ok(());
        }
        if cancellation.is_cancelled() {
            return Err("cancelled");
        }
        if std::time::Instant::now() >= deadline {
            return Err("query_deadline");
        }
        record.outcome = "suggestion";
        record.reason = "single_admissible_returned_candidate";
        record.text = Some(format!("{corrected} "));
        record.replace_before = Some(format!("{word} "));
        Ok(())
    }
    .await;
    record.latency_ms = started.elapsed().as_secs_f64() * 1000.0;
    record.query_ms = engine.take_query_ms();
    match decision {
        Ok(()) => Ok(record),
        Err("query_deadline") => {
            record.outcome = "deadline";
            record.reason = "query_deadline";
            Ok(record)
        }
        Err(error) => Err(error),
    }
}
