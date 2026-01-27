//! Plugin setup types for declaring host information requirements.
//!
//! Use these types with [`Plugin::Setup`](crate::Plugin::Setup) to specify what
//! information your plugin needs from the host during preparation.
//!
//! # Quick Reference
//!
//! | Type | Value | Use Case |
//! |------|-------|----------|
//! | `()` | - | Stateless plugins (gain, pan) |
//! | [`SampleRate`] | `f64` | Time-based DSP (delay, filter, envelope) |
//! | [`MaxBufferSize`] | `usize` | FFT, lookahead buffers |
//! | [`MainInputChannels`] | `u32` | Per-channel input processing |
//! | [`MainOutputChannels`] | `u32` | Per-channel output state |
//! | [`AuxInputCount`] | `usize` | Sidechain-aware processing |
//! | [`AuxOutputCount`] | `usize` | Multi-bus output |
//! | [`ProcessMode`] | enum | Quality settings for offline rendering |
//!
//! # Combining Types
//!
//! Request multiple values using tuples:
//!
//! ```ignore
//! type Setup = (SampleRate, MaxBufferSize);
//! type Setup = (SampleRate, MainOutputChannels);
//! type Setup = (SampleRate, MaxBufferSize, ProcessMode);
//! ```
//!
//! # Examples
//!
//! ```ignore
//! use beamer::setup::*;
//!
//! // Stateless plugin (gain, pan)
//! impl Plugin for Gain {
//!     type Setup = ();
//!     fn prepare(self, _: ()) -> Self { self }
//! }
//!
//! // Time-based DSP (delay, filter, envelope, smoothing)
//! impl Plugin for Delay {
//!     type Setup = SampleRate;
//!     fn prepare(self, sample_rate: SampleRate) -> DelayProcessor {
//!         let buffer_size = (2.0 * sample_rate.hz()) as usize;
//!         DelayProcessor { buffer: vec![0.0; buffer_size], /* ... */ }
//!     }
//! }
//!
//! // FFT or lookahead processing
//! impl Plugin for Fft {
//!     type Setup = (SampleRate, MaxBufferSize);
//!     fn prepare(self, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
//!         FftProcessor { fft_buffer: vec![0.0; mbs.0], /* ... */ }
//!     }
//! }
//! ```

pub use crate::plugin::{
    // Core trait
    PluginSetup,
    // Individual setup types
    AuxInputCount,
    AuxOutputCount,
    MainInputChannels,
    MainOutputChannels,
    MaxBufferSize,
    ProcessMode,
    SampleRate,
};
