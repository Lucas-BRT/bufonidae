//! Core logic of Bufonidae: placement policies, accounts, index, journal and
//! quotas.
//!
//! This crate depends on neither the GUI nor the CLI.

pub mod account;
pub mod config;
pub mod export;
pub(crate) mod fsutil;
pub mod get;
pub mod hashing;
pub mod index;
pub mod manager;
pub mod paths;
pub mod placement;
pub mod put;
pub mod reconcile;
pub mod repair;
pub mod secrets;
