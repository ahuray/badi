//! Local, content-minimizing broker for Badi adapters.

#![allow(clippy::missing_errors_doc, clippy::missing_panics_doc)]

pub mod control_plane;
pub mod engine;
pub mod ipc;
#[cfg(feature = "local-model-eval")]
pub mod local_model;
pub mod metrics;
pub mod model_selection;
pub mod native_host;
pub mod personalization;
pub mod policy;
pub mod protocol;
pub mod provider;
pub mod segment;
#[cfg(feature = "local-model-eval")]
pub mod semantic;
pub mod server;
pub mod settings;

pub use control_plane::{ControlPlane, ControlPlaneError, ControlPlaneSnapshot};
pub use engine::{Broker, BrokerConfig, BrokerError, BrokerEvent, BrokerEventSink, ContextOutcome};
pub use metrics::{Metrics, MetricsSnapshot};
pub use model_selection::{
    HardwareProfile, ModelAdvice, ModelArtifact, ModelTier, ModelUseCase, detect_hardware,
    recommend_model,
};
pub use policy::{PolicyDecision, PolicyReason};
pub use provider::{
    CompletionProvider, DeterministicPhraseProvider, ProviderError, ProviderRequest,
};
