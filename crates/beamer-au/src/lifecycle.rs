//! AU lifecycle state and configuration builder.
//!
//! This module provides `AuState`, which holds the two parts of a plugin
//! instance:
//! - the **Model**, created with the instance and kept for its whole life
//!   in an `Arc`. It holds the parameters and answers the host's questions
//!   about buses and MIDI.
//! - the **Processor** with its buffers, which exists while render resources
//!   are allocated.
//!
//! # State Transitions
//!
//! The AU lifecycle directly maps to `AuState` transitions:
//!
//! ```text
//! Unprepared --[allocateRenderResources]--> Prepared
//!                                                 |
//! Unprepared <--[deallocateRenderResources]-----|
//! ```
//!
//! Preparing creates a processor from the model and unpreparing drops it.
//! If the sample rate, buffer size or process mode changes while prepared,
//! the processor is dropped and a new one is created for the new
//! configuration. The model is not part of this: it stays where it is.
//!
//! # Setup Extraction
//!
//! The `PluginSetup::extract` method enables generic AU setup from platform-specific
//! parameters (sample_rate, max_frames). This allows a single `AuProcessor<P>`
//! implementation to work with any setup type the plugin declares.
//!
//! Standard Beamer setups (Nothing, SampleRate, BufferSetup, FullSetup) are provided.

use std::sync::Arc;

use beamer_core::{
    BusLayout, CachedBusConfig, ConversionBuffers, MidiCcConfig, Model,
    PluginSetup, ProcessMode, Processor,
};

/// The two parts of an AU plugin instance.
///
/// This mirrors the VST3 wrapper and maps directly to AU's
/// `allocateRenderResources` / `deallocateRenderResources` lifecycle.
pub(crate) struct AuState<P: Model> {
    /// The model of the plugin. It lives as long as the instance, in an
    /// `Arc` that every processor of the instance shares.
    pub(crate) model: Arc<P>,
    /// What exists while render resources are allocated.
    pub(crate) prepared: Option<Prepared<P>>,
}

/// A processor and the resources that were allocated with it.
pub(crate) struct Prepared<P: Model> {
    pub(crate) processor: P::Processor,
    pub(crate) sample_rate: f64,
    pub(crate) max_frames: u32,
    /// Pre-allocated conversion buffers (if processor doesn't support f64).
    pub(crate) conversion_buffers: Option<ConversionBuffers>,
    /// MIDI CC state for tracking controller values.
    /// Accessed via `midi_cc_state()` method by render block for CC event processing.
    pub(crate) midi_cc_state: Option<Box<beamer_core::MidiCcState>>,
    /// Pre-allocated MIDI output buffer for process_midi(). It is boxed
    /// because it is too large for the stack of the audio thread.
    pub(crate) midi_output_buffer: Box<beamer_core::MidiBuffer>,
}

impl<P: Model> AuState<P> {
    /// Create an unprepared state around a model.
    pub fn with_model(model: Arc<P>) -> Self {
        Self {
            model,
            prepared: None,
        }
    }

    /// Check if in prepared state.
    pub fn is_prepared(&self) -> bool {
        self.prepared.is_some()
    }

    /// Get the current sample rate (only when prepared).
    pub fn sample_rate(&self) -> Option<f64> {
        self.prepared.as_ref().map(|prepared| prepared.sample_rate)
    }

    /// Get the maximum frame count (only when prepared).
    pub fn max_frames(&self) -> Option<u32> {
        self.prepared.as_ref().map(|prepared| prepared.max_frames)
    }

    /// Get reference to processor (only when prepared).
    pub fn processor(&self) -> Option<&P::Processor> {
        self.prepared.as_ref().map(|prepared| &prepared.processor)
    }

    /// Get mutable reference to processor (only when prepared).
    pub fn processor_mut(&mut self) -> Option<&mut P::Processor> {
        self.prepared.as_mut().map(|prepared| &mut prepared.processor)
    }

    /// Drop the processor and its resources. Does nothing when unprepared.
    pub fn unprepare(&mut self) {
        self.prepared = None;
    }

    /// Get reference to MIDI CC state (only when prepared).
    ///
    /// Used by render block to track MIDI CC changes and update parameter smoothing.
    pub fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        self.prepared
            .as_ref()
            .and_then(|prepared| prepared.midi_cc_state.as_deref())
    }
}

/// Build plugin setup from AU parameters.
///
/// Creates a HostSetup with all available information, then uses the
/// `PluginSetup::extract` method to extract only what the plugin needs.
fn build_setup<S: PluginSetup>(
    sample_rate: f64,
    max_frames: u32,
    layout: &BusLayout,
    process_mode: ProcessMode,
) -> S {
    use beamer_core::HostSetup;

    let host_setup = HostSetup::new(sample_rate, max_frames as usize, layout.clone(), process_mode);

    S::extract(&host_setup)
}

/// Allocate processing resources (conversion buffers, MIDI state) for a processor.
fn allocate_processing_resources(
    supports_f64: bool,
    midi_cc_config: Option<MidiCcConfig>,
    max_frames: u32,
    layout: &BusLayout,
    bus_config: &CachedBusConfig,
) -> (
    Option<ConversionBuffers>,
    Option<Box<beamer_core::MidiCcState>>,
    Box<beamer_core::MidiBuffer>,
) {
    // Pre-allocate conversion buffers if processor doesn't support f64
    let conversion_buffers = if !supports_f64 {
        let input_channels = layout.main_input_channels as usize;
        let output_channels = layout.main_output_channels as usize;

        // Build aux bus channel counts from CachedBusConfig
        // Skip main bus (index 0) to get aux buses only
        let aux_input_channels: Vec<usize> = bus_config
            .input_buses
            .iter()
            .skip(1)
            .map(|bus_info| bus_info.channel_count)
            .collect();
        let aux_output_channels: Vec<usize> = bus_config
            .output_buses
            .iter()
            .skip(1)
            .map(|bus_info| bus_info.channel_count)
            .collect();

        Some(ConversionBuffers::allocate(
            input_channels,
            output_channels,
            &aux_input_channels,
            &aux_output_channels,
            max_frames as usize,
        ))
    } else {
        None
    };

    // Initialize MIDI CC state from plugin config
    let midi_cc_state =
        midi_cc_config.map(|cfg| Box::new(beamer_core::MidiCcState::from_config(&cfg)));

    // Pre-allocate MIDI output buffer for process_midi().
    // Uses new_boxed() to construct directly on the heap, avoiding a ~80KB
    // stack temporary that could overflow in debug builds.
    let midi_output_buffer = beamer_core::MidiBuffer::new_boxed();

    (conversion_buffers, midi_cc_state, midi_output_buffer)
}

impl<P: Model> AuState<P> {
    /// Create a processor for a configuration.
    ///
    /// Accepts `CachedBusConfig` to derive actual aux bus channel counts for
    /// proper conversion buffer allocation. A prepared plugin is prepared
    /// again, which is how a changed sample rate, buffer size or process mode
    /// reaches it.
    pub fn prepare(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        bus_config: &CachedBusConfig,
        process_mode: ProcessMode,
    ) {
        // Convert CachedBusConfig to BusLayout for plugin config
        let layout = bus_config.to_bus_layout();

        // A processor of an earlier configuration goes first, so that its
        // buffers are free before the new ones are allocated.
        if self.prepared.take().is_some() {
            log::debug!("Re-preparing plugin due to config change (was prepared)");
        }

        let plugin_setup = build_setup::<P::Setup>(sample_rate, max_frames, &layout, process_mode);
        // The smoothers ramp at the host's rate unless the plugin sets
        // another one in prepare.
        self.model.set_sample_rate(sample_rate);
        let processor = Arc::clone(&self.model).prepare(plugin_setup);

        let (conversion_buffers, midi_cc_state, midi_output_buffer) =
            allocate_processing_resources(
                processor.supports_double_precision(),
                self.model.midi_cc_config(),
                max_frames,
                &layout,
                bus_config,
            );

        self.prepared = Some(Prepared {
            processor,
            sample_rate,
            max_frames,
            conversion_buffers,
            midi_cc_state,
            midi_output_buffer,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_setup_carries_the_process_mode() {
        let layout = BusLayout::default();
        for mode in [ProcessMode::Realtime, ProcessMode::Offline] {
            assert_eq!(build_setup::<ProcessMode>(48000.0, 512, &layout, mode), mode);
        }
    }
}
