//! Generic AU processor wrapper for type erasure.
//!
//! This module provides `AuProcessor<P>`, a generic wrapper that bridges any
//! `beamer_core::Plugin` implementation to the AU API through the `AuPluginInstance`
//! trait. This enables a single Objective-C class to work with any plugin type
//! via dynamic dispatch.
//!
//! # Design Pattern
//!
//! `AuProcessor<P>` mirrors `Vst3Processor<P>` from the VST3 wrapper, implementing
//! the same sealed trait pattern for consistent behavior across plugin formats.
//! The plugin's generic type `P` is preserved, but wrapped in a trait object
//! at the AU level for Objective-C interoperability.
//!
//! # Lifecycle Management
//!
//! The processor manages two lifecycle states via `AuState<P>`:
//! - **Unprepared**: Plugin created, parameters available, no audio resources
//! - **Prepared**: Resources allocated, ready for `process()` calls
//!
//! Transitions are triggered by `allocate_render_resources()` and
//! `deallocate_render_resources()` calls from the AU host.
//!
//! # DSP Processing
//!
//! The `process()` method constructs the proper `Buffer`, `AuxiliaryBuffers`,
//! and `ProcessContext` from input/output slices, then delegates to the plugin's
//! `AudioProcessor::process()` method. Transport information comes from the host
//! via the render callback (currently placeholder).

use crate::error::{PluginError, PluginResult};
use crate::instance::AuPluginInstance;
use crate::lifecycle::{AuState, BuildAuConfig};
use beamer_core::{
    AudioProcessor, AuxiliaryBuffers, Buffer, CachedBusConfig, HasParameters, MidiEvent,
    ParameterGroups, ParameterStore, Plugin, ProcessContext, Transport,
};

/// Generic AU processor wrapper.
///
/// Mirrors `Vst3Processor<P>` - wraps any `Plugin` implementation
/// and implements `AuPluginInstance` for type erasure.
///
/// # Type Parameters
///
/// * `P` - The plugin type implementing `beamer_core::Plugin`
pub struct AuProcessor<P: Plugin> {
    state: AuState<P>,
}

impl<P: Plugin> AuProcessor<P> {
    /// Create a new AU processor.
    ///
    /// The processor starts in the Unprepared state with a default
    /// plugin instance. Call `allocate_render_resources` to prepare
    /// for audio processing.
    pub fn new() -> Self {
        Self {
            state: AuState::new(),
        }
    }
}

impl<P: Plugin> Default for AuProcessor<P> {
    fn default() -> Self {
        Self::new()
    }
}

// Single implementation using the sealed BuildAuConfig trait
//
// Clippy Allow: private_bounds
//
// `BuildAuConfig` is a sealed (pub(crate)) trait that restricts which config types work with AU.
// This is intentional API design - only NoConfig, AudioSetup, and FullAudioSetup are supported.
// The trait bound must be private because BuildAuConfig itself is crate-private.
#[allow(private_bounds)]
impl<P> AuPluginInstance for AuProcessor<P>
where
    P: Plugin + 'static,
    P::Config: BuildAuConfig,
    P::Processor: HasParameters<Parameters = P::Parameters>,
{
    fn allocate_render_resources(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        bus_config: &CachedBusConfig,
    ) -> PluginResult<()> {
        self.state
            .prepare(sample_rate, max_frames, bus_config)
            .map_err(PluginError::InitializationFailed)
    }

    fn deallocate_render_resources(&mut self) {
        let _ = self.state.unprepare();
    }

    fn is_prepared(&self) -> bool {
        self.state.is_prepared()
    }

    fn sample_rate(&self) -> Option<f64> {
        self.state.sample_rate()
    }

    fn max_frames(&self) -> Option<u32> {
        self.state.max_frames()
    }

    fn parameter_store(&self) -> Result<&dyn ParameterStore, PluginError> {
        match &self.state {
            AuState::Unprepared { plugin, .. } => Ok(plugin.parameters()),
            AuState::Prepared { processor, .. } => Ok(processor.parameters()),
            AuState::Transitioning => {
                Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        }
    }

    fn parameter_store_mut(&mut self) -> Result<&mut dyn ParameterStore, PluginError> {
        match &mut self.state {
            AuState::Unprepared { plugin, .. } => Ok(plugin.parameters_mut()),
            AuState::Prepared { processor, .. } => Ok(processor.parameters_mut()),
            AuState::Transitioning => {
                Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        }
    }

    fn parameter_groups(&self) -> Result<&dyn ParameterGroups, PluginError> {
        match &self.state {
            AuState::Unprepared { plugin, .. } => Ok(plugin.parameters()),
            AuState::Prepared { processor, .. } => Ok(processor.parameters()),
            AuState::Transitioning => {
                Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        }
    }

    fn save_state(&self) -> Vec<u8> {
        match &self.state {
            AuState::Unprepared { .. } => {
                // Can't save processor state when not prepared
                Vec::new()
            }
            AuState::Prepared { processor, .. } => {
                // Use processor's save_state which includes custom state
                processor.save_state().unwrap_or_default()
            }
            AuState::Transitioning => Vec::new(),
        }
    }

    fn load_state(&mut self, data: &[u8]) -> PluginResult<()> {
        match &mut self.state {
            AuState::Unprepared { pending_state, .. } => {
                // Defer loading until prepare() is called
                *pending_state = Some(data.to_vec());
                Ok(())
            }
            AuState::Prepared { processor, .. } => {
                // Load state immediately and reset smoothing
                processor
                    .load_state(data)
                    .map_err(|e| PluginError::StateError(e.to_string()))?;
                use beamer_core::parameter_types::Parameters;
                processor.parameters_mut().reset_smoothing();
                Ok(())
            }
            AuState::Transitioning => {
                Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        }
    }

    fn reset(&mut self) {
        if let Some(processor) = self.state.processor_mut() {
            // Full reset sequence: deactivate then reactivate
            // This matches VST3 behavior and beamer_core documentation
            processor.set_active(false);
            processor.set_active(true);
        }
    }

    fn tail_samples(&self) -> u32 {
        self.state
            .processor()
            .map(|p| p.tail_samples())
            .unwrap_or(0)
    }

    fn latency_samples(&self) -> u32 {
        self.state
            .processor()
            .map(|p| p.latency_samples())
            .unwrap_or(0)
    }

    fn supports_native_double_precision(&self) -> bool {
        self.state
            .processor()
            .map(|p| p.supports_double_precision())
            .unwrap_or(false)
    }

    fn declared_input_bus_count(&self) -> usize {
        match &self.state {
            AuState::Unprepared { plugin, .. } => plugin.input_bus_count(),
            // Deterministic fallback that doesn't disrupt the prepared processor.
            _ => P::default().input_bus_count(),
        }
    }

    fn declared_output_bus_count(&self) -> usize {
        match &self.state {
            AuState::Unprepared { plugin, .. } => plugin.output_bus_count(),
            _ => P::default().output_bus_count(),
        }
    }

    fn declared_input_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo> {
        match &self.state {
            AuState::Unprepared { plugin, .. } => plugin.input_bus_info(index),
            _ => P::default().input_bus_info(index),
        }
    }

    fn declared_output_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo> {
        match &self.state {
            AuState::Unprepared { plugin, .. } => plugin.output_bus_info(index),
            _ => P::default().output_bus_info(index),
        }
    }

    fn process(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        num_samples: usize,
    ) -> PluginResult<()> {
        // Get processor and sample_rate from prepared state
        let (processor, sample_rate) = match &mut self.state {
            AuState::Prepared {
                processor,
                sample_rate,
                ..
            } => (processor, *sample_rate),
            AuState::Unprepared { .. } => {
                return Err(PluginError::ProcessingError("not prepared".to_string()))
            }
            AuState::Transitioning => {
                return Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        };

        // Build Buffer from input/output slices
        // The Buffer::new takes iterators, so we convert slices to iterators
        let input_iter = inputs.iter().copied();
        let output_iter = outputs.iter_mut().map(|s| &mut **s);
        let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

        // Build AuxiliaryBuffers (empty for now - sidechain support is future work)
        let mut aux = AuxiliaryBuffers::empty();

        // Build ProcessContext with transport info
        // For now, use empty transport. Transport extraction from AU is handled separately.
        let transport = Transport::default();
        let context = ProcessContext::new(sample_rate, num_samples, transport);

        // Call the actual processor
        processor.process(&mut buffer, &mut aux, &context);

        Ok(())
    }

    fn process_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        num_samples: usize,
    ) -> PluginResult<()> {
        // Get processor, sample_rate, and conversion_buffers from prepared state
        let (processor, sample_rate, conversion_buffers) = match &mut self.state {
            AuState::Prepared {
                processor,
                sample_rate,
                conversion_buffers,
                ..
            } => (processor, *sample_rate, conversion_buffers),
            AuState::Unprepared { .. } => {
                return Err(PluginError::ProcessingError("not prepared".to_string()))
            }
            AuState::Transitioning => {
                return Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        };

        // Check if processor supports native f64 processing
        if processor.supports_double_precision() {
            // Native f64 processing
            let input_iter = inputs.iter().copied();
            let output_iter = outputs.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let mut aux = AuxiliaryBuffers::empty();
            let transport = Transport::default();
            let context = ProcessContext::new(sample_rate, num_samples, transport);

            processor.process_f64(&mut buffer, &mut aux, &context);
        } else {
            // Convert f64 → f32 using pre-allocated buffers, process, convert back
            let conversion = conversion_buffers.as_mut().expect(
                "conversion_buffers should be allocated when processor doesn't support f64",
            );

            // Convert f64 → f32 using pre-allocated input buffers
            for (ch_idx, input_ch) in inputs.iter().enumerate() {
                if let Some(buf) = conversion.main_input_mut(ch_idx) {
                    for (i, &sample) in input_ch.iter().take(num_samples).enumerate() {
                        buf[i] = sample as f32;
                    }
                }
            }

            // Build f32 buffer views for processing
            let input_f32_slices: Vec<&[f32]> = conversion
                .main_input_f32
                .iter()
                .map(|v| &v[..num_samples])
                .collect();
            let mut output_f32_slices: Vec<&mut [f32]> = conversion
                .main_output_f32
                .iter_mut()
                .map(|v| &mut v[..num_samples])
                .collect();

            let input_iter = input_f32_slices.iter().copied();
            let output_iter = output_f32_slices.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let mut aux = AuxiliaryBuffers::empty();
            let transport = Transport::default();
            let context = ProcessContext::new(sample_rate, num_samples, transport);

            processor.process(&mut buffer, &mut aux, &context);

            // Convert f32 → f64 back to output
            for (ch_idx, output_ch) in outputs.iter_mut().enumerate() {
                if let Some(buf) = conversion.main_output_f32.get(ch_idx) {
                    for (i, sample) in output_ch.iter_mut().take(num_samples).enumerate() {
                        *sample = buf[i] as f64;
                    }
                }
            }
        }

        Ok(())
    }

    fn process_with_context_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Get processor and conversion_buffers from prepared state
        let (processor, conversion_buffers) = match &mut self.state {
            AuState::Prepared {
                processor,
                conversion_buffers,
                ..
            } => (processor, conversion_buffers),
            AuState::Unprepared { .. } => {
                return Err(PluginError::ProcessingError("not prepared".to_string()))
            }
            AuState::Transitioning => {
                return Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        };

        let num_samples = context.num_samples;

        // Check if processor supports native f64 processing
        if processor.supports_double_precision() {
            // Native f64 processing
            let input_iter = inputs.iter().copied();
            let output_iter = outputs.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let mut aux = AuxiliaryBuffers::empty();
            processor.process_f64(&mut buffer, &mut aux, context);
        } else {
            // Convert f64 → f32 using pre-allocated buffers, process, convert back
            let conversion = conversion_buffers.as_mut().expect(
                "conversion_buffers should be allocated when processor doesn't support f64",
            );

            // Convert f64 → f32 using pre-allocated input buffers
            for (ch_idx, input_ch) in inputs.iter().enumerate() {
                if let Some(buf) = conversion.main_input_mut(ch_idx) {
                    for (i, &sample) in input_ch.iter().take(num_samples).enumerate() {
                        buf[i] = sample as f32;
                    }
                }
            }

            // Build f32 buffer views for processing
            let input_f32_slices: Vec<&[f32]> = conversion
                .main_input_f32
                .iter()
                .map(|v| &v[..num_samples])
                .collect();
            let mut output_f32_slices: Vec<&mut [f32]> = conversion
                .main_output_f32
                .iter_mut()
                .map(|v| &mut v[..num_samples])
                .collect();

            let input_iter = input_f32_slices.iter().copied();
            let output_iter = output_f32_slices.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let mut aux = AuxiliaryBuffers::empty();
            processor.process(&mut buffer, &mut aux, context);

            // Convert f32 → f64 back to output
            for (ch_idx, output_ch) in outputs.iter_mut().enumerate() {
                if let Some(buf) = conversion.main_output_f32.get(ch_idx) {
                    for (i, sample) in output_ch.iter_mut().take(num_samples).enumerate() {
                        *sample = buf[i] as f64;
                    }
                }
            }
        }

        Ok(())
    }

    fn process_with_aux(
        &mut self,
        inputs: &[&[f32]],
        outputs: &mut [&mut [f32]],
        aux_inputs: &[Vec<&[f32]>],
        aux_outputs: &mut [Vec<&mut [f32]>],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Get processor from prepared state
        let processor = match &mut self.state {
            AuState::Prepared { processor, .. } => processor,
            AuState::Unprepared { .. } => {
                return Err(PluginError::ProcessingError("not prepared".to_string()))
            }
            AuState::Transitioning => {
                return Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        };

        let num_samples = context.num_samples;

        // Build Buffer from input/output slices
        let input_iter = inputs.iter().copied();
        let output_iter = outputs.iter_mut().map(|s| &mut **s);
        let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

        // Build AuxiliaryBuffers from aux input/output slices
        let aux_input_iter = aux_inputs.iter().map(|bus| bus.iter().copied());
        let aux_output_iter = aux_outputs
            .iter_mut()
            .map(|bus| bus.iter_mut().map(|s| &mut **s));
        let mut aux = AuxiliaryBuffers::new(aux_input_iter, aux_output_iter, num_samples);

        // Call the actual processor
        processor.process(&mut buffer, &mut aux, context);

        Ok(())
    }

    fn process_with_aux_f64(
        &mut self,
        inputs: &[&[f64]],
        outputs: &mut [&mut [f64]],
        aux_inputs: &[Vec<&[f64]>],
        aux_outputs: &mut [Vec<&mut [f64]>],
        context: &ProcessContext,
    ) -> PluginResult<()> {
        // Get processor and conversion_buffers from prepared state
        let (processor, conversion_buffers) = match &mut self.state {
            AuState::Prepared {
                processor,
                conversion_buffers,
                ..
            } => (processor, conversion_buffers),
            AuState::Unprepared { .. } => {
                return Err(PluginError::ProcessingError("not prepared".to_string()))
            }
            AuState::Transitioning => {
                return Err(PluginError::ProcessingError("transitioning".to_string()))
            }
        };

        let num_samples = context.num_samples;

        // Check if processor supports native f64 processing
        if processor.supports_double_precision() {
            // Native f64 processing
            let input_iter = inputs.iter().copied();
            let output_iter = outputs.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let aux_input_iter = aux_inputs.iter().map(|bus| bus.iter().copied());
            let aux_output_iter = aux_outputs
                .iter_mut()
                .map(|bus| bus.iter_mut().map(|s| &mut **s));
            let mut aux = AuxiliaryBuffers::new(aux_input_iter, aux_output_iter, num_samples);

            processor.process_f64(&mut buffer, &mut aux, context);
        } else {
            // Convert f64 → f32 using pre-allocated buffers, process, convert back
            let conversion = conversion_buffers.as_mut().expect(
                "conversion_buffers should be allocated when processor doesn't support f64",
            );

            // Convert main inputs f64 → f32
            for (ch_idx, input_ch) in inputs.iter().enumerate() {
                if let Some(buf) = conversion.main_input_mut(ch_idx) {
                    for (i, &sample) in input_ch.iter().take(num_samples).enumerate() {
                        buf[i] = sample as f32;
                    }
                }
            }

            // Convert aux inputs f64 → f32
            for (bus_idx, bus) in aux_inputs.iter().enumerate() {
                for (ch_idx, ch) in bus.iter().enumerate() {
                    if let Some(buf) = conversion.aux_input_mut(bus_idx, ch_idx, num_samples)
                    {
                        for (i, &sample) in ch.iter().take(num_samples).enumerate() {
                            buf[i] = sample as f32;
                        }
                    }
                }
            }

            // Build f32 slices for processing
            let input_f32_slices: Vec<&[f32]> = conversion
                .main_input_f32
                .iter()
                .map(|v| &v[..num_samples])
                .collect();
            let mut output_f32_slices: Vec<&mut [f32]> = conversion
                .main_output_f32
                .iter_mut()
                .map(|v| &mut v[..num_samples])
                .collect();

            // Build aux f32 slices
            let aux_input_f32_slices: Vec<Vec<&[f32]>> = conversion
                .aux_input_f32
                .iter()
                .map(|bus| bus.iter().map(|ch| &ch[..num_samples]).collect())
                .collect();
            let mut aux_output_f32_slices: Vec<Vec<&mut [f32]>> = conversion
                .aux_output_f32
                .iter_mut()
                .map(|bus| bus.iter_mut().map(|ch| &mut ch[..num_samples]).collect())
                .collect();

            // Build Buffer and AuxiliaryBuffers
            let input_iter = input_f32_slices.iter().copied();
            let output_iter = output_f32_slices.iter_mut().map(|s| &mut **s);
            let mut buffer = Buffer::new(input_iter, output_iter, num_samples);

            let aux_input_iter = aux_input_f32_slices.iter().map(|bus| bus.iter().copied());
            let aux_output_iter = aux_output_f32_slices
                .iter_mut()
                .map(|bus| bus.iter_mut().map(|s| &mut **s));
            let mut aux = AuxiliaryBuffers::new(aux_input_iter, aux_output_iter, num_samples);

            processor.process(&mut buffer, &mut aux, context);

            // Convert main outputs f32 → f64
            for (ch_idx, output_ch) in outputs.iter_mut().enumerate() {
                if let Some(buf) = conversion.main_output_f32.get(ch_idx) {
                    for (i, sample) in output_ch.iter_mut().take(num_samples).enumerate() {
                        *sample = buf[i] as f64;
                    }
                }
            }

            // Convert aux outputs f32 → f64
            for (bus_idx, bus) in aux_outputs.iter_mut().enumerate() {
                for (ch_idx, ch) in bus.iter_mut().enumerate() {
                    if let Some(buf) = conversion.aux_output(bus_idx, ch_idx, num_samples) {
                        for (i, sample) in ch.iter_mut().take(num_samples).enumerate() {
                            *sample = buf[i] as f64;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    fn apply_parameter_events(
        &mut self,
        immediate: &[crate::render::AuParameterEvent],
        ramps: &[crate::render::AuParameterRampEvent],
    ) -> PluginResult<()> {
        // Only apply if in prepared state
        let processor = match &mut self.state {
            AuState::Prepared { processor, .. } => processor,
            _ => return Ok(()), // Ignore if not prepared
        };

        use beamer_core::parameter_types::Parameters;

        // =========================================================================
        // Parameter Automation Design Notes
        // =========================================================================
        //
        // Current implementation: Set target value, let existing smoothers interpolate.
        //
        // Why this approach is intentional:
        // 1. **Parity with VST3**: The beamer VST3 wrapper uses the same "last value"
        //    approach (see beamer-vst3/src/processor.rs lines 1916-1920).
        //
        // 2. **Smoother API limitations**: beamer_core's Smoother uses a fixed time
        //    constant configured at parameter construction (via SmoothingStyle).
        //    There is no API for dynamic per-event ramp configuration like
        //    `set_normalized_with_ramp(value, samples)`.
        //
        // 3. **Practical behavior**: For most musical parameters, the configured
        //    smoother time (e.g., 5ms exponential) provides smooth transitions
        //    that sound good regardless of the DAW's ramp duration.
        //
        // Sample offset (`sample_offset`) is currently not used because:
        // - True sample-accurate automation would require sub-block processing
        //   (splitting audio at event boundaries), which adds complexity and overhead
        // - The smoother interpolates across the entire buffer anyway
        // - Most plugins don't need sub-sample precision for parameter changes
        //
        // Ramp duration (`duration_samples`) is currently not used because:
        // - beamer_core's Smoother doesn't support dynamic ramp reconfiguration
        // - The fixed smoother time constant provides consistent behavior
        //
        // Future improvement options (if needed):
        // - Add dynamic ramp support to beamer_core::Smoother
        // - Implement sub-block processing for sample-accurate automation
        // =========================================================================

        // Apply immediate parameter changes
        // These set the target value; smoothers handle interpolation to avoid zipper noise.
        for event in immediate {
            // Convert AU parameter address to beamer parameter ID
            // AU parameter addresses map directly to beamer parameter IDs
            let param_id = event.parameter_address as u32;

            if let Some(param) = processor.parameters_mut().by_id(param_id) {
                param.set_normalized(event.value as f64);
            }
        }

        // Apply parameter ramps
        // Set the end value as the target; the parameter's smoother interpolates.
        // The ramp's `duration_samples` is not used because beamer_core's Smoother
        // uses a fixed time constant configured at parameter construction.
        for event in ramps {
            let param_id = event.parameter_address as u32;

            if let Some(param) = processor.parameters_mut().by_id(param_id) {
                param.set_normalized(event.end_value as f64);
            }
        }

        Ok(())
    }

    fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        self.state.midi_cc_state()
    }

    fn process_midi(&mut self, input: &[MidiEvent], output: &mut crate::render::MidiBuffer) {
        // Take the pre-allocated buffer temporarily to avoid borrow issues
        let mut core_output = match &mut self.state {
            AuState::Prepared {
                midi_output_buffer, ..
            } => std::mem::take(midi_output_buffer),
            _ => {
                // Not prepared - pass through events unchanged
                for event in input {
                    let _ = output.push(event.clone());
                }
                return;
            }
        };

        // Clear for reuse
        core_output.clear();

        // Get processor reference
        let processor = match &mut self.state {
            AuState::Prepared { processor, .. } => processor,
            _ => unreachable!(), // We already matched Prepared above
        };

        // Call the processor's MIDI processing method
        processor.process_midi(input, &mut core_output);

        // Copy events back to AU's MidiBuffer
        for event in core_output.iter() {
            let _ = output.push(event.clone());
        }

        // Put buffer back
        if let AuState::Prepared {
            midi_output_buffer, ..
        } = &mut self.state
        {
            *midi_output_buffer = core_output;
        }
    }
}

/// Factory function type for creating AU processor instances.
///
/// Used by the export macro to register plugin factories.
pub type AuProcessorFactory = fn() -> Box<dyn AuPluginInstance>;

/// Create a factory function for a specific plugin type.
///
/// This is used by the export_au! macro.
///
/// Clippy Allow: private_bounds - See comment on AuProcessor impl for explanation.
#[allow(private_bounds)]
pub fn create_processor_factory<P>() -> Box<dyn AuPluginInstance>
where
    P: Plugin + 'static,
    P::Config: BuildAuConfig,
    P::Processor: HasParameters<Parameters = P::Parameters>,
{
    Box::new(AuProcessor::<P>::new())
}
