//! Generic AU processor wrapper for type erasure.
//!
//! This module provides `AuProcessor<P>`, a generic wrapper that bridges any
//! `beamer_core::Descriptor` implementation to the AU API through the `AuPluginInstance`
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
//! - **Unprepared**: Descriptor created, parameters available, no audio resources
//! - **Prepared**: Resources allocated, ready for `process()` calls
//!
//! Transitions are triggered by `allocate_render_resources()` and
//! `deallocate_render_resources()` calls from the AU host.
//!
//! # DSP Processing
//!
//! The `process()` method constructs the proper `Buffer`, `AuxiliaryBuffers`,
//! and `ProcessContext` from input/output slices, then delegates to the plugin's
//! `Processor::process()` method. Transport information comes from the host
//! via the render callback (currently placeholder).

use std::marker::PhantomData;

use crate::error::{PluginError, PluginResult};
use crate::instance::AuPluginInstance;
use crate::lifecycle::AuState;
use beamer_core::{
    AuxiliaryBuffers, Buffer, CachedBusConfig, Descriptor, FactoryPresets, HasParameters,
    MidiEvent, NoPresets, ParameterGroups, ParameterStore, ProcessContext, Processor, Transport,
};

/// Generic AU processor wrapper.
///
/// Mirrors `Vst3Processor<P>` - wraps any `Descriptor` implementation
/// and implements `AuPluginInstance` for type erasure.
///
/// # Type Parameters
///
/// * `P` - The plugin type implementing `beamer_core::Descriptor`
/// * `Presets` - Optional factory presets collection (default: `NoPresets`)
pub struct AuProcessor<P, Presets = NoPresets<<P as HasParameters>::Parameters>>
where
    P: Descriptor,
    Presets: FactoryPresets<Parameters = <P as HasParameters>::Parameters>,
{
    state: AuState<P>,
    _presets: PhantomData<Presets>,
}

impl<P, Presets> AuProcessor<P, Presets>
where
    P: Descriptor,
    Presets: FactoryPresets<Parameters = <P as HasParameters>::Parameters>,
{
    /// Create a new AU processor.
    ///
    /// The processor starts in the Unprepared state with a default
    /// plugin instance. Call `allocate_render_resources` to prepare
    /// for audio processing.
    pub fn new() -> Self {
        Self {
            state: AuState::new(),
            _presets: PhantomData,
        }
    }
}

impl<P, Presets> Default for AuProcessor<P, Presets>
where
    P: Descriptor,
    Presets: FactoryPresets<Parameters = <P as HasParameters>::Parameters>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<P, Presets> AuPluginInstance for AuProcessor<P, Presets>
where
    P: Descriptor + 'static,
    P::Processor: HasParameters<Parameters = P::Parameters>,
    Presets: FactoryPresets<Parameters = P::Parameters>,
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
        use beamer_core::MidiEventKind;

        // Check if we have factory presets for automatic MIDI PC mapping
        let preset_count = Presets::count();

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

        // =========================================================================
        // MIDI Program Change → Factory Preset Mapping
        // =========================================================================
        //
        // When a plugin has factory presets, MIDI Program Change events are
        // automatically mapped to presets at the framework level:
        // - PC 0 → Preset 0, PC 1 → Preset 1, etc.
        // - PC events within preset range are applied and filtered out
        // - PC events outside preset range pass through to the plugin
        //
        // This mirrors VST3's kIsProgramChange behavior where the host handles
        // PC→preset mapping automatically.
        // =========================================================================

        if preset_count > 0 {
            // Check if any PC events map to valid factory presets
            let has_preset_pc = input.iter().any(|e| {
                matches!(&e.event, MidiEventKind::ProgramChange(pc) if (pc.program as usize) < preset_count)
            });

            if has_preset_pc {
                // Filter input: apply presets for matching PCs, pass through others
                let filtered: Vec<MidiEvent> = input
                    .iter()
                    .filter_map(|event| {
                        if let MidiEventKind::ProgramChange(pc) = &event.event {
                            if (pc.program as usize) < preset_count {
                                // Apply the factory preset
                                Presets::apply(pc.program as usize, processor.parameters());
                                // Filter out this event - it's been handled
                                return None;
                            }
                        }
                        // Pass through all other events (including out-of-range PCs)
                        Some(event.clone())
                    })
                    .collect();

                // Process remaining events through the plugin
                processor.process_midi(&filtered, &mut core_output);

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
                return;
            }
        }

        // No PC filtering needed - process all events directly
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

    fn preset_count(&self) -> u32 {
        Presets::count() as u32
    }

    fn preset_info(&self, index: u32) -> Option<(i32, &str)> {
        if (index as usize) < Presets::count() {
            Presets::info(index as usize).map(|info| (index as i32, info.name))
        } else {
            None
        }
    }

    fn apply_preset(&self, index: u32) -> bool {
        // Always apply unconditionally - never guard with "if changed".
        // Hosts may re-send the same preset, and skipping would break preset 0.
        let params = match &self.state {
            AuState::Unprepared { plugin, .. } => plugin.parameters(),
            AuState::Prepared { processor, .. } => processor.parameters(),
            AuState::Transitioning => return false,
        };
        Presets::apply(index as usize, params)
    }
}

/// Factory function type for creating AU processor instances.
///
/// Used by the export macro to register plugin factories.
pub type AuProcessorFactory = fn() -> Box<dyn AuPluginInstance>;

/// Create a factory function for a specific plugin type.
///
/// This is used by the export_au! macro.
pub fn create_processor_factory<P>() -> Box<dyn AuPluginInstance>
where
    P: Descriptor + 'static,
    P::Processor: HasParameters<Parameters = P::Parameters>,
{
    Box::new(AuProcessor::<P>::new())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use beamer_core::CachedBusInfo;
    use crate::render::MidiBuffer;
    use beamer_core::parameter_groups::{GroupInfo, ParameterGroups};
    use beamer_core::parameter_info::{ParameterFlags, ParameterInfo, ParameterUnit};
    use beamer_core::parameter_store::ParameterStore;
    use beamer_core::parameter_types::{ParameterRef, Parameters};
    use beamer_core::preset::{fnv1a_hash, FactoryPresets, PresetInfo, PresetValue};
    use beamer_core::{AuxiliaryBuffers, BusInfo, BusType, MidiEventKind, PluginResult};
    use std::sync::atomic::{AtomicU64, Ordering};

    // =========================================================================
    // Mock Parameter
    // =========================================================================

    /// A minimal mock parameter for testing.
    struct MockParameter {
        id: u32,
        name: &'static str,
        value: AtomicU64,
        info: ParameterInfo,
    }

    impl MockParameter {
        fn new(id: u32, name: &'static str, default: f64) -> Self {
            Self {
                id,
                name,
                value: AtomicU64::new(default.to_bits()),
                info: ParameterInfo {
                    id,
                    name,
                    short_name: name,
                    units: "dB",
                    unit: ParameterUnit::Decibels,
                    step_count: 0,
                    default_normalized: default,
                    flags: ParameterFlags::default(),
                    group_id: 0,
                },
            }
        }
    }

    impl ParameterRef for MockParameter {
        fn id(&self) -> u32 {
            self.id
        }
        fn name(&self) -> &'static str {
            self.name
        }
        fn short_name(&self) -> &'static str {
            self.name
        }
        fn units(&self) -> &'static str {
            "dB"
        }
        fn flags(&self) -> &ParameterFlags {
            &self.info.flags
        }
        fn default_normalized(&self) -> f64 {
            self.info.default_normalized
        }
        fn step_count(&self) -> i32 {
            0
        }
        fn get_normalized(&self) -> f64 {
            f64::from_bits(self.value.load(Ordering::Relaxed))
        }
        fn set_normalized(&self, value: f64) {
            self.value.store(value.to_bits(), Ordering::Relaxed);
        }
        fn get_plain(&self) -> f64 {
            self.get_normalized()
        }
        fn set_plain(&self, value: f64) {
            self.set_normalized(value);
        }
        fn display_normalized(&self, normalized: f64) -> String {
            format!("{:.2}", normalized)
        }
        fn parse(&self, s: &str) -> Option<f64> {
            s.parse().ok()
        }
        fn normalized_to_plain(&self, normalized: f64) -> f64 {
            normalized
        }
        fn plain_to_normalized(&self, plain: f64) -> f64 {
            plain
        }
        fn info(&self) -> &ParameterInfo {
            &self.info
        }
    }

    // =========================================================================
    // Mock Parameters Collection
    // =========================================================================

    struct TestParameters {
        gain: MockParameter,
    }

    impl TestParameters {
        fn new() -> Self {
            Self {
                gain: MockParameter::new(fnv1a_hash("gain"), "Gain", 0.5),
            }
        }
    }

    impl ParameterGroups for TestParameters {
        fn group_count(&self) -> usize {
            1
        }
        fn group_info(&self, index: usize) -> Option<GroupInfo> {
            if index == 0 {
                Some(GroupInfo::root())
            } else {
                None
            }
        }
    }

    impl Parameters for TestParameters {
        fn count(&self) -> usize {
            1
        }
        fn iter(&self) -> Box<dyn Iterator<Item = &dyn ParameterRef> + '_> {
            Box::new(std::iter::once(&self.gain as &dyn ParameterRef))
        }
        fn by_id(&self, id: u32) -> Option<&dyn ParameterRef> {
            if id == self.gain.id {
                Some(&self.gain)
            } else {
                None
            }
        }
    }

    impl ParameterStore for TestParameters {
        fn count(&self) -> usize {
            1
        }
        fn info(&self, index: usize) -> Option<&ParameterInfo> {
            if index == 0 {
                Some(&self.gain.info)
            } else {
                None
            }
        }
        fn get_normalized(&self, id: u32) -> f64 {
            if id == self.gain.id {
                self.gain.get_normalized()
            } else {
                0.0
            }
        }
        fn set_normalized(&self, id: u32, value: f64) {
            if id == self.gain.id {
                self.gain.set_normalized(value);
            }
        }
        fn normalized_to_string(&self, id: u32, normalized: f64) -> String {
            if id == self.gain.id {
                self.gain.display_normalized(normalized)
            } else {
                String::new()
            }
        }
        fn string_to_normalized(&self, id: u32, string: &str) -> Option<f64> {
            if id == self.gain.id {
                self.gain.parse(string)
            } else {
                None
            }
        }
        fn normalized_to_plain(&self, id: u32, normalized: f64) -> f64 {
            if id == self.gain.id {
                self.gain.normalized_to_plain(normalized)
            } else {
                0.0
            }
        }
        fn plain_to_normalized(&self, id: u32, plain: f64) -> f64 {
            if id == self.gain.id {
                self.gain.plain_to_normalized(plain)
            } else {
                0.0
            }
        }
    }

    // =========================================================================
    // Mock Plugin
    // =========================================================================

    #[derive(Default)]
    struct TestPlugin {
        parameters: TestParameters,
    }

    impl Default for TestParameters {
        fn default() -> Self {
            Self::new()
        }
    }

    impl HasParameters for TestPlugin {
        type Parameters = TestParameters;
        fn parameters(&self) -> &Self::Parameters {
            &self.parameters
        }
        fn parameters_mut(&mut self) -> &mut Self::Parameters {
            &mut self.parameters
        }
        fn set_parameters(&mut self, params: Self::Parameters) {
            self.parameters = params;
        }
    }

    impl Descriptor for TestPlugin {
        type Setup = ();
        type Processor = TestProcessor;

        fn prepare(self, _setup: ()) -> Self::Processor {
            TestProcessor {
                parameters: self.parameters,
            }
        }

        fn input_bus_count(&self) -> usize {
            1
        }
        fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
            if index == 0 {
                Some(BusInfo::stereo("Input"))
            } else {
                None
            }
        }
    }

    // =========================================================================
    // Mock Processor
    // =========================================================================

    struct TestProcessor {
        parameters: TestParameters,
    }

    impl HasParameters for TestProcessor {
        type Parameters = TestParameters;
        fn parameters(&self) -> &Self::Parameters {
            &self.parameters
        }
        fn parameters_mut(&mut self) -> &mut Self::Parameters {
            &mut self.parameters
        }
        fn set_parameters(&mut self, params: Self::Parameters) {
            self.parameters = params;
        }
    }

    impl Processor for TestProcessor {
        type Descriptor = TestPlugin;

        fn process(
            &mut self,
            _buffer: &mut Buffer,
            _aux: &mut AuxiliaryBuffers,
            _context: &ProcessContext,
        ) {
            // No-op for testing
        }

        fn save_state(&self) -> PluginResult<Vec<u8>> {
            Ok(vec![])
        }

        fn load_state(&mut self, _data: &[u8]) -> PluginResult<()> {
            Ok(())
        }
    }

    // =========================================================================
    // Test Presets
    // =========================================================================
    //
    // Note: MockParameter uses identity mapping (normalized == plain), so preset
    // plain_values are used directly as normalized values in assertions. This
    // simplifies the tests since we're testing MIDI PC → preset application flow,
    // not the actual dB-to-normalized conversion (which is tested in beamer-core).

    struct TestPresets;

    const PRESET_UNITY: &[PresetValue] = &[PresetValue {
        id: fnv1a_hash("gain"),
        plain_value: 0.5, // Unity (middle)
    }];

    const PRESET_QUIET: &[PresetValue] = &[PresetValue {
        id: fnv1a_hash("gain"),
        plain_value: 0.25, // Quiet
    }];

    const PRESET_BOOST: &[PresetValue] = &[PresetValue {
        id: fnv1a_hash("gain"),
        plain_value: 0.75, // Boost
    }];

    impl FactoryPresets for TestPresets {
        type Parameters = TestParameters;

        fn count() -> usize {
            3
        }

        fn info(index: usize) -> Option<PresetInfo> {
            match index {
                0 => Some(PresetInfo { name: "Unity" }),
                1 => Some(PresetInfo { name: "Quiet" }),
                2 => Some(PresetInfo { name: "Boost" }),
                _ => None,
            }
        }

        fn values(index: usize) -> &'static [PresetValue] {
            match index {
                0 => PRESET_UNITY,
                1 => PRESET_QUIET,
                2 => PRESET_BOOST,
                _ => &[],
            }
        }
    }

    // =========================================================================
    // Test Helpers
    // =========================================================================

    /// Create a prepared AuProcessor for testing MIDI functionality.
    fn create_prepared_processor() -> AuProcessor<TestPlugin, TestPresets> {
        let mut processor = AuProcessor::<TestPlugin, TestPresets>::new();

        let bus_config = CachedBusConfig::new(
            vec![CachedBusInfo::new(2, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        );

        processor
            .allocate_render_resources(44100.0, 512, &bus_config)
            .expect("Failed to prepare processor");

        processor
    }

    /// Get the gain parameter's normalized value.
    fn get_gain_normalized(processor: &AuProcessor<TestPlugin, TestPresets>) -> f64 {
        let params = processor.parameter_store().unwrap();
        let gain_id = params.info(0).unwrap().id;
        params.get_normalized(gain_id)
    }

    /// Set the gain parameter's normalized value.
    fn set_gain_normalized(processor: &mut AuProcessor<TestPlugin, TestPresets>, value: f64) {
        let params = processor.parameter_store().unwrap();
        let gain_id = params.info(0).unwrap().id;
        processor
            .parameter_store_mut()
            .unwrap()
            .set_normalized(gain_id, value);
    }

    // =========================================================================
    // MIDI Program Change → Factory Preset Tests
    // =========================================================================

    #[test]
    fn midi_pc_applies_preset_and_filters() {
        let mut processor = create_prepared_processor();

        // Set gain to 1.0 (max)
        set_gain_normalized(&mut processor, 1.0);

        // Send PC 1 (Quiet preset: gain = 0.25)
        let input = vec![MidiEvent::program_change(0, 0, 1)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // PC event should be filtered out (preset was applied)
        assert_eq!(output.len(), 0, "PC event should be filtered out");

        // Verify the preset was applied (Quiet = 0.25)
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - 0.25).abs() < 0.01,
            "Quiet preset should set gain to 0.25, got {}",
            norm_value
        );
    }

    #[test]
    fn midi_pc_out_of_range_passes_through() {
        let mut processor = create_prepared_processor();

        // Set initial gain
        set_gain_normalized(&mut processor, 0.5);
        let initial_norm = get_gain_normalized(&processor);

        // Send PC 10 (out of range - only 3 presets)
        let input = vec![MidiEvent::program_change(0, 0, 10)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // Out-of-range PC should pass through
        assert_eq!(output.len(), 1, "Out-of-range PC should pass through");

        // Verify it's a PC event with correct program number
        if let MidiEventKind::ProgramChange(pc) = &output.iter().next().unwrap().event {
            assert_eq!(pc.program, 10);
        } else {
            panic!("Expected ProgramChange event");
        }

        // Parameters should be unchanged
        let final_norm = get_gain_normalized(&processor);
        assert!(
            (final_norm - initial_norm).abs() < 0.001,
            "Out-of-range PC should not change parameters"
        );
    }

    #[test]
    fn midi_other_events_pass_through() {
        let mut processor = create_prepared_processor();

        // Send control change events
        let input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),
            MidiEvent::control_change(10, 0, 7, 0.8),
            MidiEvent::control_change(20, 0, 10, 0.5),
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // All events should pass through
        assert_eq!(output.len(), 3, "Non-PC events should pass through");
    }

    #[test]
    fn midi_mixed_events_filters_only_valid_pc() {
        let mut processor = create_prepared_processor();

        // Mix of events: CC, valid PC, CC, invalid PC, CC
        let input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),
            MidiEvent::program_change(0, 0, 2), // Boost preset (valid)
            MidiEvent::control_change(10, 0, 7, 0.8),
            MidiEvent::program_change(20, 0, 50), // Invalid (out of range)
            MidiEvent::control_change(30, 0, 10, 0.5),
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // 5 input events, but valid PC (program 2) should be filtered
        // Remaining: CC, CC, PC(50), CC = 4 events
        assert_eq!(
            output.len(),
            4,
            "Valid PC should be filtered, others pass through"
        );

        // Verify Boost preset was applied (0.75)
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - 0.75).abs() < 0.01,
            "Boost preset should set gain to 0.75, got {}",
            norm_value
        );
    }

    #[test]
    fn midi_pc_zero_applies_first_preset() {
        let mut processor = create_prepared_processor();

        // Set gain to something other than Unity
        set_gain_normalized(&mut processor, 0.3);

        // Send PC 0 (Unity preset: gain = 0.5)
        let input = vec![MidiEvent::program_change(0, 0, 0)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // PC should be filtered
        assert_eq!(output.len(), 0);

        // Verify Unity preset was applied (0.5)
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - 0.5).abs() < 0.01,
            "Unity preset should set gain to 0.5, got {}",
            norm_value
        );
    }

    #[test]
    fn midi_multiple_pc_events_last_wins() {
        let mut processor = create_prepared_processor();

        // Send multiple PC events - last valid one should win
        let input = vec![
            MidiEvent::program_change(0, 0, 0),  // Unity (0.5)
            MidiEvent::program_change(10, 0, 1), // Quiet (0.25)
            MidiEvent::program_change(20, 0, 2), // Boost (0.75) - last, should win
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&input, &mut output);

        // All PC events should be filtered
        assert_eq!(output.len(), 0);

        // Last preset (Boost) should be applied
        let norm_value = get_gain_normalized(&processor);
        assert!(
            (norm_value - 0.75).abs() < 0.01,
            "Last PC (Boost) should set gain to 0.75, got {}",
            norm_value
        );
    }
}
