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
use std::sync::Arc;

use crate::error::{PluginError, PluginResult};
use crate::instance::{AuPluginInstance, FactoryPresetAccess, SharedParameters};
use crate::lifecycle::AuState;
use beamer_core::{
    AuxiliaryBuffers, Buffer, CachedBusConfig, Descriptor, FactoryPresets, HasParameters,
    MidiEvent, NoPresets, ParameterStore, ProcessContext, ProcessMode, Processor,
    Transport, WebViewHandler,
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
    /// Cached WebView handler from the Descriptor. Captured at construction
    /// so it remains accessible after prepare() consumes the Descriptor.
    webview_handler: Option<Arc<dyn WebViewHandler>>,
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
        // Create a single descriptor instance and capture the WebView
        // handler from it before passing ownership to AuState. This
        // ensures the handler references the same instance that becomes
        // the live plugin, not a discarded throw-away copy.
        let descriptor = P::default();
        let handler = descriptor.webview_handler();
        Self {
            state: AuState::with_descriptor(descriptor),
            webview_handler: handler,
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
        process_mode: ProcessMode,
    ) -> PluginResult<()> {
        self.state
            .prepare(sample_rate, max_frames, bus_config, process_mode)
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

    fn shared_parameters(&self) -> Result<&(dyn SharedParameters + 'static), PluginError> {
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

    fn reset(&mut self) {
        if let Some(processor) = self.state.processor_mut() {
            // Full reset sequence: deactivate then reactivate
            // This matches VST3 behavior and beamer_core documentation
            processor.set_active(false);
            processor.set_active(true);
        }
    }

    fn reset_smoothing(&mut self) {
        if let Some(processor) = self.state.processor_mut() {
            use beamer_core::parameter_types::Parameters;
            processor.parameters().reset_smoothing();
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
        // Get processor, sample_rate and conversion_buffers from prepared state
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

        // The events arrive in sub-blocks that start at their sample offset, so
        // applying them here is sample accurate. A ramp is applied as a change to
        // its end value: the parameter's own smoother interpolates, with the time
        // constant configured at parameter construction, which is also what the
        // other format wrappers do.
        let parameters = processor.parameters_mut();

        // The values are in AU format (plain value, or index for an indexed
        // parameter), like the ones the host sets on the parameter directly.
        let immediate = immediate.iter().map(|e| (e.parameter_address, e.value));
        let ramps = ramps.iter().map(|e| (e.parameter_address, e.end_value));

        for (address, value) in immediate.chain(ramps) {
            // AU parameter addresses map directly to beamer parameter IDs.
            let param_id = address as u32;
            let normalized = crate::bridge::au_value_to_normalized(&*parameters, param_id, value);

            if let Some(param) = Parameters::by_id(&*parameters, param_id) {
                param.set_normalized(normalized);
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

        // Borrow processor and midi_output_buffer simultaneously from the
        // prepared state. This avoids mem::take, which would construct a new
        // default MidiBuffer on the stack - a ~80KB [MidiEvent; 1024] that
        // overflows the audio IO thread's small stack in debug builds.
        let (processor, core_output) = match &mut self.state {
            AuState::Prepared {
                processor,
                midi_output_buffer,
                ..
            } => (processor, midi_output_buffer.as_mut()),
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
                processor.process_midi(&filtered, core_output);

                // Move events to AU's MidiBuffer (avoids cloning Box<SysEx>).
                for event in core_output.drain() {
                    let _ = output.push(event);
                }
                return;
            }
        }

        // No PC filtering needed - process all events directly
        processor.process_midi(input, core_output);

        // Move events to AU's MidiBuffer (avoids cloning Box<SysEx>).
        for event in core_output.drain() {
            let _ = output.push(event);
        }
    }

    fn webview_handler(&self) -> Option<Arc<dyn WebViewHandler>> {
        self.webview_handler.clone()
    }

    fn factory_presets(&self) -> FactoryPresetAccess {
        FactoryPresetAccess {
            count: Presets::count(),
            info: Presets::info,
            apply: apply_preset::<Presets>,
        }
    }
}

/// Apply a factory preset to the shared parameters of a plugin.
///
/// Always applies, also when the preset is the current one: hosts send the
/// same preset again and expect its values back.
fn apply_preset<Presets: FactoryPresets>(parameters: &dyn SharedParameters, index: usize) -> bool {
    match parameters.as_any().downcast_ref::<Presets::Parameters>() {
        Some(parameters) => Presets::apply(index, parameters),
        None => false,
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
    use beamer_core::{AuxiliaryBuffers, BusInfo, BusType, MidiEventKind};
    use std::cell::{Cell, RefCell};
    use std::sync::atomic::{AtomicU64, Ordering};

    // =========================================================================
    // What the mocks report
    // =========================================================================
    //
    // Kept per thread, so that tests running in parallel do not see each other.

    thread_local! {
        /// Runs inside `TestParameters::save_state` on the thread that set it.
        static SAVE_STATE_HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
        /// How often the smoothers were reset on this thread.
        static SMOOTHING_RESETS: Cell<usize> = const { Cell::new(0) };
        /// How often a `TestProcessor` was activated on this thread.
        static ACTIVATIONS: Cell<usize> = const { Cell::new(0) };
        /// The sample rate the smoothers were last given on this thread.
        static SMOOTHING_SAMPLE_RATE: Cell<f64> = const { Cell::new(0.0) };
        /// What `SMOOTHING_SAMPLE_RATE` was when a `TestPlugin` was last
        /// prepared on this thread.
        static SMOOTHING_SAMPLE_RATE_AT_PREPARE: Cell<f64> = const { Cell::new(0.0) };
    }

    fn smoothing_resets() -> usize {
        SMOOTHING_RESETS.with(Cell::get)
    }

    fn activations() -> usize {
        ACTIVATIONS.with(Cell::get)
    }

    /// Latency the `TestProcessor` reports for a gain value.
    fn latency_for(gain: f64) -> u32 {
        (gain * 100.0).round() as u32
    }

    const TEST_TAIL_SAMPLES: u32 = 4410;

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
                    string_id: "",
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
            2
        }
        fn group_info(&self, index: usize) -> Option<GroupInfo> {
            match index {
                0 => Some(GroupInfo::root()),
                1 => Some(GroupInfo::new(1, "Filter", 0)),
                _ => None,
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
        fn save_state(&self) -> Vec<u8> {
            SAVE_STATE_HOOK.with(|hook| {
                if let Some(hook) = hook.borrow().as_ref() {
                    hook();
                }
            });
            let mut data = Vec::new();
            self.save_state_prefixed(&mut data, "");
            data
        }
        fn set_sample_rate(&self, sample_rate: f64) {
            SMOOTHING_SAMPLE_RATE.with(|rate| rate.set(sample_rate));
        }
        fn reset_smoothing(&self) {
            SMOOTHING_RESETS.with(|resets| resets.set(resets.get() + 1));
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
            SMOOTHING_SAMPLE_RATE_AT_PREPARE
                .with(|rate| rate.set(SMOOTHING_SAMPLE_RATE.with(Cell::get)));
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

        fn set_active(&mut self, active: bool) {
            if active {
                ACTIVATIONS.with(|activations| activations.set(activations.get() + 1));
            }
        }

        fn latency_samples(&self) -> u32 {
            latency_for(self.parameters.gain.get_normalized())
        }

        fn tail_samples(&self) -> u32 {
            TEST_TAIL_SAMPLES
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
            .allocate_render_resources(44100.0, 512, &bus_config, ProcessMode::Realtime)
            .expect("Failed to prepare processor");

        processor
    }

    /// Get the gain parameter's normalized value.
    fn get_gain_normalized(processor: &AuProcessor<TestPlugin, TestPresets>) -> f64 {
        let params = processor.shared_parameters().unwrap().store();
        let gain_id = params.info(0).unwrap().id;
        params.get_normalized(gain_id)
    }

    /// Set the gain parameter's normalized value.
    fn set_gain_normalized(processor: &mut AuProcessor<TestPlugin, TestPresets>, value: f64) {
        let params = processor.shared_parameters().unwrap().store();
        let gain_id = params.info(0).unwrap().id;
        processor
            .parameter_store_mut()
            .unwrap()
            .set_normalized(gain_id, value);
    }

    // =========================================================================
    // Declared Bus Tests
    // =========================================================================

    #[test]
    fn test_main_channels_of_declared_buses() {
        // The test plugin declares one stereo input and keeps the default
        // stereo output.
        let processor = AuProcessor::<TestPlugin, TestPresets>::new();
        let declared = crate::bridge::DeclaredBuses::capture(&processor);
        assert_eq!(declared.main_channels(), (2, 2));
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

    // =========================================================================
    // Render Tests
    // =========================================================================

    use crate::buffer_storage::ProcessBufferStorage;
    use crate::buffers::{AudioBuffer, AudioBufferList};
    use crate::render::{create_render_block_f32, AudioTimeStamp, RenderBlockTrait, SMPTETime};
    use crate::shared_state::SharedState;
    use std::sync::{Arc, Mutex};

    const FRAMES: usize = 64;

    /// A plugin whose processor fills its output with a value that tells in
    /// which process mode it was prepared.
    #[derive(Default)]
    struct ModePlugin {
        parameters: TestParameters,
    }

    struct ModeProcessor {
        parameters: TestParameters,
        value: f32,
    }

    const REALTIME_VALUE: f32 = 1.0;
    const OFFLINE_VALUE: f32 = 2.0;

    impl HasParameters for ModePlugin {
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

    impl Descriptor for ModePlugin {
        type Setup = ProcessMode;
        type Processor = ModeProcessor;

        fn prepare(self, mode: ProcessMode) -> Self::Processor {
            ModeProcessor {
                parameters: self.parameters,
                value: match mode {
                    ProcessMode::Offline => OFFLINE_VALUE,
                    _ => REALTIME_VALUE,
                },
            }
        }
    }

    impl HasParameters for ModeProcessor {
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

    impl Processor for ModeProcessor {
        type Descriptor = ModePlugin;

        fn process(
            &mut self,
            buffer: &mut Buffer,
            _aux: &mut AuxiliaryBuffers,
            _context: &ProcessContext,
        ) {
            for (_, output) in buffer.zip_channels() {
                output.fill(self.value);
            }
        }
    }

    /// A stereo AudioBufferList with its sample memory.
    #[repr(C)]
    struct StereoList {
        number_buffers: u32,
        buffers: [AudioBuffer; 2],
        samples: [[f32; FRAMES]; 2],
    }

    impl StereoList {
        fn filled(value: f32) -> Box<Self> {
            let mut list = Box::new(Self {
                number_buffers: 2,
                buffers: [
                    AudioBuffer {
                        number_channels: 1,
                        data_byte_size: (FRAMES * 4) as u32,
                        data: std::ptr::null_mut(),
                    },
                    AudioBuffer {
                        number_channels: 1,
                        data_byte_size: (FRAMES * 4) as u32,
                        data: std::ptr::null_mut(),
                    },
                ],
                samples: [[value; FRAMES]; 2],
            });
            for channel in 0..2 {
                list.buffers[channel].data = list.samples[channel].as_mut_ptr().cast();
            }
            list
        }

        fn as_list(&mut self) -> *mut AudioBufferList {
            (self as *mut Self).cast()
        }
    }

    fn stereo_bus_config() -> CachedBusConfig {
        CachedBusConfig::new(
            vec![CachedBusInfo::new(2, BusType::Main)],
            vec![CachedBusInfo::new(2, BusType::Main)],
        )
    }

    /// Prepare the plugin and create its render block, like the bridge does.
    fn render_block_for(
        plugin: &Arc<Mutex<Box<dyn AuPluginInstance>>>,
        process_mode: ProcessMode,
    ) -> Box<dyn RenderBlockTrait> {
        let bus_config = stereo_bus_config();
        plugin
            .lock()
            .unwrap()
            .allocate_render_resources(44100.0, FRAMES as u32, &bus_config, process_mode)
            .expect("prepare");
        let storage = ProcessBufferStorage::<f32>::allocate_from_config(&bus_config, FRAMES);
        let shared = Arc::new(SharedState::default());
        create_render_block_f32(Arc::clone(plugin), shared, storage, FRAMES as u32, 44100.0)
    }

    /// The timestamp of a first buffer.
    fn timestamp() -> AudioTimeStamp {
        AudioTimeStamp {
            sample_time: 0.0,
            host_time: 0,
            rate_scalar: 1.0,
            word_clock_time: 0,
            smpte_time: SMPTETime::default(),
            flags: 1,
            reserved: 0,
        }
    }

    /// Render one buffer of the main bus and return the first output sample
    /// of each channel.
    fn render(
        block: &dyn RenderBlockTrait,
        input: Option<&mut StereoList>,
        output: &mut StereoList,
    ) -> [f32; 2] {
        let timestamp = timestamp();
        let mut action_flags = 0u32;
        let input_list = input.map_or(std::ptr::null(), |list| list.as_list().cast_const());
        let status = block.process(
            &mut action_flags,
            &timestamp,
            FRAMES as u32,
            0,
            output.as_list(),
            std::ptr::null(),
            std::ptr::null(),
            input_list,
            None,
            None,
        );
        assert_eq!(status, 0);
        [output.samples[0][0], output.samples[1][FRAMES - 1]]
    }

    fn boxed<T: AuPluginInstance + 'static>(instance: T) -> Arc<Mutex<Box<dyn AuPluginInstance>>> {
        Arc::new(Mutex::new(Box::new(instance)))
    }

    #[test]
    fn test_first_render_call_is_processed() {
        let plugin = boxed(AuProcessor::<ModePlugin>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);

        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);
        assert_eq!(
            render(block.as_ref(), Some(&mut input), &mut output),
            [REALTIME_VALUE; 2]
        );
    }

    #[test]
    fn test_preparing_again_delivers_the_process_mode() {
        let plugin = boxed(AuProcessor::<ModePlugin>::new());
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);

        let block = render_block_for(&plugin, ProcessMode::Offline);
        assert_eq!(
            render(block.as_ref(), Some(&mut input), &mut output),
            [OFFLINE_VALUE; 2]
        );

        // The host leaves offline rendering while the plugin is prepared.
        let block = render_block_for(&plugin, ProcessMode::Realtime);
        assert_eq!(
            render(block.as_ref(), Some(&mut input), &mut output),
            [REALTIME_VALUE; 2]
        );
    }

    #[test]
    fn test_output_starts_silent_without_main_input() {
        // TestProcessor does not write its output.
        let plugin = boxed(AuProcessor::<TestPlugin, TestPresets>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);

        // Without input, what the host left in the buffers must not come out.
        let mut output = StereoList::filled(0.77);
        assert_eq!(render(block.as_ref(), None, &mut output), [0.0; 2]);

        // With input the output is the processor's business: the buffers may
        // be the input itself when the host processes in place.
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.77);
        assert_eq!(
            render(block.as_ref(), Some(&mut input), &mut output),
            [0.77; 2]
        );
    }

    // =========================================================================
    // Bridge Tests
    // =========================================================================

    use crate::bridge::{
        beamer_au_allocate_render_resources, beamer_au_apply_preset,
        beamer_au_deallocate_render_resources, beamer_au_get_group_count,
        beamer_au_get_group_info, beamer_au_get_input_bus_channel_count,
        beamer_au_get_input_bus_count, beamer_au_get_latency_samples,
        beamer_au_get_output_bus_channel_count, beamer_au_get_output_bus_count,
        beamer_au_get_parameter_value, beamer_au_get_preset_count, beamer_au_get_preset_info,
        beamer_au_get_state, beamer_au_get_state_size, beamer_au_get_tail_samples,
        beamer_au_render, beamer_au_reset, beamer_au_set_parameter_value, beamer_au_set_state,
        BeamerAuBusConfig, BeamerAuBusInfo, BeamerAuBusType, BeamerAuGroupInfo,
        BeamerAuPresetInfo, BeamerAuSampleFormat, BeamerInstanceHandle,
    };
    use beamer_core::MAX_BUSES;
    use std::ffi::CStr;

    /// An instance pointer that a test hands to a second thread.
    struct Instance(*mut BeamerInstanceHandle);
    // SAFETY: The bridge functions are made to be called from any thread.
    unsafe impl Send for Instance {}

    fn test_handle() -> Box<BeamerInstanceHandle> {
        let plugin = AuProcessor::<TestPlugin, TestPresets>::new();
        Box::new(BeamerInstanceHandle::new(Box::new(plugin)))
    }

    fn gain_id() -> u32 {
        fnv1a_hash("gain")
    }

    /// Allocate render resources for a stereo effect through the C-ABI.
    fn allocate(instance: *mut BeamerInstanceHandle, rendering_offline: bool) -> i32 {
        let stereo = BeamerAuBusInfo {
            channel_count: 2,
            bus_type: BeamerAuBusType::Main,
        };
        let bus_config = BeamerAuBusConfig {
            input_bus_count: 1,
            output_bus_count: 1,
            input_buses: [stereo; MAX_BUSES],
            output_buses: [stereo; MAX_BUSES],
        };
        beamer_au_allocate_render_resources(
            instance,
            44100.0,
            FRAMES as u32,
            BeamerAuSampleFormat::Float32,
            &bus_config,
            rendering_offline,
        )
    }

    #[test]
    fn test_parameter_access_follows_the_parameters() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        beamer_au_set_parameter_value(instance, gain_id(), 0.25);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);

        // Preparing moves the parameters into the processor.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
        beamer_au_set_parameter_value(instance, gain_id(), 0.75);

        // Preparing again for offline rendering moves them twice.
        assert_eq!(allocate(instance, true), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.75);

        // Deallocating moves them back.
        beamer_au_deallocate_render_resources(instance);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.75);
    }

    #[test]
    fn test_parameter_access_while_the_parameters_are_locked() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);

        // SAFETY: The handle is alive, and the calls below only use it shared.
        let shared = unsafe { &*instance };
        // With the cached pointer locked, as it is during a move, access goes
        // through the plugin instead of failing or using the pointer.
        shared.with_shared_parameters_locked(|| {
            beamer_au_set_parameter_value(instance, gain_id(), 0.125);
            assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.125);
        });
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.125);
    }

    #[test]
    fn test_parameter_access_during_repeated_preparing() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);

        // One thread sets and reads the parameter like an automation thread,
        // while this one switches the offline state back and forth.
        let setter = Instance(instance);
        let worker = std::thread::spawn(move || {
            let setter = setter;
            for i in 0..20_000u32 {
                let value = (i % 5) as f32 * 0.25;
                beamer_au_set_parameter_value(setter.0, gain_id(), value);
                let read = beamer_au_get_parameter_value(setter.0, gain_id());
                assert!((0.0..=1.0).contains(&read));
            }
        });
        for i in 0..200 {
            assert_eq!(allocate(instance, i % 2 == 0), 0);
        }
        worker.join().expect("the parameter thread panicked");

        beamer_au_set_parameter_value(instance, gain_id(), 0.5);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.5);
    }

    // =========================================================================
    // Host queries while audio runs
    // =========================================================================
    //
    // The render call fails when it finds the plugin mutex held, so nothing a
    // host calls during playback may take it.

    /// Render one buffer of the main bus through the C-ABI.
    fn render_instance(instance: *mut BeamerInstanceHandle) -> i32 {
        let timestamp = timestamp();
        let mut action_flags = 0u32;
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);
        beamer_au_render(
            instance,
            &mut action_flags,
            &timestamp,
            FRAMES as u32,
            0,
            output.as_list(),
            std::ptr::null(),
            std::ptr::null(),
            input.as_list().cast_const(),
            std::ptr::null(),
            None,
            std::ptr::null_mut(),
        )
    }

    /// The state of the instance, as a host reads it.
    fn saved_state(instance: *mut BeamerInstanceHandle) -> Vec<u8> {
        let mut state = vec![0u8; beamer_au_get_state_size(instance) as usize];
        let written = beamer_au_get_state(instance, state.as_mut_ptr(), state.len() as u32);
        assert_eq!(written as usize, state.len());
        state
    }

    fn load_state(instance: *mut BeamerInstanceHandle, state: &[u8]) -> i32 {
        beamer_au_set_state(instance, state.as_ptr(), state.len() as u32)
    }

    fn plugin_lock_count(instance: *mut BeamerInstanceHandle) -> usize {
        // SAFETY: The handle is alive, and it is only used shared.
        unsafe { &*instance }.plugin_lock_count()
    }

    #[test]
    fn test_host_queries_do_not_lock_the_plugin() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        let locks = plugin_lock_count(instance);

        // State: saved, changed and loaded again.
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);
        let state = saved_state(instance);
        assert!(!state.is_empty());
        beamer_au_set_parameter_value(instance, gain_id(), 0.75);
        assert_eq!(load_state(instance, &state), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);

        // Presets.
        assert_eq!(beamer_au_get_preset_count(instance), 3);
        let mut preset = BeamerAuPresetInfo::default();
        assert!(beamer_au_get_preset_info(instance, 2, &mut preset));
        assert_eq!(preset.number, 2);
        // SAFETY: The bridge terminates the name.
        assert_eq!(unsafe { CStr::from_ptr(preset.name.as_ptr()) }.to_str(), Ok("Boost"));
        assert!(!beamer_au_get_preset_info(instance, 3, &mut preset));
        assert!(beamer_au_apply_preset(instance, 2));
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.75);
        assert!(!beamer_au_apply_preset(instance, 3));

        // Groups.
        assert_eq!(beamer_au_get_group_count(instance), 2);
        let mut group = BeamerAuGroupInfo::default();
        assert!(beamer_au_get_group_info(instance, 1, &mut group));
        assert_eq!(group.id, 1);
        // SAFETY: The bridge terminates the name.
        assert_eq!(unsafe { CStr::from_ptr(group.name.as_ptr()) }.to_str(), Ok("Filter"));
        assert!(!beamer_au_get_group_info(instance, 2, &mut group));

        // Buses: the test plugin declares one stereo input and keeps the
        // default stereo output.
        assert_eq!(beamer_au_get_input_bus_count(instance), 1);
        assert_eq!(beamer_au_get_output_bus_count(instance), 1);
        assert_eq!(beamer_au_get_input_bus_channel_count(instance, 0), 2);
        assert_eq!(beamer_au_get_output_bus_channel_count(instance, 0), 2);

        // Latency, tail and a reset.
        assert_eq!(beamer_au_get_tail_samples(instance), TEST_TAIL_SAMPLES);
        beamer_au_get_latency_samples(instance);
        beamer_au_reset(instance);

        assert_eq!(plugin_lock_count(instance), locks);
    }

    #[test]
    fn test_render_is_not_refused_while_the_state_is_saved() {
        use std::sync::mpsc;
        use std::time::Duration;

        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);

        // A host thread saves the state and stays inside the save until the
        // render call below is through.
        let (inside_tx, inside_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let saver = Instance(instance);
        let host = std::thread::spawn(move || {
            let saver = saver;
            SAVE_STATE_HOOK.with(|hook| {
                *hook.borrow_mut() = Some(Box::new(move || {
                    inside_tx.send(()).expect("the test waits for the save");
                    let _ = release_rx.recv_timeout(Duration::from_secs(10));
                }));
            });
            let mut state = [0u8; 64];
            beamer_au_get_state(saver.0, state.as_mut_ptr(), state.len() as u32)
        });
        inside_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the host thread reaches the save");

        let status = render_instance(instance);

        release_tx.send(()).expect("the host thread waits");
        let written = host.join().expect("the host thread panicked");
        assert_eq!(status, 0);
        assert!(written > 0);
    }

    #[test]
    fn test_state_is_the_same_prepared_or_not() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        // Before preparing: the state is saved and a loaded one applies.
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);
        let quiet = saved_state(instance);
        assert!(!quiet.is_empty());
        beamer_au_set_parameter_value(instance, gain_id(), 0.75);
        assert_eq!(load_state(instance, &quiet), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);

        // It moves into the processor, and is saved to the same bytes there.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(saved_state(instance), quiet);

        // And back.
        beamer_au_set_parameter_value(instance, gain_id(), 0.75);
        let boost = saved_state(instance);
        beamer_au_deallocate_render_resources(instance);
        assert_eq!(saved_state(instance), boost);
        assert_eq!(load_state(instance, &quiet), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);

        // An empty state changes nothing.
        assert_eq!(load_state(instance, &[]), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
    }

    #[test]
    fn test_loaded_state_resets_the_smoothers_in_the_next_block() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(render_instance(instance), 0);
        let state = saved_state(instance);
        let before = smoothing_resets();

        // The smoothers belong to the audio thread: loading leaves them alone.
        assert_eq!(load_state(instance, &state), 0);
        assert_eq!(smoothing_resets(), before);

        // The next block resets them, once.
        assert_eq!(render_instance(instance), 0);
        assert_eq!(smoothing_resets(), before + 1);
        assert_eq!(render_instance(instance), 0);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_state_loaded_before_preparing_resets_the_smoothers_in_the_first_block() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        let state = saved_state(instance);
        assert_eq!(load_state(instance, &state), 0);
        let before = smoothing_resets();

        assert_eq!(allocate(instance, false), 0);
        assert_eq!(render_instance(instance), 0);
        assert_eq!(smoothing_resets(), before + 1);
    }

    #[test]
    fn test_preparing_gives_the_smoothers_the_host_sample_rate() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        SMOOTHING_SAMPLE_RATE.with(|rate| rate.set(0.0));

        // The rate is there when the plugin creates its processor.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(SMOOTHING_SAMPLE_RATE_AT_PREPARE.with(Cell::get), 44100.0);

        // Preparing again does it again.
        SMOOTHING_SAMPLE_RATE.with(|rate| rate.set(0.0));
        assert_eq!(allocate(instance, true), 0);
        assert_eq!(SMOOTHING_SAMPLE_RATE_AT_PREPARE.with(Cell::get), 44100.0);
    }

    #[test]
    fn test_reset_runs_in_the_next_block() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(render_instance(instance), 0);
        let before = activations();

        // The processor belongs to the audio thread: the request waits.
        beamer_au_reset(instance);
        assert_eq!(activations(), before);

        assert_eq!(render_instance(instance), 0);
        assert_eq!(activations(), before + 1);
        assert_eq!(render_instance(instance), 0);
        assert_eq!(activations(), before + 1);
    }

    #[test]
    fn test_reset_before_preparing_is_dropped() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        // A new processor starts clean, there is nothing left to reset.
        beamer_au_reset(instance);
        assert_eq!(allocate(instance, false), 0);
        let before = activations();
        assert_eq!(render_instance(instance), 0);
        assert_eq!(activations(), before);
    }

    #[test]
    fn test_latency_and_tail_are_published_by_the_audio_thread() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        // Unprepared: neither.
        assert_eq!(beamer_au_get_latency_samples(instance), 0);
        assert_eq!(beamer_au_get_tail_samples(instance), 0);

        // Prepared: known before the first render call.
        beamer_au_set_parameter_value(instance, gain_id(), 0.5);
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(beamer_au_get_latency_samples(instance), latency_for(0.5));
        assert_eq!(beamer_au_get_tail_samples(instance), TEST_TAIL_SAMPLES);

        // A change shows with the next block.
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);
        assert_eq!(beamer_au_get_latency_samples(instance), latency_for(0.5));
        assert_eq!(render_instance(instance), 0);
        assert_eq!(beamer_au_get_latency_samples(instance), latency_for(0.25));

        // Unprepared again: neither.
        beamer_au_deallocate_render_resources(instance);
        assert_eq!(beamer_au_get_latency_samples(instance), 0);
        assert_eq!(beamer_au_get_tail_samples(instance), 0);
    }

    #[test]
    fn test_deallocate_that_cannot_take_the_plugin_keeps_the_parameters() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);

        // SAFETY: The handle is alive, and it is only used shared here.
        let shared = unsafe { &*instance };
        // With the plugin held elsewhere nothing is unprepared and nothing
        // moves, so the cached parameters stay valid.
        shared.with_plugin_locked(|| beamer_au_deallocate_render_resources(instance));

        let locks = plugin_lock_count(instance);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
        assert!(!saved_state(instance).is_empty());
        assert_eq!(plugin_lock_count(instance), locks);
        assert_eq!(beamer_au_get_tail_samples(instance), TEST_TAIL_SAMPLES);
    }

    #[test]
    fn test_host_queries_during_repeated_preparing() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        let state = saved_state(instance);

        // One thread asks what a host asks during playback, while this one
        // switches the offline state back and forth.
        let asker = Instance(instance);
        let expected = state.clone();
        let worker = std::thread::spawn(move || {
            let asker = asker;
            for i in 0..5_000u32 {
                assert_eq!(saved_state(asker.0).len(), expected.len());
                assert_eq!(load_state(asker.0, &expected), 0);
                assert!(beamer_au_apply_preset(asker.0, i % 3));
                assert_eq!(beamer_au_get_group_count(asker.0), 2);
                assert_eq!(beamer_au_get_input_bus_count(asker.0), 1);
                let tail = beamer_au_get_tail_samples(asker.0);
                assert!(tail == 0 || tail == TEST_TAIL_SAMPLES);
                beamer_au_reset(asker.0);
            }
        });
        for i in 0..200 {
            assert_eq!(allocate(instance, i % 2 == 0), 0);
        }
        worker.join().expect("the host thread panicked");

        assert_eq!(load_state(instance, &state), 0);
        assert_eq!(saved_state(instance), state);
    }
}
