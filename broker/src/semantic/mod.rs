//! Feature-gated semantic evaluation primitives.
//!
//! This module is compiled only into the dedicated evaluator target. The
//! normal broker library and binary do not declare it, so none of these
//! constructors are an activation surface.

pub mod candidate;
pub mod client;
pub mod provenance;
pub mod runtime;
