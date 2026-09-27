//! Owned local-model runtime primitives, compiled with the `local-model`
//! feature: the pinned runtime release, artifact provenance, the contained
//! llama.cpp process and its loopback client. The writing provider activates
//! them.

pub mod client;
pub mod pinned_runtime;
#[cfg(target_os = "linux")]
#[doc(hidden)]
pub mod process;
pub mod provenance;
pub mod runtime;
