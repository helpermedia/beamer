//! # beamer-au
//!
//! Audio Unit (AUv3) implementation layer for the Beamer framework.
//!
//! This crate provides the Audio Unit interface implementations that wrap `beamer-core` traits
//! into macOS Audio Unit (AUv3) components. It handles all the AU-specific details like:
//!
//! - Audio component registration and factory management
//! - Native Objective-C AUAudioUnit subclass with C-ABI bridge to Rust
//! - Parameter tree mapping with bidirectional KVO callbacks
//! - Render block creation with zero-allocation audio processing
//! - MIDI input/output extraction (both MIDI 1.0 and MIDI 2.0 UMP formats)
//! - Transport state extraction (playback, recording, cycling)
//! - Auxiliary bus support (sidechain inputs/outputs)
//! - State persistence (preset save/load)
//!
//! ## Features
//!
//! - **Audio Effects**: Full support for audio effect plugins with mono/stereo/multichannel
//! - **Instruments**: MIDI input handling for synthesizers and samplers
//! - **MIDI Effects**: Process and transform MIDI events
//! - **Sidechain**: Auxiliary input/output buses for ducking, vocoder, etc.
//! - **f32/f64**: Automatic sample format detection from host
//!
//! ## Architecture
//!
//! Uses a **hybrid Objective-C/Rust architecture**:
//! - **Objective-C**: Native `AUAudioUnit` subclass (`BeamerAuWrapper`) for Apple runtime compatibility
//! - **Rust**: All DSP, parameters, and plugin logic via C-ABI bridge functions
//!
//! ```text
//! User Plugin (implements beamer_core::Plugin)
//!        ↓
//! AuProcessor<P> (generic AU wrapper)
//!        ↓
//! Box<dyn AuPluginInstance> (type erasure)
//!        ↓
//! C-ABI bridge (src/bridge.rs ↔ objc/BeamerAuBridge.h)
//!        ↓
//! BeamerAuWrapper (native ObjC AUAudioUnit subclass)
//!        ↓
//! AU host (Logic Pro, GarageBand, etc.)
//! ```
//!
//! ## Usage
//!
//! 1. Implement `beamer_core::Plugin` for your plugin type
//! 2. Use `export_au!` macro to generate AU entry points
//!
//! ```rust,ignore
//! use beamer_core::PluginConfig;
//! use beamer_au::{export_au, AuConfig, ComponentType, fourcc};
//!
//! // Shared plugin config
//! static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
//!     .with_vendor("My Company");
//!
//! // AU-specific config
//! static AU_CONFIG: AuConfig = AuConfig::new(
//!     ComponentType::Effect,
//!     fourcc!(b"Demo"),  // Manufacturer
//!     fourcc!(b"mypg"),  // Subtype
//! );
//!
//! export_au!(CONFIG, AU_CONFIG, MyPlugin);
//! ```
//!
//! ## Real-Time Safety
//!
//! The render path is designed for real-time audio processing:
//!
//! - **Zero allocation**: Pre-allocated buffer storage avoids heap allocations
//! - **Pre-allocated MIDI buffer**: 256 events capacity, no allocation during render
//! - **try_lock()**: Non-blocking mutex acquisition prevents priority inversion
//! - **Stack arrays**: Auxiliary bus pointers use compile-time sized arrays
//!
//! ## Platform Support
//!
//! This crate only compiles on macOS. On other platforms, the crate is empty
//! but still compiles to allow cross-compilation checks.

#![cfg_attr(not(target_os = "macos"), allow(unused))]

// =============================================================================
// Platform-independent modules
// =============================================================================

pub mod config;
pub mod error;

// Re-exports
pub use config::{AuConfig, ComponentType, FourCharCode};
pub use error::{PluginError, PluginResult};

// Re-export shared PluginConfig from beamer-core
pub use beamer_core::PluginConfig;

// =============================================================================
// macOS-only modules
// =============================================================================

#[cfg(target_os = "macos")]
pub mod bridge;
#[cfg(target_os = "macos")]
pub mod buffer_storage;
#[cfg(target_os = "macos")]
pub mod buffers;
#[cfg(target_os = "macos")]
pub mod bus_config;
#[cfg(target_os = "macos")]
pub mod error_helpers;
#[cfg(target_os = "macos")]
pub mod export;
#[cfg(target_os = "macos")]
pub mod factory;
#[cfg(target_os = "macos")]
pub mod instance;
#[cfg(target_os = "macos")]
pub mod lifecycle;
#[cfg(target_os = "macos")]
pub mod midi;
#[cfg(target_os = "macos")]
pub mod processor;
#[cfg(target_os = "macos")]
pub mod render;
#[cfg(target_os = "macos")]
pub mod sysex_pool;
#[cfg(target_os = "macos")]
mod transport; // Keep private for now

// Re-exports for macOS-only modules
#[cfg(target_os = "macos")]
pub use bus_config::{BusInfo, BusType, CachedBusConfig};
#[cfg(target_os = "macos")]
pub use error_helpers::{DEFAULT_CHANNEL_COUNT, DEFAULT_MAX_FRAMES, DEFAULT_SAMPLE_RATE};
#[cfg(target_os = "macos")]
pub use instance::AuPluginInstance;
#[cfg(target_os = "macos")]
pub use processor::AuProcessor;
#[cfg(target_os = "macos")]
pub use render::{AuParameterEvent, AuParameterRampEvent, ParameterEventBuffer};
#[cfg(target_os = "macos")]
pub use sysex_pool::SysExOutputPool;

// C-ABI bridge exports for hybrid AU architecture
#[cfg(target_os = "macos")]
pub use bridge::{
    BeamerAuBusConfig, BeamerAuBusInfo, BeamerAuBusType, BeamerAuInstanceHandle,
    BeamerAuParameterInfo, BeamerAuSampleFormat, BeamerInstanceHandle,
};
