//! Internal utilities for the Beamer VST3 framework.
//!
//! This crate provides low-level utilities shared between `beamer-core` and
//! `beamer-macros`. All utilities are compile-time safe (`const fn` where possible)
//! and have zero external dependencies.
//!
//! # Usage
//!
//! This crate is an internal implementation detail and is not intended for direct
//! use by plugin authors. Use the `beamer` facade crate instead.
//!
//! # Contents
//!
//! - [`fnv1a_32`] - FNV-1a hash function for parameter ID generation

pub mod hash;

pub use hash::fnv1a_32;
