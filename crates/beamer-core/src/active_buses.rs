//! The buses a plugin is prepared with and how a host's request becomes
//! that combination.
//!
//! A host may switch buses on and off. The format wrappers record its
//! request in a [`BusNegotiation`], which asks the plugin
//! ([`Model::supports_buses`]) and keeps the combination it accepted. The
//! plugin is prepared with the accepted [`ActiveBuses`]; where the host's
//! request and the accepted combination differ, the wrapper bridges the
//! difference per bus as its [`BusRouting`] says.

use crate::channel_layouts::{ChannelSet, Layout};
use crate::plugin::{self, HostSetup, Model, PluginSetup};
use crate::types::MAX_BUSES;

// =============================================================================
// Active Buses
// =============================================================================

/// The buses of an instance as it is prepared: which are on and the
/// channel set of each.
///
/// Bus 0 of each direction is the main bus. It is always on in the
/// combination a plugin is prepared with. A host request may have it off;
/// the wrapper then bridges it.
///
/// As a setup type, it tells [`Model::prepare()`] which aux buses are on.
/// A bus that is off there is `None` in
/// [`AuxiliaryBuffers`](crate::AuxiliaryBuffers) for the life of the
/// processor. A bus that is on can still be `None` for a single call: an
/// Audio Unit input the host has no source for right now (a sidechain
/// without a connection) has no samples.
///
/// # Example
///
/// ```ignore
/// impl Model for CompressorModel {
///     type Setup = (SampleRate, ActiveBuses);
///     fn prepare(self: Arc<Self>, (sr, buses): (SampleRate, ActiveBuses)) -> CompressorProcessor {
///         CompressorProcessor {
///             sample_rate: sr.0,
///             // The sidechain is input bus 1.
///             has_sidechain: buses.is_input_on(1),
///         }
///     }
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ActiveBuses {
    // Channel set of each input bus that is on, `None` past the count.
    inputs: [Option<ChannelSet>; MAX_BUSES],
    input_bus_count: usize,
    // Channel set of each output bus that is on, `None` past the count.
    outputs: [Option<ChannelSet>; MAX_BUSES],
    output_bus_count: usize,
}

impl ActiveBuses {
    /// No buses on, `input_bus_count` and `output_bus_count` declared
    /// (each at most [`MAX_BUSES`]). A wrapper switches buses on with
    /// [`set_input`](Self::set_input) and [`set_output`](Self::set_output).
    pub fn all_off(input_bus_count: usize, output_bus_count: usize) -> Self {
        Self {
            inputs: [None; MAX_BUSES],
            input_bus_count: input_bus_count.min(MAX_BUSES),
            outputs: [None; MAX_BUSES],
            output_bus_count: output_bus_count.min(MAX_BUSES),
        }
    }

    /// Every bus of `layout` on, with the set it starts with
    /// ([`Layout::first_input_set`]).
    ///
    /// Buses past [`MAX_BUSES`] in a direction are left out. The wrappers
    /// refuse such a model when it is prepared.
    pub fn all_on(layout: &Layout) -> Self {
        let mut buses = Self::all_off(layout.inputs.len(), layout.outputs.len());
        for bus in 0..buses.input_bus_count {
            let _ = buses.set_input(bus, layout.first_input_set(bus));
        }
        for bus in 0..buses.output_bus_count {
            let _ = buses.set_output(bus, layout.first_output_set(bus));
        }
        buses
    }

    /// The buses of the model's default layout (the first of its
    /// [`LAYOUTS`](Model::LAYOUTS)) with the default state of each
    /// ([`BusInfo::is_default_active`](crate::BusInfo::is_default_active),
    /// main buses always on).
    pub fn default_for<M: Model>(model: &M) -> Self {
        let mut buses = Self::all_on(Layout::default_of(M::LAYOUTS));
        for bus in 1..buses.input_bus_count {
            if !plugin::input_bus_info(model, bus).is_some_and(|info| info.is_default_active) {
                buses.inputs[bus] = None;
            }
        }
        for bus in 1..buses.output_bus_count {
            if !plugin::output_bus_info(model, bus).is_some_and(|info| info.is_default_active) {
                buses.outputs[bus] = None;
            }
        }
        buses
    }

    /// Channel set of input bus `bus` when it is on.
    pub fn input_set(&self, bus: usize) -> Option<ChannelSet> {
        self.inputs.get(bus).copied().flatten()
    }

    /// Channel set of output bus `bus` when it is on.
    pub fn output_set(&self, bus: usize) -> Option<ChannelSet> {
        self.outputs.get(bus).copied().flatten()
    }

    /// Channel count of input bus `bus` when it is on.
    pub fn input(&self, bus: usize) -> Option<u32> {
        self.input_set(bus).map(|set| set.channel_count())
    }

    /// Channel count of output bus `bus` when it is on.
    pub fn output(&self, bus: usize) -> Option<u32> {
        self.output_set(bus).map(|set| set.channel_count())
    }

    /// Whether input bus `bus` is on.
    pub fn is_input_on(&self, bus: usize) -> bool {
        self.input(bus).is_some()
    }

    /// Whether output bus `bus` is on.
    pub fn is_output_on(&self, bus: usize) -> bool {
        self.output(bus).is_some()
    }

    /// Number of declared input buses, on or off.
    pub fn input_bus_count(&self) -> usize {
        self.input_bus_count
    }

    /// Number of declared output buses, on or off.
    pub fn output_bus_count(&self) -> usize {
        self.output_bus_count
    }

    /// Switches input bus `bus` on with the channel set `set`, or off with
    /// `None` (or a set without channels).
    ///
    /// Returns false, changing nothing, when the bus is not declared.
    #[must_use]
    pub fn set_input(&mut self, bus: usize, set: Option<ChannelSet>) -> bool {
        if bus >= self.input_bus_count {
            return false;
        }
        self.inputs[bus] = set.filter(|set| set.channel_count() > 0);
        true
    }

    /// Switches output bus `bus` on with the channel set `set`, or off with
    /// `None` (or a set without channels).
    ///
    /// Returns false, changing nothing, when the bus is not declared.
    #[must_use]
    pub fn set_output(&mut self, bus: usize, set: Option<ChannelSet>) -> bool {
        if bus >= self.output_bus_count {
            return false;
        }
        self.outputs[bus] = set.filter(|set| set.channel_count() > 0);
        true
    }

    /// This combination with the main buses on: as requested, or with the
    /// set of `fallback` where the request has one off.
    fn with_main_buses_on(mut self, fallback: &ActiveBuses) -> Self {
        if self.input_bus_count > 0 && self.inputs[0].is_none() {
            self.inputs[0] = fallback.inputs[0];
        }
        if self.output_bus_count > 0 && self.outputs[0].is_none() {
            self.outputs[0] = fallback.outputs[0];
        }
        self
    }
}

impl PluginSetup for ActiveBuses {
    fn extract(host: &HostSetup) -> Self {
        host.buses
    }
}

// =============================================================================
// Negotiation
// =============================================================================

/// How a bus is connected between the host and the plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusRouting {
    /// On for the host and the plugin: the host's buffers reach the plugin.
    Connected,
    /// Off for the host and the plugin: the plugin sees `None`.
    Off,
    /// Off for the host but on for the plugin: an input reads silence, an
    /// output renders into memory of the wrapper's own that is dropped.
    PluginOnly,
    /// On for the host but off for the plugin: an input is ignored, an
    /// output is filled with silence.
    HostOnly,
}

/// The buses a host asked for and the buses the plugin accepted, kept by
/// a format wrapper per instance.
///
/// The wrapper records each host request with
/// [`requested_mut`](Self::requested_mut) and calls
/// [`negotiate`](Self::negotiate) before it prepares the plugin, which it
/// does with [`accepted`](Self::accepted). Until a request is accepted, the
/// plugin keeps the last combination it accepted, at first the declared
/// default ([`ActiveBuses::default_for`]).
#[derive(Clone, Copy, Debug)]
pub struct BusNegotiation {
    requested: ActiveBuses,
    accepted: ActiveBuses,
    // The last combination the plugin refused, so it is not asked again.
    refused: Option<ActiveBuses>,
}

impl BusNegotiation {
    /// Starts with the declared default as both request and accepted
    /// combination.
    pub fn new<M: Model>(model: &M) -> Self {
        Self::starting_with(ActiveBuses::default_for(model))
    }

    /// Starts with `default` as both request and accepted combination, for
    /// a wrapper that declares the buses on the model's behalf.
    pub fn starting_with(default: ActiveBuses) -> Self {
        Self {
            requested: default,
            accepted: default,
            refused: None,
        }
    }

    /// The buses the host asked for.
    pub fn requested(&self) -> &ActiveBuses {
        &self.requested
    }

    /// The buses the host asked for, to record a request.
    pub fn requested_mut(&mut self) -> &mut ActiveBuses {
        &mut self.requested
    }

    /// The buses the plugin accepted, to prepare it with.
    pub fn accepted(&self) -> &ActiveBuses {
        &self.accepted
    }

    /// Turns the host's request into the combination the plugin is
    /// prepared with.
    ///
    /// The main buses stay on. The plugin is asked with
    /// [`Model::supports_buses`] when the request differs from the
    /// combination it accepted last and from the one it refused last; when
    /// it refuses, it keeps the one it accepted. Call on a host thread.
    ///
    /// Returns whether the accepted combination changed, so the plugin
    /// needs to be prepared again.
    pub fn negotiate<M: Model>(&mut self, model: &M) -> bool {
        self.negotiate_with(|buses| model.supports_buses(buses))
    }

    /// [`negotiate`](Self::negotiate) with the plugin's answer from
    /// `supports_buses`, for a wrapper that holds the model behind a trait
    /// object.
    pub fn negotiate_with(&mut self, supports_buses: impl FnOnce(&ActiveBuses) -> bool) -> bool {
        let candidate = self.requested.with_main_buses_on(&self.accepted);
        if candidate == self.accepted || self.refused == Some(candidate) {
            return false;
        }
        if !supports_buses(&candidate) {
            self.refused = Some(candidate);
            return false;
        }
        self.accepted = candidate;
        self.refused = None;
        true
    }

    /// How input bus `bus` is connected. An undeclared bus is
    /// [`BusRouting::Off`].
    pub fn input_routing(&self, bus: usize) -> BusRouting {
        routing(self.requested.is_input_on(bus), self.accepted.is_input_on(bus))
    }

    /// How output bus `bus` is connected. An undeclared bus is
    /// [`BusRouting::Off`].
    pub fn output_routing(&self, bus: usize) -> BusRouting {
        routing(self.requested.is_output_on(bus), self.accepted.is_output_on(bus))
    }
}

/// How a bus is connected when the host has it on or off and the plugin
/// has it on or off.
fn routing(host_on: bool, plugin_on: bool) -> BusRouting {
    match (host_on, plugin_on) {
        (true, true) => BusRouting::Connected,
        (false, false) => BusRouting::Off,
        (false, true) => BusRouting::PluginOnly,
        (true, false) => BusRouting::HostOnly,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    use super::*;
    use crate::buffer::{AuxiliaryBuffers, Buffer};
    use crate::channel_layouts::BusLayout;
    use crate::parameter_store::NoParameters;
    use crate::plugin::{BusInfo, Processor};
    use crate::process_context::ProcessContext;

    /// A compressor-like model: stereo in and out, a stereo sidechain and
    /// a mono aux output. Refuses the aux output without the sidechain.
    #[derive(Default)]
    struct SidechainModel {
        fields: NoParameters,
        // How often `supports_buses` was called.
        questions: AtomicUsize,
    }

    struct SidechainProcessor;

    crate::__forward_model!(SidechainModel, fields);

    impl Model for SidechainModel {
        type Setup = ();
        type Processor = SidechainProcessor;

        fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
            SidechainProcessor
        }

        const LAYOUTS: &'static [Layout] = &[Layout {
            inputs: &[BusLayout::STEREO, BusLayout::STEREO],
            outputs: &[BusLayout::STEREO, BusLayout::MONO],
        }];

        fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
            match index {
                0 => Some(BusInfo::main("Input")),
                1 => Some(BusInfo::aux("Sidechain")),
                _ => None,
            }
        }

        fn output_bus_info(&self, index: usize) -> Option<BusInfo> {
            match index {
                0 => Some(BusInfo::main("Output")),
                1 => Some(BusInfo::aux("Detector")),
                _ => None,
            }
        }

        fn supports_buses(&self, buses: &ActiveBuses) -> bool {
            self.questions.fetch_add(1, Ordering::Relaxed);
            buses.is_input_on(1) || !buses.is_output_on(1)
        }
    }

    impl Processor for SidechainProcessor {
        fn process(&mut self, _: &mut Buffer, _: &mut AuxiliaryBuffers, _: &ProcessContext) {}
    }

    /// An instrument: no input, a mono main output.
    #[derive(Default)]
    struct InstrumentModel {
        fields: NoParameters,
    }

    crate::__forward_model!(InstrumentModel, fields);

    impl Model for InstrumentModel {
        type Setup = ();
        type Processor = SidechainProcessor;

        fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
            SidechainProcessor
        }

        const LAYOUTS: &'static [Layout] = &[Layout::INSTRUMENT_MONO];
    }

    #[test]
    fn test_default_has_main_buses_on_and_aux_buses_off() {
        let buses = ActiveBuses::default_for(&SidechainModel::default());

        assert_eq!(buses.input_bus_count(), 2);
        assert_eq!(buses.output_bus_count(), 2);
        assert_eq!(buses.input(0), Some(2));
        assert_eq!(buses.output(0), Some(2));
        assert!(!buses.is_input_on(1));
        assert!(!buses.is_output_on(1));
        // Past the declared buses.
        assert_eq!(buses.input(2), None);
        assert_eq!(buses.output(MAX_BUSES), None);
    }

    #[test]
    fn test_default_has_the_sets_of_the_first_layout() {
        /// Mono or stereo, with a mono return on by default and an aux
        /// output without info.
        #[derive(Default)]
        struct ReturnModel {
            fields: NoParameters,
        }

        crate::__forward_model!(ReturnModel, fields);

        impl Model for ReturnModel {
            type Setup = ();
            type Processor = SidechainProcessor;
            const LAYOUTS: &'static [Layout] = &[
                Layout {
                    inputs: &[BusLayout::MONO, BusLayout::MONO],
                    outputs: &[BusLayout::MONO, BusLayout::Any],
                },
                Layout {
                    inputs: &[BusLayout::STEREO, BusLayout::MONO],
                    outputs: &[BusLayout::STEREO, BusLayout::Any],
                },
            ];

            fn prepare(self: Arc<Self>, _: ()) -> Self::Processor {
                SidechainProcessor
            }

            fn input_bus_info(&self, index: usize) -> Option<BusInfo> {
                (index == 1).then_some(BusInfo { is_default_active: true, ..BusInfo::aux("Return") })
            }

            fn output_bus_info(&self, _: usize) -> Option<BusInfo> {
                None
            }
        }

        let model = ReturnModel::default();
        let buses = ActiveBuses::default_for(&model);
        assert_eq!(buses.input_set(0), Some(ChannelSet::Mono));
        assert_eq!(buses.input_set(1), Some(ChannelSet::Mono));
        assert_eq!(buses.output_set(0), Some(ChannelSet::Mono));
        // An aux bus without info is off by default.
        assert!(!buses.is_output_on(1));
        assert_eq!(plugin::output_bus_info(&model, 1), Some(BusInfo::aux("Aux 1")));
        assert_eq!(plugin::output_bus_info(&model, 2), None);
        assert_eq!(plugin::input_bus_info(&model, 0), Some(BusInfo::main("Input")));

        // Every bus on, a wildcard as stereo.
        let all_on = ActiveBuses::all_on(&ReturnModel::LAYOUTS[0]);
        assert_eq!(all_on.output_set(1), Some(ChannelSet::Stereo));
    }

    #[test]
    fn test_default_without_input_bus() {
        let buses = ActiveBuses::default_for(&InstrumentModel::default());

        assert_eq!(buses.input_bus_count(), 0);
        assert!(!buses.is_input_on(0));
        assert_eq!(buses.output(0), Some(1));
    }

    #[test]
    fn test_set_switches_declared_buses_only() {
        let mut buses = ActiveBuses::default_for(&SidechainModel::default());

        assert!(buses.set_input(1, Some(ChannelSet::Stereo)));
        assert_eq!(buses.input(1), Some(2));
        // A set without channels switches a bus off.
        assert!(buses.set_input(1, Some(ChannelSet::Discrete(0))));
        assert!(!buses.is_input_on(1));
        assert!(!buses.set_input(2, Some(ChannelSet::Stereo)));
        assert!(!buses.set_output(2, Some(ChannelSet::Stereo)));
        assert_eq!(buses, ActiveBuses::default_for(&SidechainModel::default()));
    }

    #[test]
    fn test_supported_request_is_accepted() {
        let model = SidechainModel::default();
        let mut negotiation = BusNegotiation::new(&model);
        assert!(negotiation.requested_mut().set_input(1, Some(ChannelSet::Stereo)));

        assert!(negotiation.negotiate(&model));
        assert_eq!(negotiation.accepted().input(1), Some(2));
        assert_eq!(negotiation.input_routing(1), BusRouting::Connected);
        // The same request again changes nothing.
        assert!(!negotiation.negotiate(&model));
    }

    #[test]
    fn test_refused_request_keeps_the_last_accepted_buses() {
        let model = SidechainModel::default();
        let mut negotiation = BusNegotiation::new(&model);
        // The detector output without the sidechain is refused.
        assert!(negotiation.requested_mut().set_output(1, Some(ChannelSet::Mono)));

        assert!(!negotiation.negotiate(&model));
        assert_eq!(negotiation.accepted(), &ActiveBuses::default_for(&model));
        assert_eq!(negotiation.output_routing(1), BusRouting::HostOnly);

        // With the sidechain on as well, both are accepted.
        assert!(negotiation.requested_mut().set_input(1, Some(ChannelSet::Stereo)));
        assert!(negotiation.negotiate(&model));
        assert_eq!(negotiation.output_routing(1), BusRouting::Connected);

        // Switching the sidechain off again is refused: the plugin keeps
        // it and reads silence.
        assert!(negotiation.requested_mut().set_input(1, None));
        assert!(!negotiation.negotiate(&model));
        assert_eq!(negotiation.accepted().input(1), Some(2));
        assert_eq!(negotiation.input_routing(1), BusRouting::PluginOnly);
        assert_eq!(negotiation.output_routing(1), BusRouting::Connected);
    }

    #[test]
    fn test_plugin_is_asked_once_per_combination() {
        let model = SidechainModel::default();
        let mut negotiation = BusNegotiation::new(&model);
        assert!(negotiation.requested_mut().set_output(1, Some(ChannelSet::Mono)));

        assert!(!negotiation.negotiate(&model));
        assert!(!negotiation.negotiate(&model));
        assert_eq!(model.questions.load(Ordering::Relaxed), 1);

        // A new request is asked about, also when it was refused before.
        assert!(negotiation.requested_mut().set_input(1, Some(ChannelSet::Stereo)));
        assert!(negotiation.negotiate(&model));
        assert!(negotiation.requested_mut().set_input(1, None));
        assert!(!negotiation.negotiate(&model));
        assert!(negotiation.requested_mut().set_output(1, None));
        assert!(negotiation.negotiate(&model));
        assert!(negotiation.requested_mut().set_output(1, Some(ChannelSet::Mono)));
        assert!(!negotiation.negotiate(&model));
        assert_eq!(model.questions.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn test_main_buses_stay_on() {
        let model = SidechainModel::default();
        let mut negotiation = BusNegotiation::new(&model);
        assert!(negotiation.requested_mut().set_input(0, None));
        assert!(negotiation.requested_mut().set_output(0, None));

        // Nothing changes for the plugin: it keeps its main buses.
        assert!(!negotiation.negotiate(&model));
        assert_eq!(negotiation.accepted().input(0), Some(2));
        assert_eq!(negotiation.accepted().output(0), Some(2));
        assert_eq!(negotiation.input_routing(0), BusRouting::PluginOnly);
        assert_eq!(negotiation.output_routing(0), BusRouting::PluginOnly);

        // With an aux bus on as well, the plugin is asked with its main
        // buses on.
        assert!(negotiation.requested_mut().set_input(1, Some(ChannelSet::Stereo)));
        assert!(negotiation.negotiate(&model));
        assert_eq!(negotiation.accepted().input(0), Some(2));
        assert_eq!(negotiation.accepted().input(1), Some(2));
        assert_eq!(negotiation.accepted().output(0), Some(2));
    }

    #[test]
    fn test_routing_of_buses_off_for_both_and_undeclared() {
        let model = SidechainModel::default();
        let negotiation = BusNegotiation::new(&model);

        assert_eq!(negotiation.input_routing(0), BusRouting::Connected);
        assert_eq!(negotiation.input_routing(1), BusRouting::Off);
        assert_eq!(negotiation.input_routing(2), BusRouting::Off);
        assert_eq!(negotiation.output_routing(MAX_BUSES), BusRouting::Off);
    }
}
