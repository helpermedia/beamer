//! Generic AU processor wrapper for type erasure.
//!
//! This module provides `AuProcessor<P>`, a generic wrapper that bridges any
//! `beamer_core::Model` implementation to the AU API through the `AuPluginInstance`
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
//! The processor keeps the plugin's model for the whole life of the
//! instance and manages two lifecycle states via `AuState<P>`:
//! - **Unprepared**: The model is available, no audio resources
//! - **Prepared**: A `Processor` and its resources exist, ready for `process()` calls
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
use crate::instance::{AuPluginInstance, FactoryPresetAccess};
use crate::lifecycle::{AuState, Prepared};
use beamer_core::{
    AuxiliaryBuffers, Buffer, CachedBusConfig, FactoryPresets, Model,
    MidiEvent, NoPresets, ProcessContext, ProcessMode, Processor, SharedModel, WebViewDispatch,
    WebViewEventQueue, MAX_AUX_BUSES,
};

/// Generic AU processor wrapper.
///
/// Mirrors `Vst3Processor<P>` - wraps any `Model` implementation
/// and implements `AuPluginInstance` for type erasure.
///
/// # Type Parameters
///
/// * `P` - The model type implementing `beamer_core::Model`
/// * `Presets` - Optional factory presets collection (default: `NoPresets`)
pub struct AuProcessor<P, Presets = NoPresets<P>>
where
    P: Model,
    Presets: FactoryPresets<Parameters = P>,
{
    state: AuState<P>,
    /// The plugin's queue of events for the web UI, if its model has one.
    webview_events: Option<WebViewEventQueue>,
    _presets: PhantomData<Presets>,
}

impl<P, Presets> AuProcessor<P, Presets>
where
    P: Model,
    Presets: FactoryPresets<Parameters = P>,
{
    /// Create a new AU processor.
    ///
    /// The processor starts in the Unprepared state with a default
    /// plugin instance. Call `allocate_render_resources` to prepare
    /// for audio processing.
    pub fn new() -> Self {
        let model = Arc::new(P::default());

        // Taking the queue here marks it as delivered by this instance.
        let events = beamer_core::WebViewEvents::webview_events(&*model).cloned();
        if let Some(events) = &events {
            events.claim();
        }

        Self {
            state: AuState::with_model(model),
            webview_events: events,
            _presets: PhantomData,
        }
    }
}

impl<P, Presets> Default for AuProcessor<P, Presets>
where
    P: Model,
    Presets: FactoryPresets<Parameters = P>,
{
    fn default() -> Self {
        Self::new()
    }
}

impl<P, Presets> AuPluginInstance for AuProcessor<P, Presets>
where
    P: Model,
    Presets: FactoryPresets<Parameters = P>,
{
    fn allocate_render_resources(
        &mut self,
        sample_rate: f64,
        max_frames: u32,
        bus_config: &CachedBusConfig,
        process_mode: ProcessMode,
    ) -> PluginResult<()> {
        self.state
            .prepare(sample_rate, max_frames, bus_config, process_mode);
        Ok(())
    }

    fn deallocate_render_resources(&mut self) {
        self.state.unprepare();
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

    fn shared_model(&self) -> Arc<dyn SharedModel> {
        // Clone the concrete `Arc`, then let it coerce to the trait object.
        let model: Arc<P> = Arc::clone(&self.state.model);
        model
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
        self.state.model.input_bus_count()
    }

    fn declared_output_bus_count(&self) -> usize {
        self.state.model.output_bus_count()
    }

    fn declared_input_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo> {
        self.state.model.input_bus_info(index)
    }

    fn declared_output_bus_info(&self, index: usize) -> Option<beamer_core::BusInfo> {
        self.state.model.output_bus_info(index)
    }

    fn process_block(
        &mut self,
        buffer: &mut Buffer<'_, f32>,
        aux: &mut AuxiliaryBuffers<'_, f32>,
        context: &ProcessContext,
    ) -> PluginResult<()> {
        let Some(Prepared { processor, .. }) = &mut self.state.prepared else {
            return Err(PluginError::ProcessingError("not prepared".to_string()));
        };

        processor.process(buffer, aux, context);
        Ok(())
    }

    fn process_block_f64(
        &mut self,
        buffer: &mut Buffer<'_, f64>,
        aux: &mut AuxiliaryBuffers<'_, f64>,
        context: &ProcessContext,
    ) -> PluginResult<()> {
        let Some(Prepared {
                processor,
                conversion_buffers,
                ..
            }) = &mut self.state.prepared
        else {
            return Err(PluginError::ProcessingError("not prepared".to_string()));
        };

        if processor.supports_double_precision() {
            processor.process_f64(buffer, aux, context);
            return Ok(());
        }

        // Convert f64 to f32 in the buffers allocated when the plugin was
        // prepared, process and convert back. Nothing here allocates.
        let conversion = conversion_buffers.as_mut().ok_or_else(|| {
            PluginError::ProcessingError("no conversion buffers for f64 processing".to_string())
        })?;
        let num_samples = buffer.num_samples();

        for (ch, input) in buffer.inputs().enumerate() {
            if let Some(converted) = conversion.main_input_mut(ch) {
                for (to, from) in converted.iter_mut().zip(input) {
                    *to = *from as f32;
                }
            }
        }
        // Walk the buses by index, so that a bus after an unconnected one
        // keeps its own conversion buffers.
        for bus_index in 0..aux.num_input_buses() {
            let Some(bus) = aux.input(bus_index) else {
                continue;
            };
            for (ch, input) in bus.iter_inputs().enumerate() {
                if let Some(converted) = conversion.aux_input_mut(bus_index, ch, num_samples) {
                    for (to, from) in converted.iter_mut().zip(input) {
                        *to = *from as f32;
                    }
                }
            }
        }

        // The processor starts from what the host's output buffers hold, as
        // in the 32-bit path, where it writes into them directly. They are
        // silent when there is no main input.
        for (ch, output) in buffer.outputs_mut().enumerate() {
            if let Some(converted) = conversion.main_output_mut(ch) {
                for (to, from) in converted.iter_mut().zip(output.iter()) {
                    *to = *from as f32;
                }
            }
        }
        for bus_index in 0..aux.num_output_buses() {
            let Some(mut bus) = aux.output(bus_index) else {
                continue;
            };
            for (ch, output) in bus.iter_outputs().enumerate() {
                if let Some(converted) = conversion.aux_output_mut(bus_index, ch, num_samples) {
                    for (to, from) in converted.iter_mut().zip(output.iter()) {
                        *to = *from as f32;
                    }
                }
            }
        }

        // The channels the host connected on each bus. The processor sees
        // only those, as in the 32-bit path, and not a channel holding the
        // samples of the last block in which it was connected.
        let main_input_channels = buffer.num_input_channels();
        let main_output_channels = buffer.num_output_channels();
        let mut aux_input_channels = [0usize; MAX_AUX_BUSES];
        for (bus_index, channels) in aux_input_channels.iter_mut().enumerate() {
            *channels = aux.input(bus_index).map_or(0, |bus| bus.num_channels());
        }
        let mut aux_output_channels = [0usize; MAX_AUX_BUSES];
        for (bus_index, channels) in aux_output_channels.iter_mut().enumerate() {
            *channels = aux.output(bus_index).map_or(0, |bus| bus.num_channels());
        }

        {
            let main_inputs = conversion
                .main_input_f32
                .iter()
                .take(main_input_channels)
                .map(|ch| &ch[..num_samples]);
            let main_outputs = conversion
                .main_output_f32
                .iter_mut()
                .take(main_output_channels)
                .map(|ch| &mut ch[..num_samples]);
            let aux_inputs = conversion
                .aux_input_f32
                .iter()
                .zip(aux_input_channels)
                .map(|(bus, channels)| bus.iter().take(channels).map(|ch| &ch[..num_samples]));
            let aux_outputs = conversion
                .aux_output_f32
                .iter_mut()
                .zip(aux_output_channels)
                .map(|(bus, channels)| {
                    bus.iter_mut().take(channels).map(|ch| &mut ch[..num_samples])
                });

            let mut buffer_f32 = Buffer::new(main_inputs, main_outputs, num_samples);
            let mut aux_f32 = AuxiliaryBuffers::new(aux_inputs, aux_outputs, num_samples);
            processor.process(&mut buffer_f32, &mut aux_f32, context);
        }

        for (ch, output) in buffer.outputs_mut().enumerate() {
            if let Some(converted) = conversion.main_output(ch) {
                for (to, from) in output.iter_mut().zip(converted) {
                    *to = *from as f64;
                }
            }
        }
        for bus_index in 0..aux.num_output_buses() {
            let Some(mut bus) = aux.output(bus_index) else {
                continue;
            };
            for (ch, output) in bus.iter_outputs().enumerate() {
                if let Some(converted) = conversion.aux_output(bus_index, ch, num_samples) {
                    for (to, from) in output.iter_mut().zip(converted) {
                        *to = *from as f64;
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
        if !self.state.is_prepared() {
            return Ok(());
        }

        use beamer_core::parameter_types::Parameters;

        // The events arrive in sub-blocks that start at their sample offset, so
        // applying them here is sample accurate. A ramp is applied as a change to
        // its end value: the parameter's own smoother interpolates, with the time
        // constant configured at parameter construction, which is also what the
        // other format wrappers do.
        let model: &P = &self.state.model;

        // The values are in AU format (plain value, or index for an indexed
        // parameter), like the ones the host sets on the parameter directly.
        let immediate = immediate.iter().map(|e| (e.parameter_address, e.value));
        let ramps = ramps.iter().map(|e| (e.parameter_address, e.end_value));

        for (address, value) in immediate.chain(ramps) {
            // AU parameter addresses map directly to beamer parameter IDs.
            let param_id = address as u32;
            let normalized = crate::bridge::au_value_to_normalized(model, param_id, value);

            if let Some(param) = Parameters::by_id(model, param_id) {
                param.set_normalized(normalized);
            }
        }

        Ok(())
    }

    fn midi_cc_state(&self) -> Option<&beamer_core::MidiCcState> {
        self.state.midi_cc_state()
    }

    fn process_midi(&mut self, input: &mut [MidiEvent], output: &mut crate::render::MidiBuffer) {
        use beamer_core::MidiEventKind;

        // Check if we have factory presets for automatic MIDI PC mapping
        let preset_count = Presets::count();

        // Borrow processor and midi_output_buffer simultaneously from the
        // prepared state. This avoids mem::take, which would construct a new
        // default MidiBuffer on the stack - a ~80KB [MidiEvent; 1024] that
        // overflows the audio IO thread's small stack in debug builds.
        let Some(Prepared {
            processor,
            midi_output_buffer,
            ..
        }) = &mut self.state.prepared
        else {
            // Not prepared - pass through events unchanged
            for event in input {
                let _ = output.push(std::mem::take(event));
            }
            return;
        };
        let core_output = midi_output_buffer.as_mut();
        let model: &P = &self.state.model;

        // Clear for reuse.
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
                // Apply the presets of the matching program changes and move
                // the other events to the front, in their order. This works
                // in the render call's own buffer, without allocating.
                let mut kept = 0;
                for index in 0..input.len() {
                    if let MidiEventKind::ProgramChange(pc) = &input[index].event {
                        if (pc.program as usize) < preset_count {
                            Presets::apply(pc.program as usize, model);
                            continue;
                        }
                    }
                    input.swap(kept, index);
                    kept += 1;
                }

                // Process remaining events through the plugin
                processor.process_midi(&input[..kept], core_output);

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

    fn webview_dispatch(&self) -> Arc<dyn WebViewDispatch> {
        Arc::clone(&self.state.model) as Arc<dyn WebViewDispatch>
    }

    fn webview_events(&self) -> Option<WebViewEventQueue> {
        self.webview_events.clone()
    }

    fn factory_presets(&self) -> FactoryPresetAccess {
        FactoryPresetAccess {
            count: Presets::count(),
            info: Presets::info,
            apply: apply_preset::<Presets>,
        }
    }
}

/// Apply a factory preset to the shared model of a plugin.
///
/// Always applies, also when the preset is the current one: hosts send the
/// same preset again and expect its values back.
fn apply_preset<Presets: FactoryPresets>(model: &dyn SharedModel, index: usize) -> bool {
    match model.as_any().downcast_ref::<Presets::Parameters>() {
        Some(model) => Presets::apply(index, model),
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
    P: Model,
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
        /// Runs inside `TestFields::save_state` on the thread that set it.
        static SAVE_STATE_HOOK: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
        /// How often the smoothers were reset on this thread.
        static SMOOTHING_RESETS: Cell<usize> = const { Cell::new(0) };
        /// How often a `TestProcessor` was activated on this thread.
        static ACTIVATIONS: Cell<usize> = const { Cell::new(0) };
        /// The sample rate the smoothers were last given on this thread.
        static SMOOTHING_SAMPLE_RATE: Cell<f64> = const { Cell::new(0.0) };
        /// What `SMOOTHING_SAMPLE_RATE` was when a `TestModel` was last
        /// prepared on this thread.
        static SMOOTHING_SAMPLE_RATE_AT_PREPARE: Cell<f64> = const { Cell::new(0.0) };
        /// How many `TestProcessor`s exist on this thread.
        static LIVE_PROCESSORS: Cell<usize> = const { Cell::new(0) };
        /// What `LIVE_PROCESSORS` was when a `TestModel` was last prepared
        /// on this thread.
        static LIVE_PROCESSORS_AT_PREPARE: Cell<usize> = const { Cell::new(0) };
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
                    string_id: std::borrow::Cow::Borrowed(""),
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
    // The Fields of the Test Models
    // =========================================================================

    struct TestFields {
        gain: MockParameter,
        /// State beyond parameters, of any size.
        blob: beamer_core::State<Vec<u8>>,
    }

    impl TestFields {
        fn new() -> Self {
            Self {
                gain: MockParameter::new(fnv1a_hash("gain"), "Gain", 0.5),
                blob: beamer_core::State::default(),
            }
        }
    }

    impl ParameterGroups for TestFields {
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

    impl Parameters for TestFields {
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
        fn set_sample_rate(&self, sample_rate: f64) {
            SMOOTHING_SAMPLE_RATE.with(|rate| rate.set(sample_rate));
        }
        fn reset_smoothing(&self) {
            SMOOTHING_RESETS.with(|resets| resets.set(resets.get() + 1));
        }
    }

    impl beamer_core::Saved for TestFields {
        fn save_state(&self) -> Vec<u8> {
            SAVE_STATE_HOOK.with(|hook| {
                if let Some(hook) = hook.borrow().as_ref() {
                    hook();
                }
            });
            let mut writer = beamer_core::state_format::StateWriter::new();
            self.save_entries(&mut writer, "");
            writer.finish()
        }
        fn save_entries(&self, writer: &mut beamer_core::state_format::StateWriter, _prefix: &str) {
            writer.parameter("gain", &self.gain);
            writer.state("blob", &self.blob);
        }
        fn load_entries(&self, entries: &beamer_core::state_format::StateEntries<'_>, _prefix: &str) {
            entries.load_parameter("gain", &self.gain);
            entries.load_state("blob", &self.blob);
        }
    }

    impl beamer_core::WebViewEvents for TestFields {}

    // =========================================================================
    // Mock Plugin
    // =========================================================================

    #[derive(Default)]
    struct TestModel {
        fields: TestFields,
    }

    impl Default for TestFields {
        fn default() -> Self {
            Self::new()
        }
    }

    beamer_core::__forward_model!(TestModel, fields);

    impl Model for TestModel {
        type Setup = ();
        type Processor = TestProcessor;

        fn prepare(self: Arc<Self>, _setup: ()) -> Self::Processor {
            SMOOTHING_SAMPLE_RATE_AT_PREPARE
                .with(|rate| rate.set(SMOOTHING_SAMPLE_RATE.with(Cell::get)));
            LIVE_PROCESSORS_AT_PREPARE.with(|live| live.set(LIVE_PROCESSORS.with(Cell::get)));
            LIVE_PROCESSORS.with(|live| live.set(live.get() + 1));
            TestProcessor { model: self }
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
        model: Arc<TestModel>,
    }

    impl Drop for TestProcessor {
        fn drop(&mut self) {
            // A thread that ends may have dropped its counter already.
            let _ = LIVE_PROCESSORS.try_with(|live| live.set(live.get().saturating_sub(1)));
        }
    }

    impl Processor for TestProcessor {
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
            latency_for(self.model.fields.gain.get_normalized())
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
        type Parameters = TestModel;

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
    fn create_prepared_processor() -> AuProcessor<TestModel, TestPresets> {
        let mut processor = AuProcessor::<TestModel, TestPresets>::new();

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
    fn get_gain_normalized(processor: &AuProcessor<TestModel, TestPresets>) -> f64 {
        let model = processor.shared_model();
        let parameters = model.parameter_store();
        let gain_id = parameters.info(0).unwrap().id;
        parameters.get_normalized(gain_id)
    }

    /// Set the gain parameter's normalized value.
    fn set_gain_normalized(processor: &mut AuProcessor<TestModel, TestPresets>, value: f64) {
        let model = processor.shared_model();
        let parameters = model.parameter_store();
        let gain_id = parameters.info(0).unwrap().id;
        parameters.set_normalized(gain_id, value);
    }

    // =========================================================================
    // Declared Bus Tests
    // =========================================================================

    #[test]
    fn test_main_channels_of_declared_buses() {
        // The test plugin declares one stereo input and keeps the default
        // stereo output.
        let processor = AuProcessor::<TestModel, TestPresets>::new();
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
        let mut input = vec![MidiEvent::program_change(0, 0, 1)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

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
        let mut input = vec![MidiEvent::program_change(0, 0, 10)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

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
        let mut input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),
            MidiEvent::control_change(10, 0, 7, 0.8),
            MidiEvent::control_change(20, 0, 10, 0.5),
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

        // All events should pass through
        assert_eq!(output.len(), 3, "Non-PC events should pass through");
    }

    #[test]
    fn midi_mixed_events_filters_only_valid_pc() {
        let mut processor = create_prepared_processor();

        // Mix of events: CC, valid PC, CC, invalid PC, CC
        let mut input = vec![
            MidiEvent::control_change(0, 0, 1, 0.5),
            MidiEvent::program_change(0, 0, 2), // Boost preset (valid)
            MidiEvent::control_change(10, 0, 7, 0.8),
            MidiEvent::program_change(20, 0, 50), // Invalid (out of range)
            MidiEvent::control_change(30, 0, 10, 0.5),
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

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
        let mut input = vec![MidiEvent::program_change(0, 0, 0)];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

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
        let mut input = vec![
            MidiEvent::program_change(0, 0, 0),  // Unity (0.5)
            MidiEvent::program_change(10, 0, 1), // Quiet (0.25)
            MidiEvent::program_change(20, 0, 2), // Boost (0.75) - last, should win
        ];
        let mut output = MidiBuffer::with_capacity(16);

        processor.process_midi(&mut input, &mut output);

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
    use crate::render::{
        create_render_block_f32, create_render_block_f64, AudioTimeStamp, RenderBlockTrait, SMPTETime,
    };
    use crate::shared_state::SharedState;
    use std::sync::{Arc, Mutex};

    const FRAMES: usize = 64;

    /// A plugin whose processor fills its output with a value that tells in
    /// which process mode it was prepared.
    #[derive(Default)]
    struct ModeModel {
        fields: TestFields,
    }

    struct ModeProcessor {
        value: f32,
    }

    const REALTIME_VALUE: f32 = 1.0;
    const OFFLINE_VALUE: f32 = 2.0;

    beamer_core::__forward_model!(ModeModel, fields);

    impl Model for ModeModel {
        type Setup = ProcessMode;
        type Processor = ModeProcessor;

        fn prepare(self: Arc<Self>, mode: ProcessMode) -> Self::Processor {
            ModeProcessor {
                value: match mode {
                    ProcessMode::Offline => OFFLINE_VALUE,
                    _ => REALTIME_VALUE,
                },
            }
        }
    }

    impl Processor for ModeProcessor {
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
        let model = plugin.lock().unwrap().shared_model();
        let storage = ProcessBufferStorage::<f32>::allocate_from_config(&bus_config, FRAMES);
        let shared = Arc::new(SharedState::default());
        create_render_block_f32(
            Arc::clone(plugin),
            model,
            shared,
            storage,
            FRAMES as u32,
            44100.0,
        )
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
        let plugin = boxed(AuProcessor::<ModeModel>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);

        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);
        assert_eq!(
            render(block.as_ref(), Some(&mut input), &mut output),
            [REALTIME_VALUE; 2]
        );
    }

    /// Counts the heap allocations of the thread that asks for it.
    mod allocations {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local! {
            /// Allocations counted on this thread, `None` while not counting.
            static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
        }

        pub struct Counting;

        // SAFETY: Every call is passed on to the system allocator unchanged.
        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                // A thread that ends may have dropped its counter already.
                let _ = COUNT.try_with(|count| {
                    if let Some(n) = count.get() {
                        count.set(Some(n + 1));
                    }
                });
                // SAFETY: The caller's guarantees for `alloc` are passed on.
                unsafe { System.alloc(layout) }
            }

            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                // SAFETY: The caller's guarantees for `dealloc` are passed on.
                unsafe { System.dealloc(ptr, layout) }
            }

            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                let _ = COUNT.try_with(|count| {
                    if let Some(n) = count.get() {
                        count.set(Some(n + 1));
                    }
                });
                // SAFETY: The caller's guarantees for `realloc` are passed on.
                unsafe { System.realloc(ptr, layout, new_size) }
            }
        }

        #[global_allocator]
        static ALLOCATOR: Counting = Counting;

        /// How many allocations `f` makes on this thread.
        pub fn count(f: impl FnOnce()) -> usize {
            COUNT.with(|count| count.set(Some(0)));
            f();
            COUNT.with(|count| count.replace(None)).unwrap_or(0)
        }
    }

    #[test]
    fn test_render_call_does_not_allocate() {
        let plugin = boxed(AuProcessor::<ModeModel>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);

        // The first call may set up what stays for the life of the block.
        render(block.as_ref(), Some(&mut input), &mut output);

        let allocations = allocations::count(|| {
            for _ in 0..8 {
                render(block.as_ref(), Some(&mut input), &mut output);
            }
        });
        assert_eq!(allocations, 0);
    }

    /// A plugin whose processor doubles its input, in f32 only.
    #[derive(Default)]
    struct DoublingModel {
        fields: TestFields,
    }

    struct DoublingProcessor;

    beamer_core::__forward_model!(DoublingModel, fields);

    impl Model for DoublingModel {
        type Setup = ();
        type Processor = DoublingProcessor;

        fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
            DoublingProcessor
        }
    }

    thread_local! {
        /// Whether the last `DoublingProcessor::process` call on this
        /// thread ran inside a process scope.
        static SAW_PROCESS_SCOPE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
    }

    impl Processor for DoublingProcessor {
        fn process(
            &mut self,
            buffer: &mut Buffer,
            _aux: &mut AuxiliaryBuffers,
            _context: &ProcessContext,
        ) {
            SAW_PROCESS_SCOPE.with(|saw| saw.set(Some(beamer_core::audio_thread::in_process())));
            for (input, output) in buffer.zip_channels() {
                for (from, to) in input.iter().zip(output) {
                    *to = from * 2.0;
                }
            }
        }
    }

    /// A stereo AudioBufferList of 64-bit samples with its sample memory.
    #[repr(C)]
    struct StereoList64 {
        number_buffers: u32,
        buffers: [AudioBuffer; 2],
        samples: [[f64; FRAMES]; 2],
    }

    impl StereoList64 {
        fn filled(value: f64) -> Box<Self> {
            let buffer = AudioBuffer {
                number_channels: 1,
                data_byte_size: (FRAMES * 8) as u32,
                data: std::ptr::null_mut(),
            };
            let mut list = Box::new(Self {
                number_buffers: 2,
                buffers: [buffer, buffer],
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

    #[test]
    fn test_render_call_with_events_does_not_allocate() {
        use crate::render::test_events::{link, midi_event, parameter_event_at};
        use crate::render::AURenderEventType;

        let plugin = boxed(AuProcessor::<TestModel, TestPresets>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);

        // More parameter changes than the standard library's stable sort
        // sorts in its stack buffer, in reverse order, a few ramps, notes
        // and a program change that selects a factory preset.
        let gain = u64::from(gain_id());
        let mut events = Vec::new();
        for i in 0..512 {
            let time = (FRAMES - 1 - i % FRAMES) as i64;
            events.push(parameter_event_at(AURenderEventType::Parameter as u8, time, gain, -6.0, 0));
        }
        for i in 0..4 {
            let time = (i * 8) as i64;
            events.push(parameter_event_at(AURenderEventType::ParameterRamp as u8, time, gain, -3.0, 16));
        }
        events.push(midi_event(4, [0x90, 60, 100]));
        events.push(midi_event(2, [0xC0, 1, 0]));
        events.push(midi_event(40, [0x80, 60, 0]));
        let list = link(&mut events);

        let render_with_events = |input: &mut StereoList, output: &mut StereoList| {
            let timestamp = timestamp();
            let mut action_flags = 0u32;
            let status = block.process(
                &mut action_flags,
                &timestamp,
                FRAMES as u32,
                0,
                output.as_list(),
                list,
                std::ptr::null(),
                input.as_list().cast_const(),
                None,
                None,
            );
            assert_eq!(status, 0);
        };

        render_with_events(&mut input, &mut output);
        let allocations = allocations::count(|| {
            for _ in 0..8 {
                render_with_events(&mut input, &mut output);
            }
        });
        assert_eq!(allocations, 0);
    }

    thread_local! {
        /// What the `RecordingProcessor` saw on this thread: per process()
        /// call its length and the offsets of the MIDI events it got.
        static RECORDED: std::cell::RefCell<Vec<(usize, Vec<u32>)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }

    /// A plugin that records the MIDI of each process() call and passes it on.
    #[derive(Default)]
    struct RecordingModel {
        fields: TestFields,
    }

    #[derive(Default)]
    struct RecordingProcessor {
        midi_offsets: Vec<u32>,
    }

    beamer_core::__forward_model!(RecordingModel, fields);

    impl Model for RecordingModel {
        type Setup = ();
        type Processor = RecordingProcessor;

        fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
            RecordingProcessor::default()
        }
    }

    impl Processor for RecordingProcessor {
        fn process(
            &mut self,
            buffer: &mut Buffer,
            _aux: &mut AuxiliaryBuffers,
            _context: &ProcessContext,
        ) {
            let offsets = std::mem::take(&mut self.midi_offsets);
            RECORDED.with(|recorded| recorded.borrow_mut().push((buffer.num_samples(), offsets)));
        }

        fn process_midi(&mut self, input: &[MidiEvent], output: &mut beamer_core::MidiBuffer) {
            for event in input {
                self.midi_offsets.push(event.sample_offset);
                output.push(event.clone());
            }
        }
    }

    unsafe extern "C" fn record_midi_output(
        context: *mut std::ffi::c_void,
        sample_offset: u32,
        _bytes: *const u8,
        _length: u32,
    ) {
        // SAFETY: The test passes a pointer to its vector, alive for the call.
        let received = unsafe { &mut *context.cast::<Vec<u32>>() };
        received.push(sample_offset);
    }

    #[test]
    fn test_midi_reaches_each_sub_block_relative_to_it() {
        use crate::render::test_events::{link, midi_event, parameter_event_at};
        use crate::render::{AURenderEventType, MidiOutputSink};

        let plugin = boxed(AuProcessor::<RecordingModel>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);

        // A parameter change at sample 32 splits the buffer in two halves.
        // One note falls in each.
        let gain = u64::from(gain_id());
        let mut events = vec![
            midi_event(10, [0x90, 60, 100]),
            parameter_event_at(AURenderEventType::Parameter as u8, 32, gain, -6.0, 0),
            midi_event(40, [0x90, 64, 100]),
        ];
        let list = link(&mut events);

        let mut sent: Vec<u32> = Vec::new();
        let context: *mut Vec<u32> = &mut sent;
        // SAFETY: The sink is only used during the render call below, while
        // `sent` is alive.
        let sink = unsafe { MidiOutputSink::new(record_midi_output, context.cast()) };

        RECORDED.with(|recorded| recorded.borrow_mut().clear());
        let timestamp = timestamp();
        let mut action_flags = 0u32;
        let status = block.process(
            &mut action_flags,
            &timestamp,
            FRAMES as u32,
            0,
            output.as_list(),
            list,
            std::ptr::null(),
            input.as_list().cast_const(),
            None,
            Some(sink),
        );
        assert_eq!(status, 0);

        // Each process() call got the note of its half, relative to its start.
        let recorded = RECORDED.with(|recorded| recorded.borrow().clone());
        assert_eq!(recorded, [(32, vec![10]), (32, vec![8])]);

        // The host gets the output at the offsets of the whole buffer.
        assert_eq!(sent, [10, 40]);
    }

    /// Render a buffer of `frames` samples without input, with a gain change
    /// and a note at sample 0. Checks that nothing was rendered and the gain
    /// changed, and returns the offsets of the MIDI the host got back.
    fn render_events_without_audio(output: &mut StereoList, frames: u32) -> Vec<u32> {
        use crate::render::test_events::{link, midi_event, parameter_event_at};
        use crate::render::{AURenderEventType, MidiOutputSink};

        let plugin = boxed(AuProcessor::<RecordingModel>::new());
        let block = render_block_for(&plugin, ProcessMode::Realtime);
        let model = plugin.lock().unwrap().shared_model();
        let gain_before = model.parameter_store().get_normalized(gain_id());

        let gain = u64::from(gain_id());
        let mut events = vec![
            parameter_event_at(AURenderEventType::Parameter as u8, 0, gain, -6.0, 0),
            midi_event(0, [0x90, 60, 100]),
        ];
        let list = link(&mut events);

        let mut sent: Vec<u32> = Vec::new();
        let context: *mut Vec<u32> = &mut sent;
        // SAFETY: The sink is only used during the render call below, while
        // `sent` is alive.
        let sink = unsafe { MidiOutputSink::new(record_midi_output, context.cast()) };

        RECORDED.with(|recorded| recorded.borrow_mut().clear());
        let timestamp = timestamp();
        let mut action_flags = 0u32;
        let status = block.process(
            &mut action_flags,
            &timestamp,
            frames,
            0,
            output.as_list(),
            list,
            std::ptr::null(),
            std::ptr::null(),
            None,
            Some(sink),
        );
        assert_eq!(status, 0);

        // Nothing was rendered.
        assert!(RECORDED.with(|recorded| recorded.borrow().is_empty()));

        assert_ne!(model.parameter_store().get_normalized(gain_id()), gain_before);
        sent
    }

    #[test]
    fn test_call_without_samples_applies_its_events() {
        let mut output = StereoList::filled(0.0);
        assert_eq!(render_events_without_audio(&mut output, 0), [0]);
    }

    #[test]
    fn test_call_without_channels_applies_its_events() {
        // Null output buffers and no input: an effect has no channels to
        // write to.
        let mut output = StereoList::filled(0.0);
        output.buffers[0].data = std::ptr::null_mut();
        output.buffers[1].data = std::ptr::null_mut();
        assert_eq!(render_events_without_audio(&mut output, FRAMES as u32), [0]);
    }

    #[test]
    fn test_f64_render_converts_for_an_f32_processor_without_allocating() {
        let plugin = boxed(AuProcessor::<DoublingModel>::new());
        let bus_config = stereo_bus_config();
        plugin
            .lock()
            .unwrap()
            .allocate_render_resources(44100.0, FRAMES as u32, &bus_config, ProcessMode::Realtime)
            .expect("prepare");
        let model = plugin.lock().unwrap().shared_model();
        let storage = ProcessBufferStorage::<f64>::allocate_from_config(&bus_config, FRAMES);
        let block = create_render_block_f64(
            Arc::clone(&plugin),
            model,
            Arc::new(SharedState::default()),
            storage,
            FRAMES as u32,
            44100.0,
        );

        let mut input = StereoList64::filled(0.25);
        input.samples[1][FRAMES - 1] = -0.5;
        let mut output = StereoList64::filled(0.0);
        let render = |input: &mut StereoList64, output: &mut StereoList64| {
            let timestamp = timestamp();
            let mut action_flags = 0u32;
            let status = block.process(
                &mut action_flags,
                &timestamp,
                FRAMES as u32,
                0,
                output.as_list(),
                std::ptr::null(),
                std::ptr::null(),
                input.as_list().cast_const(),
                None,
                None,
            );
            assert_eq!(status, 0);
        };

        render(&mut input, &mut output);
        assert_eq!(output.samples[0][0], 0.5);
        assert_eq!(output.samples[1][FRAMES - 1], -1.0);
        // The render call marks the time the plugin processes.
        assert_eq!(SAW_PROCESS_SCOPE.with(|saw| saw.get()), Some(cfg!(debug_assertions)));

        let allocations = allocations::count(|| {
            for _ in 0..8 {
                render(&mut input, &mut output);
            }
        });
        assert_eq!(allocations, 0);
    }

    #[test]
    fn test_f64_conversion_starts_from_silence_without_main_input() {
        let mut plugin = AuProcessor::<DoublingModel>::new();
        plugin
            .allocate_render_resources(44100.0, FRAMES as u32, &stereo_bus_config(), ProcessMode::Realtime)
            .expect("prepare");
        let context = ProcessContext::with_empty_transport(44100.0, FRAMES);
        let input = [0.25f64; FRAMES];
        let mut left = [0.0f64; FRAMES];
        let mut right = [0.0f64; FRAMES];

        let mut buffer = Buffer::new([&input[..], &input[..]], [&mut left[..], &mut right[..]], FRAMES);
        plugin
            .process_block_f64(&mut buffer, &mut AuxiliaryBuffers::empty(), &context)
            .expect("process");
        assert_eq!(left, [0.5; FRAMES]);

        // Without main input the render call silences the outputs first, and
        // the processor gets no input channels and these silent outputs.
        left.fill(0.0);
        right.fill(0.0);
        let mut buffer = Buffer::new(std::iter::empty(), [&mut left[..], &mut right[..]], FRAMES);
        plugin
            .process_block_f64(&mut buffer, &mut AuxiliaryBuffers::empty(), &context)
            .expect("process");
        assert_eq!(left, [0.0; FRAMES]);
        assert_eq!(right, [0.0; FRAMES]);
    }

    /// A plugin with two sidechains whose processor plays the second one
    /// when only that one is connected.
    #[derive(Default)]
    struct SecondSidechainModel {
        fields: TestFields,
    }

    struct SecondSidechainProcessor;

    beamer_core::__forward_model!(SecondSidechainModel, fields);

    impl Model for SecondSidechainModel {
        type Setup = ();
        type Processor = SecondSidechainProcessor;

        fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
            SecondSidechainProcessor
        }

        fn input_bus_count(&self) -> usize {
            3
        }

        fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
            match index {
                0 => Some(BusInfo::mono("Input")),
                1 => Some(BusInfo::aux("First", 1)),
                2 => Some(BusInfo::aux("Second", 1)),
                _ => None,
            }
        }
    }

    impl Processor for SecondSidechainProcessor {
        fn process(
            &mut self,
            buffer: &mut Buffer,
            aux: &mut AuxiliaryBuffers,
            _context: &ProcessContext,
        ) {
            let output = buffer.output(0);
            match (aux.input(0), aux.input(1)) {
                (None, Some(second)) => output.copy_from_slice(second.input(0)),
                _ => output.fill(-1.0),
            }
        }
    }

    #[test]
    fn test_f64_conversion_keeps_a_sidechain_after_an_unconnected_one() {
        let mut plugin = AuProcessor::<SecondSidechainModel>::new();
        let bus_config = CachedBusConfig::new(
            vec![
                CachedBusInfo::new(1, BusType::Main),
                CachedBusInfo::new(1, BusType::Aux),
                CachedBusInfo::new(1, BusType::Aux),
            ],
            vec![CachedBusInfo::new(1, BusType::Main)],
        );
        plugin
            .allocate_render_resources(44100.0, FRAMES as u32, &bus_config, ProcessMode::Realtime)
            .expect("prepare");

        let main_input = [0.0f64; FRAMES];
        let second = [0.75f64; FRAMES];
        let mut main_output = [0.0f64; FRAMES];
        let mut buffer = Buffer::new([&main_input[..]], [&mut main_output[..]], FRAMES);
        // The host connected the second sidechain only.
        let mut aux = AuxiliaryBuffers::new(
            [Vec::new(), vec![&second[..]]],
            std::iter::empty::<[&mut [f64]; 0]>(),
            FRAMES,
        );
        let context = ProcessContext::with_empty_transport(44100.0, FRAMES);
        plugin
            .process_block_f64(&mut buffer, &mut aux, &context)
            .expect("process");

        assert_eq!(main_output, [0.75; FRAMES]);
    }

    #[test]
    fn test_preparing_again_delivers_the_process_mode() {
        let plugin = boxed(AuProcessor::<ModeModel>::new());
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
        let plugin = boxed(AuProcessor::<TestModel, TestPresets>::new());
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
        beamer_au_is_prepared,
        beamer_au_copy_state, beamer_au_free_state, beamer_au_get_tail_samples,
        beamer_au_render, beamer_au_reset, beamer_au_set_parameter_value, beamer_au_set_state,
        BeamerAuBusConfig, BeamerAuBusInfo, BeamerAuBusType, BeamerAuGroupInfo,
        BeamerAuPresetInfo, BeamerAuSampleFormat, BeamerInstanceHandle,
    };
    use crate::error::os_status;
    use beamer_core::{MAX_BUSES, MAX_CHANNELS};
    use std::ffi::CStr;

    /// An instance pointer that a test hands to a second thread.
    struct Instance(*mut BeamerInstanceHandle);
    // SAFETY: The bridge functions are made to be called from any thread.
    unsafe impl Send for Instance {}

    fn test_handle() -> Box<BeamerInstanceHandle> {
        let plugin = AuProcessor::<TestModel, TestPresets>::new();
        Box::new(BeamerInstanceHandle::new(Box::new(plugin)))
    }

    fn gain_id() -> u32 {
        fnv1a_hash("gain")
    }

    /// Allocate render resources for a stereo effect through the C-ABI.
    fn allocate(instance: *mut BeamerInstanceHandle, rendering_offline: bool) -> i32 {
        allocate_with(instance, 2, FRAMES as u32, rendering_offline)
    }

    /// Allocate render resources for an effect with one input and one output
    /// bus of `channels` channels.
    fn allocate_with(
        instance: *mut BeamerInstanceHandle,
        channels: u32,
        max_frames: u32,
        rendering_offline: bool,
    ) -> i32 {
        let bus = BeamerAuBusInfo {
            channel_count: channels,
            bus_type: BeamerAuBusType::Main,
        };
        let bus_config = BeamerAuBusConfig {
            input_bus_count: 1,
            output_bus_count: 1,
            input_buses: [bus; MAX_BUSES],
            output_buses: [bus; MAX_BUSES],
        };
        beamer_au_allocate_render_resources(
            instance,
            44100.0,
            max_frames,
            BeamerAuSampleFormat::Float32,
            &bus_config,
            rendering_offline,
        )
    }

    #[test]
    fn test_failed_allocation_leaves_the_instance_unprepared() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        assert_eq!(allocate(instance, false), 0);
        assert!(beamer_au_is_prepared(instance));
        assert_eq!(handle.has_render_setup(), (true, true));

        // A configuration the bus validation refuses.
        let status = allocate_with(instance, MAX_CHANNELS as u32 + 1, FRAMES as u32, false);
        assert_eq!(status, os_status::K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED);

        // Neither the old processor nor the old bus layout are left over.
        assert!(!beamer_au_is_prepared(instance));
        assert_eq!(handle.has_render_setup(), (false, false));
        assert_eq!(beamer_au_get_latency_samples(instance), 0);

        // An invalid buffer size is refused the same way.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(
            allocate_with(instance, 2, 0, false),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE
        );
        assert!(!beamer_au_is_prepared(instance));

        // The next valid allocation prepares it again.
        assert_eq!(allocate(instance, false), 0);
        assert!(beamer_au_is_prepared(instance));
        assert_eq!(handle.has_render_setup(), (true, true));
    }

    #[test]
    fn test_allocation_after_a_panic_in_the_plugin() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);

        // A panic while the plugin mutex was held fails the next allocation
        // and leaves the instance unprepared.
        handle.poison_plugin_lock();
        assert_eq!(
            allocate(instance, false),
            os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT
        );
        assert!(!beamer_au_is_prepared(instance));
        assert_eq!(handle.has_render_setup(), (false, false));

        // The allocation after that works again.
        assert_eq!(allocate(instance, false), 0);
        assert!(beamer_au_is_prepared(instance));
    }

    #[test]
    fn test_parameter_values_stay_through_preparing() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        beamer_au_set_parameter_value(instance, gain_id(), 0.25);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);

        // Preparing creates a processor that shares the model.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
        beamer_au_set_parameter_value(instance, gain_id(), 0.75);

        // Preparing again for offline rendering replaces the processor.
        assert_eq!(allocate(instance, true), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.75);

        // Deallocating drops the processor.
        beamer_au_deallocate_render_resources(instance);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.75);
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
        render_frames(instance, FRAMES as u32)
    }

    /// Ask for `frames` frames of the main bus through the C-ABI. The
    /// buffers hold `FRAMES` frames.
    fn render_frames(instance: *mut BeamerInstanceHandle, frames: u32) -> i32 {
        let timestamp = timestamp();
        let mut action_flags = 0u32;
        let mut input = StereoList::filled(0.5);
        let mut output = StereoList::filled(0.0);
        beamer_au_render(
            instance,
            &mut action_flags,
            &timestamp,
            frames,
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
        let mut size = 0u32;
        let buffer = beamer_au_copy_state(instance, &mut size);
        assert!(!buffer.is_null());
        // SAFETY: The bridge returned a buffer of this size.
        let state = unsafe { std::slice::from_raw_parts(buffer, size as usize) }.to_vec();
        beamer_au_free_state(buffer, size);
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
            let mut size = 0u32;
            let buffer = beamer_au_copy_state(saver.0, &mut size);
            beamer_au_free_state(buffer, size);
            size
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

        // The processor shares it: it is saved to the same bytes when prepared.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(saved_state(instance), quiet);

        // And when unprepared again.
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
    fn test_state_of_any_size_is_saved_and_loaded_whole() {
        // An instance whose state holds some hundred kilobytes.
        let plugin = AuProcessor::<TestModel, TestPresets>::new();
        let shared = plugin.shared_model();
        let model = shared
            .as_any()
            .downcast_ref::<TestModel>()
            .expect("the model of the test plugin");
        let blob: Vec<u8> = (0..300_000u32).map(|index| (index % 251) as u8).collect();
        model.fields.blob.set(blob.clone());
        let mut handle = Box::new(BeamerInstanceHandle::new(Box::new(plugin)));
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);

        // The bridge hands out exactly what the model saves.
        let state = saved_state(instance);
        assert!(state.len() > blob.len());
        assert_eq!(state, beamer_core::Saved::save_state(model));

        // Another instance takes all of it.
        let mut other = test_handle();
        let other_instance: *mut BeamerInstanceHandle = &mut *other;
        assert_eq!(load_state(other_instance, &state), 0);
        assert_eq!(beamer_au_get_parameter_value(other_instance, gain_id()), 0.25);
        assert_eq!(saved_state(other_instance), state);
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
    fn test_preparing_again_drops_the_processor_before_it_creates_one() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        beamer_au_set_parameter_value(instance, gain_id(), 0.25);

        assert_eq!(allocate(instance, false), 0);
        assert_eq!(LIVE_PROCESSORS_AT_PREPARE.with(Cell::get), 0);
        assert_eq!(LIVE_PROCESSORS.with(Cell::get), 1);

        // A new configuration: the buffers of the old processor are free
        // before the new ones are allocated.
        assert_eq!(allocate(instance, true), 0);
        assert_eq!(LIVE_PROCESSORS_AT_PREPARE.with(Cell::get), 0);
        assert_eq!(LIVE_PROCESSORS.with(Cell::get), 1);

        // The new processor shares the model: its latency follows the gain.
        assert_eq!(render_instance(instance), 0);
        assert_eq!(beamer_au_get_latency_samples(instance), latency_for(0.25));

        beamer_au_deallocate_render_resources(instance);
        assert_eq!(LIVE_PROCESSORS.with(Cell::get), 0);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
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
        // With the plugin held elsewhere nothing is unprepared. The model
        // does not depend on that.
        shared.with_plugin_locked(|| beamer_au_deallocate_render_resources(instance));

        let locks = plugin_lock_count(instance);
        assert_eq!(beamer_au_get_parameter_value(instance, gain_id()), 0.25);
        assert!(!saved_state(instance).is_empty());
        assert_eq!(plugin_lock_count(instance), locks);
        assert_eq!(beamer_au_get_tail_samples(instance), TEST_TAIL_SAMPLES);
    }

    #[test]
    fn test_render_refuses_more_frames_than_were_negotiated() {
        use crate::error::os_status::K_AUDIO_UNIT_ERR_TOO_MANY_FRAMES_TO_PROCESS;

        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(render_frames(instance, FRAMES as u32), 0);
        assert_eq!(
            render_frames(instance, FRAMES as u32 + 1),
            K_AUDIO_UNIT_ERR_TOO_MANY_FRAMES_TO_PROCESS
        );

        // A smaller maximum of a new allocation applies at once.
        assert_eq!(allocate_with(instance, 2, FRAMES as u32 / 2, false), 0);
        assert_eq!(render_frames(instance, FRAMES as u32 / 2), 0);
        assert_eq!(
            render_frames(instance, FRAMES as u32),
            K_AUDIO_UNIT_ERR_TOO_MANY_FRAMES_TO_PROCESS
        );
    }

    #[test]
    fn test_channel_count_follows_the_negotiated_bus_configuration() {
        let mut handle = test_handle();
        let instance: *mut BeamerInstanceHandle = &mut *handle;
        let channel_counts = || {
            [
                beamer_au_get_input_bus_channel_count(instance, 0),
                beamer_au_get_output_bus_channel_count(instance, 0),
            ]
        };

        // Declared: stereo.
        assert_eq!(channel_counts(), [2, 2]);

        // The host negotiates mono.
        assert_eq!(allocate_with(instance, 1, FRAMES as u32, false), 0);
        assert_eq!(channel_counts(), [1, 1]);

        // Without render resources the declared buses count again.
        beamer_au_deallocate_render_resources(instance);
        assert_eq!(channel_counts(), [2, 2]);
    }

    #[test]
    fn test_dropping_the_instance_frees_the_parameters() {
        let plugin = AuProcessor::<TestModel, TestPresets>::new();
        let model = Arc::downgrade(&plugin.shared_model());
        let mut handle = Box::new(BeamerInstanceHandle::new(Box::new(plugin)));
        let instance: *mut BeamerInstanceHandle = &mut *handle;

        // The handle, the processor and the render block each hold it.
        assert_eq!(allocate(instance, false), 0);
        assert_eq!(render_instance(instance), 0);
        assert!(model.upgrade().is_some());

        drop(handle);
        assert!(model.upgrade().is_none());
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
                assert_eq!(beamer_au_get_input_bus_channel_count(asker.0, 0), 2);
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
