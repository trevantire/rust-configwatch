//! Configuration source implementations.
//!
//! This module provides built-in configuration sources:
//! - `file` — Watch configuration files (JSON, TOML)
//! - `env` — Monitor environment variables

pub mod env;
pub mod file;
