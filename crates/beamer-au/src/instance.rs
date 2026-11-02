//! Type-erased AU plugin instance trait.
//!
//! This module provides the `AuPluginInstance` trait which enables the native
//! AUv2 and AUv3 wrappers to work with any plugin through dynamic dispatch via
//! the C-ABI bridge.
//!
//! # Why Type Erasure?
//!
//! Rust generics don't translate to C. The wrappers that xtask generates call
//! the same C-ABI functions for every plugin, so the bridge needs one concrete
//! type to hold whatever plugin the library exports.
//!
//! The solution is type erasure: we define a trait that captures all the
//! operations we need, then use `Box<dyn AuPluginInstance>` to store any
//! plugin implementation.

use std::sync::Arc;

use crate::error::PluginResult;
use beamer_core::{
    ActiveBuses, AuxiliaryBuffers, Buffer, MidiBuffer, MidiEvent, MidiEventsMut, ProcessContext, ProcessMode,
    SharedModel,
};
use beamer_webview::{WebViewDispatch, WebViewEventQueue};

/// The factory presets of a plugin type.
///
/// The presets are static data, so the bridge captures this once when an
/// instance is created and never has to ask the plugin again.
#[derive(Clone, Copy)]
pub struct FactoryPresetAccess {
    /// Number of factory presets.
    pub count: usize,
    /// Returns the name of the preset at an index.
    pub name: fn(usize) -> Option<&'static str>,
    /// Applies the preset at an index to the parameters of the plugin.
    pub apply: fn(&dyn SharedModel, usize) -> bool,
}

impl FactoryPresetAccess {
    /// A plugin without factory presets.
    pub fn none() -> Self {
        Self {
            count: 0,
            name: |_| None,
            apply: |_, _| false,
        }
    }
}

/// Type-erased interface for AU plugin instances.
///
/// This trait enables the native wrappers to work with any `Model`
/// implementation through dynamic dispatch.
///
/// # Implementation
///
/// This trait is implemented by `AuProcessor<P>` for any `P: Model`.
/// Plugin authors don't implement this directly - they implement the
/// `beamer_core::Model` trait instead.
pub trait AuPluginInstance: Send + 'static {
    /// Allocate render resources (prepare for audio processing).
    ///
    /// Called when AU host calls `allocateRenderResourcesAndReturnError:`.
    /// The model creates a Processor for this configuration.
    ///
    /// # Arguments
    /// * `sample_rate` - The sample rate in Hz
    /// * `max_frames` - Maximum number of frames per render call
    /// * `buses` - The buses the plugin accepted
    /// * `process_mode` - Whether the host renders in real time or offline
    fn allocate_render_resources(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        buses: &ActiveBuses,
        process_mode: ProcessMode,
    ) -> PluginResult<()>;

    /// Deallocate render resources (return to unprepared state).
    ///
    /// Called when AU host calls `deallocateRenderResources`.
    /// This drops the Processor. The model stays.
    ///
    /// This method must be idempotent: calling it when already unprepared
    /// is a safe no-op. The destroy path relies on this to ensure cleanup
    /// even if a prior deallocation was skipped.
    fn deallocate_render_resources(&mut self);

    /// Check if currently in prepared state (render resources allocated).
    fn is_prepared(&self) -> bool;

    /// Get the current sample rate (only valid when prepared).
    fn sample_rate(&self) -> Option<f64>;

    /// Get the maximum frame count (only valid when prepared).
    fn max_frames(&self) -> Option<u32>;

    /// Get the model as the bridge shares it with host threads.
    ///
    /// The bridge keeps this `Arc` for the life of the instance, so that
    /// parameter values, state, groups and presets need no plugin mutex
    /// (see `SharedModel`). It must be the `Arc` that the processors of
    /// this instance use: what the host sets through it is what the audio
    /// thread reads.
    fn shared_model(&self) -> Arc<dyn SharedModel>;

    /// Reset DSP state (clear delay lines, reset filters, etc.).
    ///
    /// The render call runs this on the audio thread at the start of the
    /// block that follows a reset request of the host.
    fn reset(&mut self);

    /// Get the tail length in samples.
    ///
    /// Returns the number of samples the plugin will continue to output
    /// after input has stopped (e.g., reverb/delay tail).
    fn tail_samples(&self) -> u32;

    /// Get the processing latency in samples.
    ///
    /// Returns the latency introduced by the plugin's processing.
    /// Used by the host for delay compensation.
    fn latency_samples(&self) -> u32;

    /// Get the largest latency in samples the processor will report, 0
    /// while unprepared.
    fn max_latency_samples(&self) -> u32;

    /// Returns whether the processor can process f64 audio natively.
    ///
    /// Note: The AU wrapper may still accept float64 stream formats and convert
    /// internally even if this returns false.
    fn supports_native_double_precision(&self) -> bool;

    // =========================================================================
    // Bus Configuration (static, known before prepare)
    // =========================================================================

    /// The layouts the plugin supports, the default first (see
    /// `Model::LAYOUTS`). They give the buses of the AU bus arrays (before
    /// allocate/render).
    fn layouts(&self) -> &'static [beamer_core::Layout];

    /// The buses of the default layout with the default state of each (see
    /// `ActiveBuses::default_for`).
    fn default_buses(&self) -> ActiveBuses;

    /// The name the plugin gives an input bus of its default layout (see
    /// `Model::input_bus_info`), `None` past its input buses.
    fn input_bus_name(&self, index: usize) -> Option<&'static str>;

    /// The name the plugin gives an output bus, as `input_bus_name`.
    fn output_bus_name(&self, index: usize) -> Option<&'static str>;

    /// Whether the plugin can process with these buses on (see
    /// `Model::supports_buses`).
    fn supports_buses(&self, buses: &ActiveBuses) -> bool;

    /// Process one block of audio (f32).
    ///
    /// Only valid when prepared. Returns an error if not in prepared state.
    /// The render block builds the buffers from the host's channels without
    /// allocating, as this runs on the audio thread. MIDI reaches the plugin
    /// through [`process_midi`](Self::process_midi).
    fn process_block(
        &mut self,
        buffer: &mut Buffer<'_, f32>,
        aux: &mut AuxiliaryBuffers<'_, f32>,
        context: &ProcessContext,
    ) -> PluginResult<()>;

    /// Process one block of audio (f64).
    ///
    /// A processor without f64 support processes through pre-allocated f32
    /// buffers.
    fn process_block_f64(
        &mut self,
        buffer: &mut Buffer<'_, f64>,
        aux: &mut AuxiliaryBuffers<'_, f64>,
        context: &ProcessContext,
    ) -> PluginResult<()>;

    /// Apply parameter events from host automation.
    ///
    /// This method is called before processing audio to apply sample-accurate
    /// parameter changes from the host. The default implementation does nothing.
    ///
    /// # Arguments
    /// * `events` - Parameter value changes, ramps included (see
    ///   [`AuParameterEvent`](crate::render::AuParameterEvent))
    fn apply_parameter_events(&mut self, _events: &[crate::render::AuParameterEvent]) -> PluginResult<()> {
        // Default: do nothing
        Ok(())
    }

    /// Get reference to MIDI CC state (if configured).
    ///
    /// Returns `None` if the plugin didn't configure MIDI CC tracking via
    /// `Model::midi_cc_config()`. When `Some`, the state contains
    /// current values for all enabled MIDI controllers (CC, pitch bend, aftertouch).
    ///
    /// The framework automatically updates this state from incoming MIDI events
    /// and passes it to `ProcessContext` so plugins can query values via
    /// `context.midi_cc()`.
    fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        None // Default implementation
    }

    /// Apply the factory presets that MIDI program changes select and move
    /// the other events to the front, in their order. Returns how many
    /// events are left; those go to `process_midi()` and into the context of
    /// the `process()` call that follows.
    ///
    /// The default implementation keeps every event.
    fn apply_preset_program_changes(&mut self, events: MidiEventsMut<'_>) -> usize {
        events.len()
    }

    /// Process MIDI events (transform input into output).
    ///
    /// This method allows plugins to process, transform or generate MIDI events.
    /// The default implementation passes through all input events unchanged.
    ///
    /// # Arguments
    /// * `input` - The MIDI events of one `process()` call, timed from its
    ///   first sample, or the events no `process()` call reached, timed from
    ///   the start of the render call
    /// * `output` - Output MIDI buffer to write events to, with the events of
    ///   the earlier calls of the render call before them
    ///
    /// # Examples
    /// - MIDI effects: Transpose notes, change velocities, add effects
    /// - Instruments: Generate note-off events for voice management
    /// - Arpeggiators: Transform held notes into patterns
    fn process_midi(&mut self, input: &[MidiEvent<'_>], output: &mut MidiBuffer) {
        // Default: pass through all events
        for event in input {
            output.push(*event);
        }
    }

    // =========================================================================
    // WebView
    // =========================================================================

    /// Returns where the web UI's invoke calls and events go: the model.
    /// Parameter synchronization is automatic and does not go
    /// through here.
    fn webview_dispatch(&self) -> Arc<dyn WebViewDispatch>;

    /// Returns the plugin's queue of events for the web UI.
    ///
    /// Returns `None` if the plugin sends no events.
    fn webview_events(&self) -> Option<WebViewEventQueue> {
        None
    }

    // =========================================================================
    // Factory Presets
    // =========================================================================

    /// Returns the factory presets of the plugin.
    fn factory_presets(&self) -> FactoryPresetAccess {
        FactoryPresetAccess::none()
    }
}
