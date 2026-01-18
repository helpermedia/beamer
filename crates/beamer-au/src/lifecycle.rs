//! AU lifecycle state machine and configuration builder.
//!
//! This module provides the `AuState` enum that manages the two-phase lifecycle:
//! - **Unprepared**: Plugin created, parameters available, but no audio processing
//! - **Prepared**: Resources allocated, ready for audio processing
//!
//! # Design Philosophy
//!
//! The state machine mirrors the VST3 wrapper's pattern, providing clean separation
//! between plugin configuration (unprepared) and audio processing (prepared) phases.
//! This design ensures that audio resources are only allocated when needed, and that
//! parameters remain accessible before and after allocation.
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
//! If the sample rate or buffer size changes while prepared, the state machine
//! automatically unprepares and re-prepares to adapt to the new configuration.
//!
//! # Configuration Building
//!
//! The `BuildAuConfig` sealed trait enables generic AU setup from platform-specific
//! parameters (sample_rate, max_frames). This allows a single `AuProcessor<P>`
//! implementation to work with any `ProcessorConfig` type, as long as the config
//! type implements `BuildAuConfig`.
//!
//! Standard Beamer configs (NoConfig, AudioSetup, FullAudioSetup) are provided.

use beamer_core::{
    AudioProcessor, AudioSetup, CachedBusConfig, ConversionBuffers, FullAudioSetup, HasParameters,
    NoConfig, Plugin, ProcessorConfig,
};
use log;

/// AU lifecycle states with clean transitions.
///
/// This mirrors the VST3 state machine and maps directly to AU's
/// `allocateRenderResources` / `deallocateRenderResources` lifecycle.
pub(crate) enum AuState<P: Plugin> {
    /// Plugin created but not prepared for audio.
    ///
    /// In this state:
    /// - Parameters are accessible
    /// - Audio processing is not possible
    /// - Bus configuration can be queried
    /// - State can be loaded but deferred until prepare()
    Unprepared {
        plugin: P,
        pending_state: Option<Vec<u8>>,
    },

    /// Resources allocated, ready to process audio.
    ///
    /// In this state:
    /// - Parameters are accessible (through processor)
    /// - Audio processing is possible
    /// - Sample rate and max frames are known
    Prepared {
        processor: P::Processor,
        sample_rate: f64,
        max_frames: u32,
        /// Pre-allocated conversion buffers (if processor doesn't support f64)
        conversion_buffers: Option<ConversionBuffers>,
        /// MIDI CC state for tracking controller values (boxed to reduce enum size).
        /// Accessed via `midi_cc_state()` method by render block for CC event processing.
        midi_cc_state: Option<Box<beamer_core::MidiCcState>>,
        /// Pre-allocated MIDI output buffer for process_midi() (boxed to reduce enum size)
        midi_output_buffer: Box<beamer_core::MidiBuffer>,
    },

    /// Temporary state during transitions.
    ///
    /// This state should never be observed externally. It exists only
    /// to satisfy Rust's ownership rules during state transitions.
    Transitioning,
}

impl<P: Plugin> AuState<P> {
    /// Create a new state machine in unprepared state.
    pub fn new() -> Self {
        Self::Unprepared {
            plugin: P::default(),
            pending_state: None,
        }
    }

    /// Check if in prepared state.
    pub fn is_prepared(&self) -> bool {
        matches!(self, Self::Prepared { .. })
    }

    /// Get the current sample rate (only when prepared).
    pub fn sample_rate(&self) -> Option<f64> {
        match self {
            Self::Prepared { sample_rate, .. } => Some(*sample_rate),
            _ => None,
        }
    }

    /// Get the maximum frame count (only when prepared).
    pub fn max_frames(&self) -> Option<u32> {
        match self {
            Self::Prepared { max_frames, .. } => Some(*max_frames),
            _ => None,
        }
    }

    /// Get reference to processor (only when prepared).
    pub fn processor(&self) -> Option<&P::Processor> {
        match self {
            Self::Prepared { processor, .. } => Some(processor),
            _ => None,
        }
    }

    /// Get mutable reference to processor (only when prepared).
    pub fn processor_mut(&mut self) -> Option<&mut P::Processor> {
        match self {
            Self::Prepared { processor, .. } => Some(processor),
            _ => None,
        }
    }

    /// Transition from Prepared to Unprepared.
    pub fn unprepare(&mut self) -> Result<(), String> {
        let old_state = std::mem::replace(self, Self::Transitioning);

        match old_state {
            Self::Prepared { processor, .. } => {
                let plugin = processor.unprepare();
                *self = Self::Unprepared {
                    plugin,
                    pending_state: None,
                };
                Ok(())
            }
            Self::Unprepared {
                plugin,
                pending_state,
            } => {
                *self = Self::Unprepared {
                    plugin,
                    pending_state,
                };
                Ok(()) // Already unprepared, no-op
            }
            Self::Transitioning => Err("Invalid state: transitioning".to_string()),
        }
    }

    /// Get reference to MIDI CC state (only when prepared).
    ///
    /// Used by render block to track MIDI CC changes and update parameter smoothing.
    pub fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        match self {
            Self::Prepared { midi_cc_state, .. } => midi_cc_state.as_deref(),
            _ => None,
        }
    }
}

impl<P: Plugin> Default for AuState<P> {
    fn default() -> Self {
        Self::new()
    }
}

/// Sealed trait for building processor config from AU setup.
///
/// This is an internal trait that allows us to have a single implementation
/// of `AuPluginInstance` that works with any `ProcessorConfig`.
pub(crate) trait BuildAuConfig: ProcessorConfig {
    fn build_au(sample_rate: f64, max_frames: u32, layout: beamer_core::BusLayout) -> Self;
}

impl BuildAuConfig for NoConfig {
    fn build_au(_sample_rate: f64, _max_frames: u32, _layout: beamer_core::BusLayout) -> Self {
        NoConfig
    }
}

impl BuildAuConfig for AudioSetup {
    fn build_au(sample_rate: f64, max_frames: u32, _layout: beamer_core::BusLayout) -> Self {
        AudioSetup {
            sample_rate,
            max_buffer_size: max_frames as usize,
        }
    }
}

impl BuildAuConfig for FullAudioSetup {
    fn build_au(sample_rate: f64, max_frames: u32, layout: beamer_core::BusLayout) -> Self {
        FullAudioSetup {
            sample_rate,
            max_buffer_size: max_frames as usize,
            layout,
        }
    }
}

// Generic prepare for any plugin with BuildAuConfig
//
// Clippy Allow: private_bounds
//
// `BuildAuConfig` is a sealed trait (pub(crate)) that restricts which config types can be used
// with AU plugins. This is intentional API design - we only support NoConfig, AudioSetup, and
// FullAudioSetup. The trait bound must be private because the trait itself is crate-private.
// Public consumers don't need to see or implement BuildAuConfig.
#[allow(private_bounds)]
impl<P: Plugin> AuState<P>
where
    P::Config: BuildAuConfig,
{
    /// Transition from Unprepared to Prepared using BuildAuConfig.
    ///
    /// Accepts `CachedBusConfig` to derive actual aux bus channel counts for
    /// proper conversion buffer allocation.
    pub fn prepare(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        bus_config: &CachedBusConfig,
    ) -> Result<(), String> {
        // Convert CachedBusConfig to BusLayout for plugin config
        let layout = bus_config.to_bus_layout();
        let old_state = std::mem::replace(self, Self::Transitioning);

        match old_state {
            Self::Unprepared {
                plugin,
                pending_state,
            } => {
                // Capture MIDI CC config before consuming the plugin
                let midi_cc_config = plugin.midi_cc_config();

                let config = P::Config::build_au(sample_rate, max_frames, layout.clone());
                let mut processor = plugin.prepare(config);

                // Apply any pending state that was set before preparation
                if let Some(data) = pending_state {
                    if let Err(e) = processor.load_state(&data) {
                        log::warn!("Failed to load pending state: {:?}", e);
                    }
                    use beamer_core::parameter_types::Parameters;
                    processor.parameters_mut().set_sample_rate(sample_rate);
                    processor.parameters_mut().reset_smoothing();
                }

                // Pre-allocate conversion buffers if processor doesn't support f64
                let conversion_buffers = if !processor.supports_double_precision() {
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

                // Pre-allocate MIDI output buffer for process_midi()
                let midi_output_buffer = Box::new(beamer_core::MidiBuffer::new());

                *self = Self::Prepared {
                    processor,
                    sample_rate,
                    max_frames,
                    conversion_buffers,
                    midi_cc_state,
                    midi_output_buffer,
                };
                Ok(())
            }
            Self::Prepared { processor, .. } => {
                // Sample rate or buffer size changed - need to unprepare and re-prepare
                log::debug!("Re-preparing plugin due to config change (was prepared)");

                let plugin = processor.unprepare();

                // Capture MIDI CC config before consuming the plugin
                let midi_cc_config = plugin.midi_cc_config();

                let config = P::Config::build_au(sample_rate, max_frames, layout.clone());
                let new_processor = plugin.prepare(config);

                // Pre-allocate conversion buffers if processor doesn't support f64
                let conversion_buffers = if !new_processor.supports_double_precision() {
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

                // Pre-allocate MIDI output buffer for process_midi()
                let midi_output_buffer = Box::new(beamer_core::MidiBuffer::new());

                *self = Self::Prepared {
                    processor: new_processor,
                    sample_rate,
                    max_frames,
                    conversion_buffers,
                    midi_cc_state,
                    midi_output_buffer,
                };
                Ok(())
            }
            Self::Transitioning => Err("Invalid state: transitioning".to_string()),
        }
    }
}
