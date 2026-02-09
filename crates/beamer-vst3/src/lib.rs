//! # beamer-vst3
//!
//! VST3 implementation layer for the Beamer framework.
//!
//! This crate provides the VST3 interface implementations that wrap `beamer-core` traits
//! into VST3 COM interfaces. It handles all the VST3-specific details like:
//!
//! - Plugin factory (IPluginFactory, IPluginFactory2, IPluginFactory3)
//! - Generic processor wrapper ([`Vst3Processor`])
//! - Platform entry points
//!
//! ## Architecture
//!
//! Uses the **combined component** pattern where processor and controller are
//! implemented by the same object. This is the modern, recommended approach
//! used by most audio plugin frameworks.
//!
//! ```text
//! User Plugin (implements beamer_core::Descriptor)
//!        ↓
//! Vst3Processor<P> (generic VST3 wrapper)
//!        ↓
//! VST3 COM interfaces (IComponent, IAudioProcessor, IEditController)
//! ```
//!
//! ## Usage
//!
//! 1. Implement `beamer_core::Descriptor` for your plugin type
//! 2. Use `export_vst3!` macro to generate entry points
//!
//! ```rust,ignore
//! use beamer_core::{Config, config::Category};
//!
//! static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
//!     .with_vendor("My Company");
//!
//! export_vst3!(CONFIG, MyGain);
//! ```

#![allow(non_upper_case_globals)]
#![allow(non_camel_case_types)]
#![allow(non_snake_case)]

pub mod export;
pub mod factory;
pub mod processor;
pub mod util;
pub mod wrapper;

// Re-exports
pub use factory::Factory;
pub use processor::Vst3Processor;

// Re-export shared types from beamer-core
pub use beamer_core::Config;
pub use beamer_core::{FactoryPresets, HasParameters, NoPresets};

// Re-export vst3 crate for use in macros and UIDs
pub use vst3;
