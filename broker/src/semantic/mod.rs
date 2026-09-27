//! Owned local-model runtime primitives, compiled with the `local-model`
//! feature: artifact provenance, the contained llama.cpp process and its
//! loopback client. The writing provider activates them; the historical
//! evaluator candidate stays gated by its own qualification receipt.

pub mod candidate;
pub mod client;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod process;
pub mod provenance;
pub mod runtime;
