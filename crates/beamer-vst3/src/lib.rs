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
//! User Plugin (implements beamer_core::Plugin)
//!        ↓
//! Vst3Processor<P> (generic VST3 wrapper)
//!        ↓
//! VST3 COM interfaces (IComponent, IAudioProcessor, IEditController)
//! ```
//!
//! ## Usage
//!
//! 1. Implement `beamer_core::Plugin` for your plugin type
//! 2. Use `export_vst3!` macro to generate entry points
//!
//! ```rust,ignore
//! use beamer_core::{Plugin, AudioProcessor, Buffer, Parameters, ParamInfo};
//! use beamer_vst3::{export_vst3, Vst3Processor, PluginConfig, vst3};
//!
//! // Define your plugin
//! struct MyGain { parameters: MyParameters }
//!
//! impl AudioProcessor for MyGain {
//!     fn setup(&mut self, _: f64, _: usize) {}
//!     fn process(&mut self, buffer: &mut Buffer) { /* DSP here */ }
//! }
//!
//! impl Plugin for MyGain {
//!     type Parameters = MyParameters;
//!     fn parameters(&self) -> &Self::Parameters { &self.parameters }
//!     fn create() -> Self { Self { parameters: MyParameters::new() } }
//! }
//!
//! // Configure and export
//! static CONFIG: PluginConfig = PluginConfig::new(
//!     "My Plugin",
//!     vst3::uid(0x12345678, 0x9ABCDEF0, 0xABCDEF12, 0x34567890),
//! )
//! .with_vendor("My Company");
//!
//! export_vst3!(CONFIG, Vst3Processor<MyGain>);
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
pub use wrapper::PluginConfig;

// Re-export vst3 crate for use in macros and UIDs
pub use vst3;
