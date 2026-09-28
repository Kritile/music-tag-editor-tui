//! Core functionality for inspecting and editing a local music library.

pub mod changes;
pub mod cli;
pub mod config;
pub mod domain;
pub mod duplicates;
pub mod export;
mod fsutil;
pub mod library;
pub mod online;
pub mod quarantine;
pub mod rename;
pub mod rules;
pub mod tags;
pub mod tui;
