//! Core logic of Bufonidae: placement policies, accounts, index, journal and
//! quotas.
//!
//! This crate depends on neither the GUI nor the CLI.

pub mod account;
pub mod get;
pub mod index;
pub mod manager;
pub mod placement;
pub mod put;
pub mod secrets;
