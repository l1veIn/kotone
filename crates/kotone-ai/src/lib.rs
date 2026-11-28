//! Shared AI infrastructure. Domain consumers own their prompts and validate
//! results; transport and credential transactions have one implementation.

pub mod agent;
pub mod companion;
pub mod connections;
pub mod model_listing;
pub mod secrets;
pub mod transport;
pub mod web;
