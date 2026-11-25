//! Hestia: a health check for a Linux laptop. Read-only, no root.

pub mod alerts;
pub mod checks;
pub mod cli;
pub mod demo;
pub mod history;
pub mod report;
pub mod web;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
