//! Type-erased AU plugin instance trait.
//!
//! This module provides the `AuPluginInstance` trait which enables the native
//! Objective-C wrapper (`BeamerAuWrapper`) to work with any plugin through
//! dynamic dispatch via the C-ABI bridge.
//!
//! # Why Type Erasure?
//!
//! Rust generics don't translate to Objective-C. The Objective-C wrapper
//! (`BeamerAuWrapper.m`) is a single concrete class that works with all
//! plugins through dynamic dispatch.
//!
//! The solution is type erasure: we define a trait that captures all the
//! operations we need, then use `Box<dyn AuPluginInstance>` to store any
//! plugin implementation.

use std::sync::Arc;

use crate::error::{PluginError, PluginResult};
use beamer_core::{
    CachedBusConfig, MidiEvent, ParameterGroups, ParameterStore, ProcessContext, WebViewHandler,
};

/// Type-erased interface for AU plugin instances.
///
/// This trait enables the ObjC wrapper (`BeamerAuWrapper`) to work with
/// any `Descriptor` implementation through dynamic dispatch.
///
/// # Implementation
///
/// This trait is implemented by `AuProcessor<P>` for any `P: Descriptor`.
/// Plugin authors don't implement this directly - they implement the
/// `beamer_core::Descriptor` trait instead.
pub trait AuPluginInstance: Send + 'static {
    /// Allocate render resources (prepare for audio processing).
    ///
    /// Called when AU host calls `allocateRenderResourcesAndReturnError:`.
    /// This triggers the Descriptor → Processor transition.
    ///
    /// # Arguments
    /// * `sample_rate` - The sample rate in Hz
    /// * `max_frames` - Maximum number of frames per render call
    /// * `bus_config` - The cached bus configuration from the host
    fn allocate_render_resources(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        bus_config: &CachedBusConfig,
    ) -> PluginResult<()>;

    /// Deallocate render resources (return to unprepared state).
    ///
    /// Called when AU host calls `deallocateRenderResources`.
    /// This triggers the Processor → Descriptor transition.
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

    /// Get parameter store for building AUParameterTree.
    ///
    /// Returns a reference to the parameter store which provides:
    /// - Parameter count and metadata
    /// - Normalized value get/set
    /// - Value formatting
    ///
    /// # Errors
    /// Returns an error if the plugin is in an invalid state (transitioning).
    fn parameter_store(&self) -> Result<&dyn ParameterStore, PluginError>;

    /// Get mutable parameter store for setting values from host.
    ///
    /// # Errors
    /// Returns an error if the plugin is in an invalid state (transitioning).
    fn parameter_store_mut(&mut self) -> Result<&mut dyn ParameterStore, PluginError>;

    /// Get parameter groups for building hierarchical AUParameterTree.
    ///
    /// Returns a reference to the `ParameterGroups` trait object which provides:
    /// - Group count and metadata
    /// - Hierarchical group relationships
    ///
    /// # Errors
    /// Returns an error if the plugin is in an invalid state (transitioning).
    fn parameter_groups(&self) -> Result<&dyn ParameterGroups, PluginError>;

    /// Save plugin state to bytes.
    ///
    /// Uses the same format as VST3 for cross-format compatibility.
    /// Called by AU host for preset saving.
    fn save_state(&self) -> Vec<u8>;

    /// Load plugin state from bytes.
    ///
    /// Uses the same format as VST3 for cross-format compatibility.
    /// Called by AU host for preset loading.
    fn load_state(&mut self, data: &[u8]) -> PluginResult<()>;

    /// Reset DSP state (clear delay lines, reset filters, etc.).
    ///
    /// Called when the transport stops/starts or when the plugin is
    /// activated/deactivated.
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

    /// Returns whether the processor can process f64 audio natively.
    ///
    /// Note: The AU wrapper may still accept float64 stream formats and convert
    /// internally even if this returns false.
    fn supports_native_double_precision(&self) -> bool;

    // =========================================================================
    // Bus Configuration (static, known before prepare)
    // =========================================================================

    /// Returns the number of audio input buses the plugin declares.
    ///
    /// This is used during AU bus array creation (before allocate/render).
    fn declared_input_bus_count(&self) -> usize;

    /// Returns the number of audio output buses the plugin declares.
    ///
    /// This is used during AU bus array creation (before allocate/render).
    fn declared_output_bus_count(&self) -> usize;

    /// Returns information about an input bus the plugin declares.
    fn declared_input_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo>;

    /// Returns information about an output bus the plugin declares.
    fn declared_output_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo>;

    /// Process audio (f32).
    ///
    /// Only valid when prepared. Returns error if not in prepared state.
    ///
    /// # Arguments
    /// * `inputs` - Slice of input channel slices
    /// * `outputs` - Slice of mutable output channel slices
    /// * `num_samples` - Number of samples to process
    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        num_samples: usize,
    ) -> PluginResult<()>;

    /// Process audio with full process context.
    ///
    /// This is the primary processing method that includes transport, timing,
    /// and other contextual information. Plugins should prefer implementing this
    /// over the basic `process` method.
    ///
    /// # Arguments
    /// * `inputs` - Slice of input channel slices
    /// * `outputs` - Slice of mutable output channel slices
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_context(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: delegate to basic process without context
        self.process(inputs, outputs, context.num_samples)
    }

    /// Process audio (f64).
    ///
    /// Only valid when prepared. Returns error if not in prepared state.
    ///
    /// # Arguments
    /// * `inputs` - Slice of input channel slices
    /// * `outputs` - Slice of mutable output channel slices
    /// * `num_samples` - Number of samples to process
    fn process_f64(
        &mut self,
        _inputs: &[&[f64]],
        _outputs: &mut [&mut [f64]],
        _num_samples: usize,
    ) -> PluginResult<()> {
        // Default: not supported (plugin only supports f32)
        Err(PluginError::ProcessingError(
            "f64 processing not supported".to_string(),
        ))
    }

    /// Process audio with full process context (f64).
    ///
    /// This is the f64 equivalent of process_with_context.
    ///
    /// # Arguments
    /// * `inputs` - Slice of input channel slices
    /// * `outputs` - Slice of mutable output channel slices
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_context_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: delegate to basic f64 process without context
        self.process_f64(inputs, outputs, context.num_samples)
    }

    /// Process audio with auxiliary buses (f32).
    ///
    /// This method provides access to auxiliary input/output buses (e.g., sidechain inputs).
    /// Plugins that don't use auxiliary buses can rely on the default implementation which
    /// delegates to `process_with_context`.
    ///
    /// # Arguments
    /// * `inputs` - Main bus input channel slices
    /// * `outputs` - Main bus output channel slices
    /// * `aux_inputs` - Auxiliary input buses (each bus is a slice of channel slices)
    /// * `aux_outputs` - Auxiliary output buses (each bus is a slice of channel slices)
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_aux(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        _aux_inputs: &[Vec<&[f32]>],
        _aux_outputs: &mut [Vec<&mut [f32]>],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: ignore aux buses and delegate to basic process_with_context
        self.process_with_context(inputs, outputs, context)
    }

    /// Process audio with auxiliary buses (f64).
    ///
    /// This is the f64 equivalent of process_with_aux.
    ///
    /// # Arguments
    /// * `inputs` - Main bus input channel slices
    /// * `outputs` - Main bus output channel slices
    /// * `aux_inputs` - Auxiliary input buses (each bus is a slice of channel slices)
    /// * `aux_outputs` - Auxiliary output buses (each bus is a slice of channel slices)
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_aux_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        _aux_inputs: &[Vec<&[f64]>],
        _aux_outputs: &mut [Vec<&mut [f64]>],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: ignore aux buses and delegate to basic process_with_context_f64
        self.process_with_context_f64(inputs, outputs, context)
    }

    /// Process audio with auxiliary buses and MIDI (f32).
    ///
    /// This is the primary processing method for instruments and MIDI effects.
    /// Provides access to MIDI events for the current buffer.
    ///
    /// # Arguments
    /// * `inputs` - Main bus input channel slices
    /// * `outputs` - Main bus output channel slices
    /// * `aux_inputs` - Auxiliary input buses
    /// * `aux_outputs` - Auxiliary output buses
    /// * `midi_events` - MIDI events for this buffer (sorted by sample offset)
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_midi(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        aux_inputs: &[Vec<&[f32]>],
        aux_outputs: &mut [Vec<&mut [f32]>],
        _midi_events: &[MidiEvent],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: ignore MIDI and delegate to process_with_aux
        self.process_with_aux(inputs, outputs, aux_inputs, aux_outputs, context)
    }

    /// Process audio with auxiliary buses and MIDI (f64).
    ///
    /// This is the f64 equivalent of process_with_midi.
    ///
    /// # Arguments
    /// * `inputs` - Main bus input channel slices
    /// * `outputs` - Main bus output channel slices
    /// * `aux_inputs` - Auxiliary input buses
    /// * `aux_outputs` - Auxiliary output buses
    /// * `midi_events` - MIDI events for this buffer (sorted by sample offset)
    /// * `context` - Processing context with transport, sample rate, timing, etc.
    fn process_with_midi_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        aux_inputs: &[Vec<&[f64]>],
        aux_outputs: &mut [Vec<&mut [f64]>],
        _midi_events: &[MidiEvent],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Default: ignore MIDI and delegate to process_with_aux_f64
        self.process_with_aux_f64(inputs, outputs, aux_inputs, aux_outputs, context)
    }

    /// Apply parameter events from host automation.
    ///
    /// This method is called before processing audio to apply sample-accurate
    /// parameter changes from the host. The default implementation does nothing.
    ///
    /// # Arguments
    /// * `immediate` - Immediate parameter value changes
    /// * `ramps` - Ramped parameter changes for smooth automation
    fn apply_parameter_events(
        &mut self,
        _immediate: &[crate::render::AuParameterEvent],
        _ramps: &[crate::render::AuParameterRampEvent],
    ) -> PluginResult<()> {
        // Default: do nothing
        Ok(())
    }

    /// Get reference to MIDI CC state (if configured).
    ///
    /// Returns `None` if the plugin didn't configure MIDI CC tracking via
    /// `Config::midi_cc_config()`. When `Some`, the state contains
    /// current values for all enabled MIDI controllers (CC, pitch bend, aftertouch).
    ///
    /// The framework automatically updates this state from incoming MIDI events
    /// and passes it to `ProcessContext` so plugins can query values via
    /// `context.midi_cc()`.
    fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        None // Default implementation
    }

    /// Process MIDI events (input → output transformation).
    ///
    /// This method allows plugins to process, transform, or generate MIDI events.
    /// The default implementation passes through all input events unchanged.
    ///
    /// # Arguments
    /// * `input` - Input MIDI events for this buffer
    /// * `output` - Output MIDI buffer to write events to
    ///
    /// # Examples
    /// - MIDI effects: Transpose notes, change velocities, add effects
    /// - Instruments: Generate note-off events for voice management
    /// - Arpeggiators: Transform held notes into patterns
    fn process_midi(&mut self, input: &[MidiEvent], output: &mut crate::render::MidiBuffer) {
        // Default: pass through all events
        for event in input {
            output.push(event.clone());
        }
    }

    // =========================================================================
    // WebView Handler
    // =========================================================================

    /// Returns the WebView handler for custom invoke/event messages.
    ///
    /// Returns `None` if the plugin does not provide a custom handler.
    /// Parameter synchronization is automatic and does not require this.
    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        None
    }

    // =========================================================================
    // Factory Presets
    // =========================================================================

    /// Returns the number of factory presets.
    ///
    /// Returns 0 if the plugin has no factory presets.
    fn preset_count(&self) -> u32 {
        0
    }

    /// Returns preset info (number, name) for given index.
    ///
    /// Returns `None` if the index is out of range.
    fn preset_info(&self, _index: u32) -> Option<(i32, &str)> {
        None
    }

    /// Applies a factory preset by index.
    ///
    /// Only parameters specified in the preset are modified; other parameters
    /// retain their current values (sparse application).
    ///
    /// Returns `true` if the preset was applied successfully, `false` if the
    /// index is out of range.
    fn apply_preset(&self, _index: u32) -> bool {
        false
    }
}
