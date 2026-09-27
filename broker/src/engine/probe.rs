use std::sync::Arc;
use std::time::Instant;

use tokio::time;
use tokio_util::sync::CancellationToken;

use super::generation::checked_proposal;
use super::{Broker, BrokerError, duration_millis};
use crate::metrics::NoSuggestionReason;
use crate::protocol::{ProbeRequestPayload, ProbeResultPayload};
use crate::provider::{
    ProviderError, ProviderOutcome, ProviderRequest, RequestTrigger, WritingProposal,
};

impl Broker {
    /// Runs one private diagnostic request through the provider and the same
    /// display checks as a suggestion. It opens no session, creates no commit
    /// authority, changes no counters, and neither logs nor retains the text.
    pub async fn probe(
        &self,
        payload: ProbeRequestPayload,
    ) -> Result<ProbeResultPayload, BrokerError> {
        payload.validate()?;
        let provider = self.inner.provider_kind;
        if self.is_paused().await {
            return Ok(ProbeResultPayload::paused(provider));
        }
        if self.inner.shutdown.is_cancelled() {
            return Err(BrokerError::ShuttingDown);
        }
        let _permit = Arc::clone(&self.inner.provider_admissions)
            .try_acquire_owned()
            .map_err(|_| BrokerError::ProviderBusy)?;
        let started = Instant::now();
        let answer = self.ask_provider(payload, started).await?;
        let latency_ms = duration_millis(started.elapsed());
        // A pause that arrived during inference withholds the result, as it
        // withholds display from a real session.
        if self.is_paused().await {
            return Ok(ProbeResultPayload::paused(provider));
        }
        Ok(match answer {
            Ok(proposal) => ProbeResultPayload::suggested(
                provider,
                proposal.text,
                proposal.replace_before,
                latency_ms,
            ),
            Err(reason) => ProbeResultPayload::no_suggestion(provider, reason, latency_ms),
        })
    }

    /// The displayable proposal, or why there is none, within the same
    /// deadlines a suggestion gets.
    async fn ask_provider(
        &self,
        payload: ProbeRequestPayload,
        started: Instant,
    ) -> Result<Result<WritingProposal, NoSuggestionReason>, BrokerError> {
        let ProbeRequestPayload {
            before,
            after,
            language,
            allow_replacement,
            explicit,
        } = payload;
        let trigger = RequestTrigger::from_explicit(explicit);
        let generation_deadline = started + self.inner.config.generation_timeout_for(trigger);
        let deadline = generation_deadline.min(started + self.inner.config.provider_timeout);
        let cancellation = CancellationToken::new();
        let request = ProviderRequest {
            before: before.clone(),
            after: after.clone(),
            language,
        };
        let result = tokio::select! {
            () = self.inner.shutdown.cancelled() => {
                cancellation.cancel();
                return Err(BrokerError::ShuttingDown);
            }
            result = time::timeout_at(
                deadline.into(),
                self.inner.provider.propose_outcome(
                    request,
                    cancellation.clone(),
                    allow_replacement,
                    trigger,
                ),
            ) => result,
        };
        Ok(match result {
            _ if Instant::now() >= generation_deadline => {
                cancellation.cancel();
                Err(NoSuggestionReason::Timeout)
            }
            Err(_) => {
                cancellation.cancel();
                Err(NoSuggestionReason::Timeout)
            }
            Ok(Err(ProviderError::Cancelled)) => Err(NoSuggestionReason::Stale),
            Ok(Err(ProviderError::Unavailable)) => Err(NoSuggestionReason::ProviderError),
            Ok(Ok(ProviderOutcome::NoSuggestion(reason))) => Err(reason),
            Ok(Ok(ProviderOutcome::Proposal(raw))) => {
                checked_proposal(self.inner.provider_kind, &before, &after, raw)
                    .filter(|proposal| allow_replacement || proposal.replace_before.is_none())
                    .ok_or(NoSuggestionReason::OutputRejected)
            }
        })
    }
}
