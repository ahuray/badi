//! Local, content-minimizing broker for Omatype adapters.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod engine;
pub mod ipc;
pub mod metrics;
pub mod policy;
pub mod protocol;
pub mod provider;
pub mod segment;
pub mod server;

pub use engine::{Broker, BrokerConfig, BrokerError, BrokerEvent, ContextOutcome};
pub use metrics::{Metrics, MetricsSnapshot};
pub use policy::{PolicyDecision, PolicyReason};
pub use provider::{
    CompletionProvider, DeterministicPhraseProvider, ProviderError, ProviderRequest,
};
