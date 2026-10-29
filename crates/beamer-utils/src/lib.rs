//! Internal utilities for the Beamer audio plugin framework (AU and VST3).
//!
//! This crate provides low-level utilities shared between `beamer-core`,
//! `beamer-macros` and the bundler. It has zero external dependencies.
//!
//! # Usage
//!
//! This crate is an internal implementation detail and is not intended for direct
//! use by plugin authors. Use the `beamer` facade crate instead.
//!
//! # Contents
//!
//! - [`fnv1a_32`] - FNV-1a hash function for parameter ID generation
//! - [`web_ui`] - Where the web UI of a plugin comes from, for the export
//!   macro and the bundler

pub mod hash;
pub mod web_ui;

pub use hash::fnv1a_32;
