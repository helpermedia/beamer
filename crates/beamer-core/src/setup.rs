//! Plugin setup types for declaring host information requirements.
//!
//! Use these types with [`Model::Setup`](crate::Model::Setup) to specify what
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
//! | [`ActiveBuses`] | struct | Sidechain-aware and multi-bus processing |
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
//! impl Model for GainModel {
//!     type Setup = ();
//!     fn prepare(self: Arc<Self>, _: ()) -> GainProcessor {
//!         GainProcessor { model: self }
//!     }
//! }
//!
//! // Time-based DSP (delay, filter, envelope, smoothing)
//! impl Model for DelayModel {
//!     type Setup = SampleRate;
//!     fn prepare(self: Arc<Self>, sample_rate: SampleRate) -> DelayProcessor {
//!         let buffer_size = (2.0 * sample_rate.hz()) as usize;
//!         DelayProcessor { buffer: vec![0.0; buffer_size], /* ... */ }
//!     }
//! }
//!
//! // FFT or lookahead processing
//! impl Model for FftModel {
//!     type Setup = (SampleRate, MaxBufferSize);
//!     fn prepare(self: Arc<Self>, (sr, mbs): (SampleRate, MaxBufferSize)) -> FftProcessor {
//!         FftProcessor { fft_buffer: vec![0.0; mbs.0], /* ... */ }
//!     }
//! }
//! ```

pub use crate::active_buses::ActiveBuses;
pub use crate::plugin::{
    // Core trait
    PluginSetup,
    // Individual setup types
    MainInputChannels,
    MainOutputChannels,
    MaxBufferSize,
    ProcessMode,
    SampleRate,
};
