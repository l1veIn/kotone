//! Shared AI infrastructure. Domain consumers own their prompts and validate
//! results; transport and credential transactions have one implementation.

pub mod connections;
pub mod reminders;
pub mod secrets;
pub mod transport;
