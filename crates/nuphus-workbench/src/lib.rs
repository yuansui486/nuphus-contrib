//! Public workbench state, deliberately independent of Tauri, MCP and any Agent.
//!
//! Workflow definitions use the upstream IR. This crate owns authoring revisions
//! and run identity, not a second workflow executor. Native validation/execution
//! is supplied by the host and must precede publishing/starting a run.

pub mod auth;
pub mod catalog;
pub mod edit;
#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "gateway")]
pub mod resources;
pub mod service;
pub mod store;
pub mod types;

pub use store::WorkbenchStore;
pub use types::*;

pub const API_VERSION: &str = "1";

#[cfg(test)]
mod service_tests;
#[cfg(test)]
mod tests;
