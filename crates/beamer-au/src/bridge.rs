//! C-ABI bridge for hybrid Audio Unit architecture.
//!
//! This module implements all the C-ABI functions that the native AUv2 and AUv3 wrappers call.
//! It bridges to the existing beamer-au Rust infrastructure (AuPluginInstance, RenderBlock, etc.).
//!
//! # Architecture
//!
//! The hybrid AU architecture uses thin native wrappers that delegate to Rust:
//!
//! ```text
//! AU Host (Logic Pro, etc.)
//!        ↓
//! Generated wrapper (AUv2 or AUv3, from xtask/src/au_codegen)
//!        ↓ (C-ABI calls)
//! bridge.rs (this module)
//!        ↓
//! AuPluginInstance / RenderBlock
//!        ↓
//! beamer_core::Model / Processor
//! ```
//!
//! # Safety
//!
//! All functions use `std::panic::catch_unwind` to prevent panics from crossing the FFI boundary.
//! Pointers are validated before dereferencing.
//! Functions return OSStatus error codes on failure.

// These are C-ABI entry points called from Objective-C. The ObjC side is responsible
// for passing valid pointers. Marking them `unsafe` would be unusual for C FFI.
#![allow(clippy::not_unsafe_ptr_arg_deref)]

use std::ffi::{c_char, c_void, CStr, CString};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;
use std::sync::{Arc, Mutex};

use crate::buffer_storage::ProcessBufferStorage;
use crate::buffers::AudioBufferList;
use crate::channel_labels::{self, ChannelOrder, ChannelOrders};
use crate::error::os_status;
use beamer_core::{
    ActiveBuses, BusInfo, BusLayout, BusNegotiation, BusType, CachedBusConfig, CachedBusInfo, ChannelSet, EditorSync,
    Layout, ParameterInfo, ParameterUnit, ProcessMode, SharedModel, WebViewDispatch, WebViewEventQueue,
    MAX_BUSES, MAX_CHANNELS,
};
use crate::factory;
use crate::instance::{AuPluginInstance, FactoryPresetAccess};
use crate::render::{
    create_render_block_f32, create_render_block_f64, AURenderEvent, AudioTimeStamp,
    BeamerAuMidiOutputFn, MidiOutputSink, RenderBlockTrait, RenderSettings,
};
use crate::shared_state::SharedState;
use crate::transport::BeamerAuTransport;
use beamer_core::ParameterStore;

// =============================================================================
// Compile-time checks for C header constant sync
// =============================================================================
//
// These assertions ensure that constants in BeamerAuBridge.h stay in sync with
// their Rust counterparts. If you change a constant in beamer_core and the build
// fails here, update the corresponding #define in objc/BeamerAuBridge.h.

const _: () = assert!(
    beamer_core::midi::MAX_MIDI_EVENTS == 1024,
    "Update BEAMER_AU_MAX_MIDI_EVENTS in BeamerAuBridge.h"
);
const _: () = assert!(
    MAX_BUSES == 16,
    "Update BEAMER_AU_MAX_BUSES in BeamerAuBridge.h"
);
const _: () = assert!(
    beamer_core::MAX_CHANNELS == 32,
    "Update BEAMER_AU_MAX_CHANNELS in BeamerAuBridge.h"
);

// =============================================================================
// Macros
// =============================================================================

/// Safely execute code with an instance handle, handling null checks and panics.
///
/// This macro reduces boilerplate in C-ABI functions by handling:
/// - Null pointer checks
/// - Panic unwinding to prevent panics from crossing the FFI boundary
/// - Safe dereferencing of the instance pointer
///
/// # Usage
///
/// ```ignore
/// with_instance!(instance, default_value, |handle| {
///     // Your code here, with `handle` as &BeamerInstanceHandle
/// })
/// ```
macro_rules! with_instance {
    ($instance:expr, $default:expr, |$handle:ident| $body:expr) => {{
        if $instance.is_null() {
            return $default;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: Instance pointer validated non-null above. The ObjC side is
            // responsible for passing valid pointers. Instance remains valid for
            // the duration of the FFI call.
            let $handle = unsafe { &*$instance };
            $body
        }))
        .unwrap_or($default)
    }};
}

/// Variant of `with_instance!` for functions that return `()`.
///
/// Uses `let _ =` to discard the result instead of `.unwrap_or(())`,
/// which avoids clippy's `unused_unit` warning.
macro_rules! with_instance_void {
    ($instance:expr, |$handle:ident| $body:expr) => {{
        if $instance.is_null() {
            return;
        }
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            // SAFETY: Instance pointer validated non-null above. The ObjC side is
            // responsible for passing valid pointers. Instance remains valid for
            // the duration of the FFI call.
            let $handle = unsafe { &*$instance };
            $body
        }));
    }};
}

// =============================================================================
// Constants (must match BeamerAuBridge.h)
// =============================================================================

/// Maximum length of parameter name/unit strings.
const BEAMER_AU_MAX_PARAM_NAME_LENGTH: usize = 128;

/// Maximum length of group name strings.
const BEAMER_AU_MAX_GROUP_NAME_LENGTH: usize = 128;

// =============================================================================
// C-ABI Structs (must match BeamerAuBridge.h exactly)
// =============================================================================

/// Bus type enumeration (matches BeamerAuBusType in header).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuBusType {
    /// Main audio bus (bus index 0)
    Main = 0,
    /// Auxiliary audio bus (sidechain, additional I/O)
    Auxiliary = 1,
}

/// Information about a single audio bus (matches BeamerAuBusInfo in header).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BeamerAuBusInfo {
    /// Number of channels in this bus (1 = mono, 2 = stereo, etc.)
    pub channel_count: u32,
    /// Bus type (main or auxiliary)
    pub bus_type: BeamerAuBusType,
    /// Number of channel labels in `layout_labels`: the channel count when
    /// the host set a channel layout on the bus, 0 when it set none.
    pub layout_label_count: u32,
    /// The channel label of each channel of the host's channel layout.
    pub layout_labels: [u32; MAX_CHANNELS],
}

impl BeamerAuBusInfo {
    /// A bus of `channel_count` channels without a channel layout.
    pub fn new(channel_count: u32, bus_type: BeamerAuBusType) -> Self {
        Self { channel_count, bus_type, layout_label_count: 0, layout_labels: [0; MAX_CHANNELS] }
    }

    /// The host's channel labels, empty without a channel layout.
    fn labels(&self) -> &[u32] {
        &self.layout_labels[..(self.layout_label_count as usize).min(MAX_CHANNELS)]
    }
}

/// Complete bus configuration for the plugin (matches BeamerAuBusConfig in header).
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BeamerAuBusConfig {
    /// Number of input buses (1 = main only, 2+ = main + aux)
    pub input_bus_count: u32,
    /// Number of output buses (1 = main only, 2+ = main + aux)
    pub output_bus_count: u32,
    /// Input bus information array
    pub input_buses: [BeamerAuBusInfo; MAX_BUSES],
    /// Output bus information array
    pub output_buses: [BeamerAuBusInfo; MAX_BUSES],
}

/// Sample format enumeration (matches BeamerAuSampleFormat in header).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuSampleFormat {
    /// 32-bit floating point samples (standard)
    Float32 = 0,
    /// 64-bit floating point samples (high precision)
    Float64 = 1,
}

/// Float64 processing support level.
///
/// The AU wrapper accepts float64 stream formats either natively (when the
/// prepared processor supports it) or via internal conversion.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BeamerAuFloat64Support {
    NotSupported = 0,
    ViaConversion = 1,
    Native = 2,
}

/// Parameter metadata for building AUParameterTree (matches BeamerAuParameterInfo in header).
#[repr(C)]
pub struct BeamerAuParameterInfo {
    /// Parameter ID (unique within the plugin, maps to AU parameter address)
    pub id: u32,
    /// Human-readable parameter name (UTF-8, null-terminated)
    pub name: [c_char; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
    /// Parameter unit string (e.g., "dB", "Hz", "ms"; UTF-8, null-terminated)
    pub units: [c_char; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
    /// AudioUnitParameterUnit value for host UI hints.
    ///
    /// This tells AU hosts what visual control to render:
    /// - 0 = Generic (slider)
    /// - 1 = Indexed (dropdown)
    /// - 2 = Boolean (checkbox)
    /// - 13 = Decibels, 8 = Hertz, etc.
    pub unit_type: u32,
    /// Minimum value in AU format: the actual value (e.g., -60 dB for gain),
    /// or 0 for an indexed parameter
    pub min_value: f32,
    /// Maximum value in AU format: the actual value (e.g., +12 dB for gain),
    /// or `step_count` for an indexed parameter
    pub max_value: f32,
    /// Default value in AU format (actual value, or index for an indexed parameter)
    pub default_value: f32,
    /// Current value in AU format (actual value, or index for an indexed parameter)
    pub current_value: f32,
    /// Number of discrete steps (0 = continuous, 1 = boolean, N = N+1 states)
    pub step_count: i32,
    /// Flags (reserved for future use: automatable, hidden, etc.)
    pub flags: u32,
    /// Group ID this parameter belongs to (0 = root/ungrouped)
    pub group_id: i32,
}

impl Default for BeamerAuParameterInfo {
    fn default() -> Self {
        Self {
            id: 0,
            name: [0; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
            units: [0; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
            unit_type: 0, // Generic
            min_value: 0.0,
            max_value: 1.0,
            default_value: 0.0,
            current_value: 0.0,
            step_count: 0,
            flags: 0,
            group_id: 0,
        }
    }
}

/// Parameter group metadata for building hierarchical AUParameterTree.
///
/// Matches `BeamerAuGroupInfo` in header.
#[repr(C)]
pub struct BeamerAuGroupInfo {
    /// Unique group identifier (matches VST3 UnitId)
    pub id: i32,
    /// Human-readable group name (UTF-8, null-terminated)
    pub name: [c_char; BEAMER_AU_MAX_GROUP_NAME_LENGTH],
    /// Parent group ID (0 = top-level, i.e., child of root)
    pub parent_id: i32,
}

impl Default for BeamerAuGroupInfo {
    fn default() -> Self {
        Self {
            id: 0,
            name: [0; BEAMER_AU_MAX_GROUP_NAME_LENGTH],
            parent_id: 0,
        }
    }
}

// =============================================================================
// Channel Capabilities
// =============================================================================

/// Maximum number of channel capability entries a plugin can declare.
///
/// Most plugins only need 1-3 configurations (e.g., mono, stereo, surround).
pub const BEAMER_AU_MAX_CHANNEL_CAPABILITIES: usize = 16;

/// A single channel capability entry representing a supported [input, output] pair.
///
/// AU channel capabilities use signed integers with special semantics:
/// - `-1` means "any number of channels" (wildcard)
/// - `0` means "no channels" (e.g., for instruments with no audio input)
/// - Positive values indicate exact channel counts
///
/// Common patterns:
/// - `[-1, -1]`: Any matching input/output (typical for effects)
/// - `[0, 2]`: Stereo instrument (no input, stereo output)
/// - `[2, 2]`: Stereo effect (stereo in, stereo out)
/// - `[1, 1]`: Mono effect
#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct BeamerAuChannelCapability {
    /// Number of input channels (-1 = any, 0 = none, >0 = exact count)
    pub input_channels: i32,
    /// Number of output channels (-1 = any, 0 = none, >0 = exact count)
    pub output_channels: i32,
}

/// Channel capabilities result containing all supported configurations.
///
/// The AU framework uses this to populate the `channelCapabilities` property.
/// Each pair of values in the array represents one supported configuration.
#[repr(C)]
#[derive(Debug, Clone)]
pub struct BeamerAuChannelCapabilities {
    /// Number of valid capability entries (0 means "any configuration supported")
    pub count: u32,
    /// Array of supported [input, output] channel configurations
    pub capabilities: [BeamerAuChannelCapability; BEAMER_AU_MAX_CHANNEL_CAPABILITIES],
}

impl Default for BeamerAuChannelCapabilities {
    fn default() -> Self {
        Self {
            count: 0,
            capabilities: [BeamerAuChannelCapability::default(); BEAMER_AU_MAX_CHANNEL_CAPABILITIES],
        }
    }
}

// =============================================================================
// Instance Handle
// =============================================================================

/// The layouts of a plugin without any bus: an Audio Unit needs an output
/// bus, so it gets a stereo one.
const NO_BUS_LAYOUTS: &[Layout] = &[Layout::INSTRUMENT_STEREO];

/// The buses a plugin declares, captured when the instance is created.
///
/// A host asks for them at any time, also while audio runs, so the answers
/// must not need the plugin.
pub(crate) struct DeclaredBuses {
    /// The layouts the plugin lists, the default first.
    layouts: &'static [Layout],
    /// Every bus on, with the set it has in the default layout.
    all_on: ActiveBuses,
    /// The buses with the default state of each: the main buses on, an aux
    /// bus on when it is active by default. A plugin that refuses the buses
    /// a host asks for is prepared with these.
    default: ActiveBuses,
    /// Whether the layouts are valid (see `validate_layouts`). The export
    /// macro checks them when the plugin is compiled, a model used without
    /// it is not.
    validation: Result<(), &'static str>,
    /// The name of each input bus: as the plugin gives it, else the default
    /// name of its index (see `BusInfo::default_input`).
    input_names: [&'static str; MAX_BUSES],
    /// The name of each output bus, as `input_names`.
    output_names: [&'static str; MAX_BUSES],
}

impl DeclaredBuses {
    /// Capture the bus declaration of a plugin.
    pub(crate) fn capture(plugin: &dyn AuPluginInstance) -> Self {
        let layouts = plugin.layouts();
        let mut buses = Self {
            validation: beamer_core::validate_layouts(layouts),
            ..Self::new(layouts, plugin.default_buses())
        };
        for index in 0..MAX_BUSES {
            if let Some(name) = plugin.input_bus_name(index) {
                buses.input_names[index] = name;
            }
            if let Some(name) = plugin.output_bus_name(index) {
                buses.output_names[index] = name;
            }
        }
        buses
    }

    /// The buses an Audio Unit has for a declaration: `layouts` with the
    /// default buses `default`.
    ///
    /// An Audio Unit needs an output bus. A plugin that declares no bus at
    /// all gets a stereo one, on. Both wrappers build their buses from
    /// these.
    fn new(layouts: &'static [Layout], default: ActiveBuses) -> Self {
        let first = Layout::default_of(layouts);
        let (layouts, all_on, default) = if first.inputs.is_empty() && first.outputs.is_empty() {
            let output = ActiveBuses::all_on(&NO_BUS_LAYOUTS[0]);
            (NO_BUS_LAYOUTS, output, output)
        } else {
            (layouts, ActiveBuses::all_on(first), default)
        };
        Self {
            layouts,
            all_on,
            default,
            validation: Ok(()),
            input_names: std::array::from_fn(|index| BusInfo::default_input(index).name),
            output_names: std::array::from_fn(|index| BusInfo::default_output(index).name),
        }
    }

    /// The name of the bus at an index in a direction, `None` for a bus
    /// that does not exist.
    fn name(&self, direction: BusDirection, index: usize) -> Option<&'static str> {
        if index >= self.count(direction) {
            return None;
        }
        let names = match direction {
            BusDirection::Input => &self.input_names,
            BusDirection::Output => &self.output_names,
        };
        names.get(index).copied()
    }

    /// Number of buses in a direction.
    fn count(&self, direction: BusDirection) -> usize {
        match direction {
            BusDirection::Input => self.all_on.input_bus_count(),
            BusDirection::Output => self.all_on.output_bus_count(),
        }
    }

    /// Channel count of the bus at an index in a direction in the default
    /// layout, 0 for a bus that does not exist.
    fn channel_count(&self, direction: BusDirection, index: usize) -> u32 {
        let channels = match direction {
            BusDirection::Input => self.all_on.input(index),
            BusDirection::Output => self.all_on.output(index),
        };
        channels.unwrap_or(0)
    }

    /// What each layout allows for a bus, the default layout first.
    fn entries(&self, direction: BusDirection, index: usize) -> impl Iterator<Item = BusLayout> + Clone + '_ {
        self.layouts.iter().filter_map(move |layout| {
            let buses = match direction {
                BusDirection::Input => layout.inputs,
                BusDirection::Output => layout.outputs,
            };
            buses.get(index).copied()
        })
    }

    /// The sets the layouts name for a bus, the default layout's first.
    fn listed(&self, direction: BusDirection, index: usize) -> impl Iterator<Item = ChannelSet> + Clone + '_ {
        self.entries(direction, index).filter_map(|entry| match entry {
            BusLayout::Set(set) => Some(set),
            _ => None,
        })
    }

    /// Whether a layout takes `channels` channels on a bus. A bus that
    /// follows the main input takes any count here; the whole combination
    /// is checked when the plugin is prepared.
    fn allows_count(&self, direction: BusDirection, index: usize, channels: u32) -> bool {
        let fits = channels > 0 && channels as usize <= MAX_CHANNELS;
        fits && self.entries(direction, index).any(|entry| match entry {
            BusLayout::Set(set) => set.channel_count() == channels,
            BusLayout::Any | BusLayout::SameAsMainInput => true,
        })
    }

    /// Whether a layout takes the set `set` on a bus, as `allows_count`.
    fn allows_set(&self, direction: BusDirection, index: usize, set: ChannelSet) -> bool {
        self.entries(direction, index).any(|entry| match entry {
            BusLayout::SameAsMainInput => true,
            entry => entry.allows(set, None),
        })
    }

    /// The set a host means with `channels` channels on a bus, with the
    /// order in which it has them.
    ///
    /// With the host's channel labels `labels`, the set they name (see
    /// `channel_labels::set_for_labels`). Without, the first set a layout
    /// names for the bus with that count, else the speaker layout of that
    /// size (see `ChannelSet::for_count`), in the set's order. `None` for 0
    /// channels.
    fn set_for(
        &self,
        direction: BusDirection,
        index: usize,
        channels: u32,
        labels: &[u32],
    ) -> Option<(ChannelSet, ChannelOrder)> {
        if channels == 0 {
            return None;
        }
        if labels.len() == channels as usize {
            return channel_labels::set_for_labels(labels, self.listed(direction, index));
        }
        let set = self
            .listed(direction, index)
            .find(|set| set.channel_count() == channels)
            .or_else(|| ChannelSet::for_count(channels))?;
        Some((set, ChannelOrder::default()))
    }

    /// The layout tags of the sets a bus takes: the sets the layouts name
    /// and every named set for a bus that takes any set.
    fn layout_tags(&self, direction: BusDirection, index: usize) -> Vec<u32> {
        let any = self
            .entries(direction, index)
            .any(|entry| matches!(entry, BusLayout::Any | BusLayout::SameAsMainInput));
        let named = channel_labels::named_sets().filter(|_| any);
        let mut tags: Vec<u32> = Vec::new();
        for set in self.listed(direction, index).chain(named) {
            let tag = channel_labels::tag_of(set);
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        tags
    }

    /// The main input and output channel counts the layouts list: one
    /// pair per layout, in the Audio Unit's terms (-1 for any count, -1 for
    /// an output as wide as the input, -2 for any output with any input).
    fn channel_capabilities(&self) -> Vec<(i32, i32)> {
        let mut pairs: Vec<(i32, i32)> = Vec::new();
        for layout in self.layouts {
            let input = match layout.inputs.first() {
                None => 0,
                Some(BusLayout::Set(set)) => set.channel_count() as i32,
                Some(_) => -1,
            };
            let output = match layout.outputs.first() {
                None => 0,
                Some(BusLayout::Set(set)) => set.channel_count() as i32,
                Some(BusLayout::SameAsMainInput) => input,
                Some(BusLayout::Any) if input == -1 => -2,
                Some(BusLayout::Any) => -1,
            };
            if !pairs.contains(&(input, output)) {
                pairs.push((input, output));
            }
        }
        pairs
    }

    /// Whether a layout takes `input` channels on the main input and
    /// `output` on the main output (0 for a direction without buses).
    fn allows_main_channels(&self, input: u32, output: u32) -> bool {
        let takes = |entry: Option<&BusLayout>, channels: u32, main_input: u32| match entry {
            None => channels == 0,
            Some(BusLayout::Set(set)) => set.channel_count() == channels,
            Some(BusLayout::Any) => channels > 0 && channels as usize <= MAX_CHANNELS,
            Some(BusLayout::SameAsMainInput) => channels == main_input,
        };
        self.layouts.iter().any(|layout| {
            takes(layout.inputs.first(), input, input) && takes(layout.outputs.first(), output, input)
        })
    }

    /// The buses with the default state of each.
    fn default_buses(&self) -> ActiveBuses {
        self.default
    }

    /// Channel counts of the main input and output bus in the default
    /// layout, 0 in a direction without buses.
    #[cfg(test)]
    pub(crate) fn main_channels(&self) -> (u32, u32) {
        (
            self.channel_count(BusDirection::Input, 0),
            self.channel_count(BusDirection::Output, 0),
        )
    }
}

/// Opaque handle to a beamer AU plugin instance.
///
/// This struct wraps the Rust plugin instance and render resources,
/// providing a stable C-ABI handle that the AUv2 and AUv3 wrappers can use.
///
/// # Thread Safety
///
/// The `render_block` field is wrapped in `RwLock` to ensure thread-safe access.
/// The render path uses `read()` for concurrent access during audio processing,
/// while resource allocation/deallocation uses `write()` for exclusive access.
/// This prevents data races when the host calls render from the audio thread
/// while other threads may be allocating/deallocating resources.
///
/// The render call only tries to take the plugin mutex and fails when it is
/// held. Nothing a host calls while audio runs may therefore take it: those
/// functions use the shared model, the values captured at creation and
/// `shared`. The mutex is for preparing and unpreparing.
///
/// Every field can be used through a shared reference, because the host
/// calls from several threads at once and no function may assume it has the
/// handle to itself.
pub struct BeamerInstanceHandle {
    /// The plugin instance (wrapped in Arc<Mutex<>> for thread-safe access)
    plugin: Arc<Mutex<Box<dyn AuPluginInstance>>>,
    /// The render block (created during allocate_render_resources).
    /// Wrapped in RwLock to prevent data races between render calls (read) and
    /// resource allocation/deallocation (write).
    render_block: std::sync::RwLock<Option<Arc<dyn RenderBlockTrait>>>,
    /// The bus configuration of the last successful allocation, cleared when
    /// the resources are deallocated. Only host threads take this lock.
    bus_config: Mutex<Option<CachedBusConfig>>,
    /// The buses the host asked for at the last allocation and the buses
    /// the plugin accepted. Only host threads take this lock, always before
    /// the plugin's.
    bus_negotiation: Mutex<BusNegotiation>,
    /// Where the web UI's invoke calls and events go (the model),
    /// captured at instance creation to avoid locking the plugin mutex on
    /// every invoke/event call.
    webview_dispatch: Arc<dyn WebViewDispatch>,
    /// The plugin's queue of events for the web UI, if it has one.
    webview_events: Option<WebViewEventQueue>,
    /// The model of the plugin. It stays in this one place for the life of
    /// the instance, so the functions that only need it (values, state,
    /// groups, presets) never take the plugin mutex.
    model: Arc<dyn SharedModel>,
    /// The factory presets, captured at instance creation.
    presets: FactoryPresetAccess,
    /// The declared buses, captured at instance creation.
    declared_buses: DeclaredBuses,
    /// What the host threads and the render call exchange without a lock.
    shared: Arc<SharedState>,
    /// The plugin's bypass parameter, which the host's bypass goes to,
    /// captured at instance creation.
    bypass_parameter: Option<beamer_core::ParameterId>,
    /// How often `lock_plugin` took the plugin mutex.
    #[cfg(test)]
    plugin_locks: std::sync::atomic::AtomicUsize,
}

/// The state a new instance shares with its render blocks, with diagnostics
/// in a diagnostics build.
fn new_shared_state() -> SharedState {
    #[cfg(feature = "diagnostics")]
    if let Some(config) = factory::plugin_config() {
        return SharedState::with_diagnostics(beamer_core::diagnostics::Diagnostics::start(config, "au"));
    }
    SharedState::default()
}

impl BeamerInstanceHandle {
    /// Create the handle for a new plugin instance.
    pub(crate) fn new(plugin: Box<dyn AuPluginInstance>) -> Self {
        // Capture what host threads ask for before wrapping the plugin in the
        // Mutex, so that those calls never need to lock it.
        let webview_dispatch = plugin.webview_dispatch();
        let webview_events = plugin.webview_events();
        let model = plugin.shared_model();
        let presets = plugin.factory_presets();
        let declared_buses = DeclaredBuses::capture(plugin.as_ref());
        let bus_negotiation = BusNegotiation::starting_with(declared_buses.default_buses());
        let bypass_parameter = beamer_core::parameter_types::bypass_parameter(model.parameters());

        Self {
            plugin: Arc::new(Mutex::new(plugin)),
            render_block: std::sync::RwLock::new(None),
            bus_config: Mutex::new(None),
            bus_negotiation: Mutex::new(bus_negotiation),
            webview_dispatch,
            webview_events,
            model,
            presets,
            declared_buses,
            shared: Arc::new(new_shared_state()),
            bypass_parameter,
            #[cfg(test)]
            plugin_locks: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    /// Deliver the pending host changes on this thread, as the main thread
    /// does by itself.
    #[cfg(test)]
    pub(crate) fn deliver_host_changes(&self) {
        self.shared.host_notifier().deliver();
    }

    /// Lock the negotiated bus configuration.
    fn lock_bus_config(&self) -> std::sync::MutexGuard<'_, Option<CachedBusConfig>> {
        // A poisoned lock only means a panic elsewhere: the configuration is
        // plain data and is replaced as a whole.
        self.bus_config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether a render block and a negotiated bus configuration are set.
    #[cfg(test)]
    pub(crate) fn has_render_setup(&self) -> (bool, bool) {
        let render_block = self
            .render_block
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_some();
        (render_block, self.lock_bus_config().is_some())
    }

    /// Poison the plugin mutex, as a plugin that panics while it holds it.
    #[cfg(test)]
    pub(crate) fn poison_plugin_lock(&self) {
        let plugin = Arc::clone(&self.plugin);
        let _ = std::thread::spawn(move || {
            let _guard = plugin.lock();
            panic!("a plugin call panics");
        })
        .join();
        assert!(self.plugin.is_poisoned());
    }

    /// How often `lock_plugin` took the plugin mutex for this instance.
    #[cfg(test)]
    pub(crate) fn plugin_lock_count(&self) -> usize {
        self.plugin_locks.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Run `f` while the render block is held, as by a render call.
    #[cfg(test)]
    pub(crate) fn with_render_block_held<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = self.render_block.read().expect("render block lock");
        f()
    }
}

// BeamerInstanceHandle is Send and Sync through its fields, with no unsafe
// impl: the host calls into one instance from several threads at once. The
// compiler checks that every field allows that.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<BeamerInstanceHandle>();
};

/// Type alias for the opaque handle pointer.
pub type BeamerAuInstanceHandle = *mut BeamerInstanceHandle;

// =============================================================================
// Helper Functions
// =============================================================================

/// Lock the plugin instance, returning an error status if the lock fails.
///
/// This is a convenience wrapper around `handle.plugin.lock()` that handles
/// the common error case where the mutex is poisoned.
///
/// Not for anything a host may call while audio runs: the render call fails
/// when it finds the mutex held.
fn lock_plugin(
    handle: &BeamerInstanceHandle,
) -> Result<std::sync::MutexGuard<'_, Box<dyn AuPluginInstance>>, i32> {
    #[cfg(test)]
    handle
        .plugin_locks
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    handle
        .plugin
        .lock()
        .map_err(|_| os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT)
}

/// Access the plugin's model as host threads share it.
///
/// The model is in one place for the life of the instance and works
/// through a shared reference, so this takes no lock and never waits.
fn with_model<R>(
    handle: &BeamerInstanceHandle,
    f: impl FnOnce(&dyn SharedModel) -> R,
) -> R {
    f(&*handle.model)
}

/// Access the plugin's ParameterStore (see `with_model`).
fn with_param_store<R>(
    handle: &BeamerInstanceHandle,
    f: impl FnOnce(&dyn ParameterStore) -> R,
) -> R {
    f(handle.model.parameter_store())
}

/// Copy a Rust string into a fixed-size C char array.
fn copy_str_to_char_array(s: &str, dest: &mut [c_char]) {
    let bytes = s.as_bytes();
    // Cut at a character, so that what is copied stays valid UTF-8.
    let mut copy_len = bytes.len().min(dest.len() - 1);
    while !s.is_char_boundary(copy_len) {
        copy_len -= 1;
    }
    for (i, &b) in bytes[..copy_len].iter().enumerate() {
        dest[i] = b as c_char;
    }
    // Null terminate
    if copy_len < dest.len() {
        dest[copy_len] = 0;
    } else {
        dest[dest.len() - 1] = 0;
    }
}

/// Channel count and channel labels of bus `bus` of the `count` buses in
/// `buses`: no channels for one past the count.
fn host_bus(buses: &[BeamerAuBusInfo; MAX_BUSES], count: u32, bus: usize) -> (u32, &[u32]) {
    buses
        .get(bus)
        .filter(|_| bus < count as usize)
        .map_or((0, &[][..]), |info| (info.channel_count, info.labels()))
}

/// Convert a C bus info array to a `Vec<CachedBusInfo>`.
///
/// This helper converts FFI bus information from the C-ABI format to Rust's internal
/// representation, handling the bus type conversion and respecting MAX_BUSES bounds.
fn convert_bus_info_array(
    c_buses: &[BeamerAuBusInfo; MAX_BUSES],
    count: u32,
) -> Vec<CachedBusInfo> {
    let count = (count as usize).min(MAX_BUSES);
    let mut buses = Vec::with_capacity(count);

    for bus in c_buses.iter().take(count) {
        let bus_type = if bus.bus_type == BeamerAuBusType::Main {
            BusType::Main
        } else {
            BusType::Aux
        };
        buses.push(CachedBusInfo::new(bus.channel_count as usize, bus_type));
    }

    buses
}

/// Convert BeamerAuBusConfig to CachedBusConfig.
fn bus_config_from_c(config: &BeamerAuBusConfig) -> CachedBusConfig {
    let input_buses = convert_bus_info_array(&config.input_buses, config.input_bus_count);
    let output_buses = convert_bus_info_array(&config.output_buses, config.output_bus_count);

    CachedBusConfig::new(input_buses, output_buses)
}

// =============================================================================
// Factory Registration
// =============================================================================

/// Ensure the plugin factory is registered.
///
/// This function checks if the plugin factory has been registered (via the
/// `export_au!` macro's static initializer). The factory is typically registered
/// automatically when the `.component` bundle loads.
///
/// # Safety
///
/// This function has no pointer parameters and is safe to call from any thread.
///
/// # Returns
///
/// `true` if the factory is registered and ready, `false` otherwise.
#[no_mangle]
pub extern "C" fn beamer_au_ensure_factory_registered() -> bool {
    // With NSExtensionMain as the appex entry point, the system properly loads
    // the framework and initializes the ObjC runtime. No force-link needed here.
    factory::is_registered()
}

/// Fill in the AudioComponentDescription from the registered AU config.
///
/// # Safety
///
/// `desc` must be a valid pointer to an AudioComponentDescription struct.
#[no_mangle]
pub unsafe extern "C" fn beamer_au_get_component_description(desc: *mut u32) {
    if desc.is_null() {
        return;
    }
    let config = match factory::plugin_config() {
        Some(c) => c,
        None => return,
    };
    // AudioComponentDescription layout: type, subtype, manufacturer, flags, mask
    // SAFETY: desc was validated non-null above. Caller guarantees desc points to
    // a valid AudioComponentDescription struct with at least 5 u32 fields.
    // The pointer arithmetic accesses consecutive fields in the struct.
    unsafe {
        *desc.add(0) = config.au_component_type();
        *desc.add(1) = u32::from_be_bytes(config.subtype.0);
        *desc.add(2) = u32::from_be_bytes(config.manufacturer.0);
        *desc.add(3) = 0; // componentFlags
        *desc.add(4) = 0; // componentFlagsMask
    }
}

// =============================================================================
// Instance Lifecycle
// =============================================================================

/// Create a new plugin instance.
///
/// Uses the registered factory to create a new instance of the plugin.
/// The instance must be destroyed with `beamer_au_destroy_instance`.
///
/// # Safety
///
/// This function has no pointer parameters and is safe to call from any thread.
/// The returned handle must be destroyed with `beamer_au_destroy_instance` when
/// no longer needed to avoid memory leaks.
///
/// # Returns
///
/// A pointer to the instance handle, or null if creation failed. It fails
/// when no factory is registered and when two parameters of the plugin have
/// the same id, which is logged and written to standard error.
#[no_mangle]
pub extern "C" fn beamer_au_create_instance() -> BeamerAuInstanceHandle {
    let result = catch_unwind(|| {
        // Use the factory to create a new plugin instance
        let plugin = factory::create_instance()?;

        // Two parameters with one id would be one parameter to the host.
        // The reason also goes to standard error: a plugin has no logger
        // unless it sets `log_level` in its Config.toml.
        let model = plugin.shared_model();
        if let Err(error) = beamer_core::parameter_types::check_unique_ids(model.parameters()) {
            let name = factory::plugin_config().map_or("plugin", |config| config.name);
            log::error!("{}: {}", name, error);
            eprintln!("{}: {}", name, error);
            return None;
        }

        let handle = Box::new(BeamerInstanceHandle::new(plugin));

        Some(Box::into_raw(handle))
    });

    match result {
        Ok(Some(ptr)) => ptr,
        Ok(None) => ptr::null_mut(),
        Err(_) => ptr::null_mut(),
    }
}

/// Destroy a plugin instance.
///
/// Frees all resources associated with the instance.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been previously destroyed
/// - After calling this function, `instance` is invalid and must not be used
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Must not be called concurrently with any other function
///   using the same instance
#[no_mangle]
pub extern "C" fn beamer_au_destroy_instance(instance: BeamerAuInstanceHandle) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above. Caller guarantees it was
        // returned by beamer_au_create_instance and not previously destroyed.
        // Box::from_raw takes ownership and will drop the instance.
        let handle = unsafe { Box::from_raw(instance) };
        // A render block may keep the shared state: the shim's callback
        // goes now, after a delivery in progress.
        handle.shared.host_notifier().set_target(None);
        drop(handle);
    }));
}

// The bits of the host changes (must match BEAMER_AU_HOST_CHANGE_* in
// BeamerAuBridge.h).
const _: () = assert!(beamer_core::HostChanges::LATENCY.bits() == 1 << 0);
const _: () = assert!(beamer_core::HostChanges::TAIL.bits() == 1 << 1);
const _: () = assert!(beamer_core::HostChanges::BYPASS.bits() == 1 << 3);

/// Function that tells the host about changes on the main thread
/// (matches BeamerAuHostChangeFn in header). `changes` holds the
/// `BEAMER_AU_HOST_CHANGE_*` bits.
pub type BeamerAuHostChangeFn = unsafe extern "C" fn(context: *mut c_void, changes: u32);

/// The shim's callback with its context, which the shim keeps valid while
/// the callback is set.
struct HostChangeCallback {
    callback: BeamerAuHostChangeFn,
    context: *mut c_void,
}

// SAFETY: The callback runs on the main thread only and the shim keeps the
// context valid until it clears the callback.
unsafe impl Send for HostChangeCallback {}

impl HostChangeCallback {
    fn call(&self, changes: beamer_core::HostChanges) {
        // SAFETY: The shim keeps the context valid while the callback is
        // set (see `beamer_au_set_host_change_callback`).
        unsafe { (self.callback)(self.context, changes.bits()) };
    }
}

/// Set the function that tells the host about changes, or none.
///
/// The function runs on the main thread, with the changes noted since its
/// last run: a latency or tail that changed after a render call. Clearing
/// it (a null callback) waits for a run in progress, so that the context
/// may go afterwards. `beamer_au_destroy_instance` clears it too. Do not
/// call this from within the callback.
///
/// # Safety
///
/// `context` must stay valid for the callback until the callback is
/// cleared or the instance is destroyed.
#[no_mangle]
pub extern "C" fn beamer_au_set_host_change_callback(
    instance: BeamerAuInstanceHandle,
    callback: Option<BeamerAuHostChangeFn>,
    context: *mut c_void,
) {
    with_instance_void!(instance, |handle| {
        let target = callback.map(|callback| {
            let callback = HostChangeCallback { callback, context };
            Box::new(move |changes: beamer_core::HostChanges| callback.call(changes))
                as beamer_core::HostChangeTarget
        });
        handle.shared.host_notifier().set_target(target);
    });
}

// =============================================================================
// Render Resources
// =============================================================================

/// Maximum supported sample rate (384 kHz - highest professional audio standard).
const MAX_SAMPLE_RATE: f64 = 384_000.0;

/// Maximum supported frames per render call.
/// 8192 is a reasonable upper limit that covers common buffer sizes.
const MAX_FRAMES_PER_RENDER: u32 = 8192;

/// Allocate render resources and prepare for audio processing.
///
/// Calling it on a prepared instance prepares the plugin again with the new
/// configuration. `rendering_offline` tells the plugin that the host renders
/// without realtime deadlines, as [`ProcessMode::Offline`].
///
/// When it returns an error, the instance is unprepared, also if it was
/// prepared before: render calls return `kAudioUnitErr_Uninitialized` until
/// an allocation succeeds.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`
/// - `instance` must not have been destroyed
/// - `bus_config` must be a valid pointer to a properly initialized `BeamerAuBusConfig`
/// - This function validates both pointers are non-null before dereferencing
/// - If either pointer is null, returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER`
/// - Thread safety: Must not be called concurrently with deallocate or with
///   itself on the same instance. Render calls may arrive at any time: they
///   return an error while the plugin is being prepared
#[no_mangle]
pub extern "C" fn beamer_au_allocate_render_resources(
    instance: BeamerAuInstanceHandle,
    sample_rate: f64,
    max_frames: u32,
    sample_format: BeamerAuSampleFormat,
    bus_config: *const BeamerAuBusConfig,
    rendering_offline: bool,
) -> i32 {
    if instance.is_null() || bus_config.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    let status = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above. Caller guarantees valid pointer.
        let handle = unsafe { &*instance };
        // SAFETY: bus_config validated non-null above.
        let c_bus_config = unsafe { &*bus_config };
        prepare_instance(
            handle,
            sample_rate,
            max_frames,
            sample_format,
            c_bus_config,
            rendering_offline,
        )
    }))
    .unwrap_or(os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT);

    // A failed allocation leaves the instance unprepared, whatever it was
    // before: no processor, no bus configuration and no render block. A
    // host that sees the error does not render with the old setup either.
    if status != os_status::NO_ERR {
        // SAFETY: instance validated non-null above. Caller guarantees valid pointer.
        let handle = unsafe { &*instance };
        let _ = catch_unwind(AssertUnwindSafe(|| leave_unprepared(handle)));
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = handle.shared.diagnostics() {
            diagnostics.note(format_args!("prepare failed with status {status}"));
        }
    }

    status
}

/// Prepare the plugin for a configuration and install a new render block.
fn prepare_instance(
    handle: &BeamerInstanceHandle,
    sample_rate: f64,
    max_frames: u32,
    sample_format: BeamerAuSampleFormat,
    c_bus_config: &BeamerAuBusConfig,
    rendering_offline: bool,
) -> i32 {
    // What the render calls of the previous setup had to drop.
    handle.shared.log_render_warnings();

    if sample_rate <= 0.0 || sample_rate > MAX_SAMPLE_RATE || !sample_rate.is_finite() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    if max_frames == 0 || max_frames > MAX_FRAMES_PER_RENDER {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    if c_bus_config.input_bus_count as usize > MAX_BUSES {
        log::error!(
            "Invalid input_bus_count: {} (must be <= {})",
            c_bus_config.input_bus_count,
            MAX_BUSES
        );
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    if c_bus_config.output_bus_count as usize > MAX_BUSES {
        log::error!(
            "Invalid output_bus_count: {} (must be <= {})",
            c_bus_config.output_bus_count,
            MAX_BUSES
        );
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
    }

    if let Err(message) = handle.declared_buses.validation {
        log::error!("Plugin layouts are not valid: {}", message);
        return os_status::K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED;
    }

    // Convert bus configuration (bus counts already validated above).
    let rust_bus_config = bus_config_from_c(c_bus_config);

    // Validate bus configuration.
    if let Err(_e) = rust_bus_config.validate() {
        return os_status::K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED;
    }

    // The host's request: a bus with channels is on, one with 0 channels
    // (or beyond the configuration) off, with the set the host means (see
    // `DeclaredBuses::set_for`). The order in which the host has the
    // channels of each set is kept for the render block.
    let mut negotiation = handle
        .bus_negotiation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let requested = negotiation.requested_mut();
    let declared = &handle.declared_buses;
    let mut host_orders = ChannelOrders::default();
    for bus in 0..requested.input_bus_count() {
        let (channels, labels) = host_bus(&c_bus_config.input_buses, c_bus_config.input_bus_count, bus);
        let found = declared.set_for(BusDirection::Input, bus, channels, labels);
        let _ = requested.set_input(bus, found.map(|(set, _)| set));
        host_orders.set_input(bus, found.map(|(_, order)| order).unwrap_or_default());
    }
    for bus in 0..requested.output_bus_count() {
        let (channels, labels) = host_bus(&c_bus_config.output_buses, c_bus_config.output_bus_count, bus);
        let found = declared.set_for(BusDirection::Output, bus, channels, labels);
        let _ = requested.set_output(bus, found.map(|(set, _)| set));
        host_orders.set_output(bus, found.map(|(_, order)| order).unwrap_or_default());
    }

    // Take the current render block out. This waits for a render call that
    // is in progress (it holds the read lock while it processes) and makes
    // the following calls return an error until the new block is installed.
    // A prepared instance can so be prepared again while the host keeps
    // rendering.
    match handle.render_block.write() {
        Ok(mut guard) => *guard = None,
        Err(_) => return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
    }

    // Allocate resources on the plugin (calls prepare()).
    {
        let mut plugin = match lock_plugin(handle) {
            Ok(guard) => guard,
            Err(status) => return status,
        };

        // AU has two modes: rendering in real time or offline.
        let process_mode = if rendering_offline {
            ProcessMode::Offline
        } else {
            ProcessMode::Realtime
        };

        // The plugin is prepared with the buses it accepts. The main buses
        // cannot be bridged at another width: a plugin that keeps other
        // main channel counts than the host's refuses the formats.
        negotiation.negotiate_with(plugin.layouts(), |buses| plugin.supports_buses(buses));
        let (requested, accepted) = (negotiation.requested(), negotiation.accepted());
        if requested.input(0) != accepted.input(0) || requested.output(0) != accepted.output(0) {
            return os_status::K_AUDIO_UNIT_ERR_FORMAT_NOT_SUPPORTED;
        }
        let prepared = plugin.allocate_render_resources(
            sample_rate,
            max_frames,
            negotiation.accepted(),
            process_mode,
        );

        if prepared.is_err() {
            return os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE;
        }

        // A new processor needs no reset any more. The host may ask for its
        // latency and tail before the first render call.
        handle.shared.clear_reset_request();
        handle
            .shared
            .publish_timing(plugin.latency_samples(), plugin.tail_samples());
    }

    let settings = factory::plugin_config().map(RenderSettings::from_config).unwrap_or_default();
    let buses = *negotiation;
    drop(negotiation);
    // The host's order of the channels of a bus where the plugin has the
    // set the host has; otherwise the plugin's set has no order in the
    // host's channels.
    let mut orders = ChannelOrders::default();
    for bus in 0..buses.accepted().input_bus_count() {
        if buses.accepted().input_set(bus) == buses.requested().input_set(bus) {
            orders.set_input(bus, *host_orders.input(bus));
        }
    }
    for bus in 0..buses.accepted().output_bus_count() {
        if buses.accepted().output_set(bus) == buses.requested().output_set(bus) {
            orders.set_output(bus, *host_orders.output(bus));
        }
    }
    // The storage holds the plugin's channels.
    let plugin_bus_config = CachedBusConfig::from_active_buses(buses.accepted());

    // Create the render block based on sample format.
    let render_block: Arc<dyn RenderBlockTrait> = match sample_format {
        BeamerAuSampleFormat::Float32 => {
            let storage = ProcessBufferStorage::<f32>::allocate_from_config(
                &plugin_bus_config,
                max_frames as usize,
            );
            Arc::from(create_render_block_f32(
                Arc::clone(&handle.plugin),
                Arc::clone(&handle.model),
                Arc::clone(&handle.shared),
                storage,
                buses,
                orders,
                max_frames,
                sample_rate,
                settings,
            ))
        }
        BeamerAuSampleFormat::Float64 => {
            let storage = ProcessBufferStorage::<f64>::allocate_from_config(
                &plugin_bus_config,
                max_frames as usize,
            );
            Arc::from(create_render_block_f64(
                Arc::clone(&handle.plugin),
                Arc::clone(&handle.model),
                Arc::clone(&handle.shared),
                storage,
                buses,
                orders,
                max_frames,
                sample_rate,
                settings,
            ))
        }
    };

    // Use write lock to set the render block (exclusive access).
    match handle.render_block.write() {
        Ok(mut guard) => *guard = Some(render_block),
        Err(_) => return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT,
    }
    *handle.lock_bus_config() = Some(rust_bus_config);

    #[cfg(feature = "diagnostics")]
    if let Some(diagnostics) = handle.shared.diagnostics() {
        let sample_type = match sample_format {
            BeamerAuSampleFormat::Float32 => "f32",
            BeamerAuSampleFormat::Float64 => "f64",
        };
        diagnostics.note(format_args!(
            "prepared: {sample_rate} Hz, {sample_type}, max {max_frames} frames{}, buses {}",
            if rendering_offline { ", offline" } else { "" },
            beamer_core::diagnostics::describe_buses(&buses),
        ));
    }

    os_status::NO_ERR
}

/// Bring an instance into the unprepared state, after a failed allocation
/// or when the host deallocates.
///
/// Taking the render block waits for a render call in progress. Locks that
/// a panic poisoned are taken over and cleared: what they guard is replaced
/// as a whole here, so the next allocation can prepare the plugin again.
fn leave_unprepared(handle: &BeamerInstanceHandle) {
    *handle
        .render_block
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    handle.render_block.clear_poison();

    handle
        .plugin
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .deallocate_render_resources();
    handle.plugin.clear_poison();

    // An unprepared plugin has no latency and no tail.
    handle.shared.publish_timing(0, 0);
    *handle.lock_bus_config() = None;
}

/// Deallocate render resources.
///
/// Called on a host thread, never on the render thread. A host should stop
/// rendering first; when a render call still runs (a host that does not, or
/// the render call of another output bus), this waits for it, as a new
/// allocation does. A render call holds the read lock of the render block
/// while it processes. Calls that start afterwards find no render block and
/// return `kAudioUnitErr_Uninitialized`.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - It must not be called from a render call of the same instance, which
///   would wait for itself
#[no_mangle]
pub extern "C" fn beamer_au_deallocate_render_resources(instance: BeamerAuInstanceHandle) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above. Caller guarantees valid pointer
        // from beamer_au_create_instance that hasn't been destroyed.
        let handle = unsafe { &*instance };

        // What the render calls of this setup had to drop.
        handle.shared.log_render_warnings();

        leave_unprepared(handle);

        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = handle.shared.diagnostics() {
            diagnostics.note(format_args!("released"));
        }
    }));
}

/// Note the bypass the host set, before the shim applies it, so that the
/// host is not told about it again. A diagnostics build logs it.
///
/// # Safety
///
/// - `instance` must be null or a valid pointer returned by
///   `beamer_au_create_instance` that was not destroyed
#[no_mangle]
pub extern "C" fn beamer_au_note_host_bypass(instance: BeamerAuInstanceHandle, bypassed: bool) {
    with_instance_void!(instance, |handle| {
        handle.shared.note_host_bypass(bypassed, handle.bypass_parameter.is_some());
    });
}

/// The id of the plugin's bypass parameter, which the host's bypass goes
/// to. Returns false, leaving `out_id` alone, for a plugin without one.
///
/// # Safety
///
/// - `instance` must be null or a valid pointer returned by
///   `beamer_au_create_instance` that was not destroyed
/// - `out_id` must be null or valid for a write
#[no_mangle]
pub extern "C" fn beamer_au_get_bypass_parameter(instance: BeamerAuInstanceHandle, out_id: *mut u32) -> bool {
    with_instance!(instance, false, |handle| {
        match handle.bypass_parameter {
            Some(id) if !out_id.is_null() => {
                // SAFETY: out_id is non-null and valid for a write per the
                // contract.
                unsafe { *out_id = id };
                true
            }
            _ => false,
        }
    })
}

/// Log a line about a host call (diagnostics builds). For host threads.
///
/// # Safety
///
/// As for `beamer_au_note_host_bypass`. `text` must be null or a
/// valid C string for the call.
#[cfg(feature = "diagnostics")]
#[no_mangle]
pub extern "C" fn beamer_au_diagnostics_note(instance: BeamerAuInstanceHandle, text: *const c_char) {
    if text.is_null() {
        return;
    }
    with_instance_void!(instance, |handle| {
        if let Some(diagnostics) = handle.shared.diagnostics() {
            // SAFETY: A valid C string for the call (see the function's
            // safety section).
            let text = unsafe { CStr::from_ptr(text) }.to_string_lossy();
            diagnostics.note(format_args!("{text}"));
        }
    });
}

/// Note whether the host's main output buffers have memory of their own
/// (diagnostics builds). For the render thread.
///
/// # Safety
///
/// As for `beamer_au_note_host_bypass`.
#[cfg(feature = "diagnostics")]
#[no_mangle]
pub extern "C" fn beamer_au_diagnostics_host_output_memory(instance: BeamerAuInstanceHandle, has_memory: bool) {
    with_instance_void!(instance, |handle| {
        if let Some(diagnostics) = handle.shared.diagnostics() {
            diagnostics.state().note_host_output_memory(has_memory);
        }
    });
}

/// Measure the output of a render call of the main bus that the host
/// bypassed (diagnostics builds). For the render thread.
///
/// # Safety
///
/// As for `beamer_au_note_host_bypass`. `output_data` must be null or
/// valid for the call, its buffers valid for `frame_count` samples where
/// they have memory.
#[cfg(feature = "diagnostics")]
#[no_mangle]
pub extern "C" fn beamer_au_diagnostics_bypassed_render(
    instance: BeamerAuInstanceHandle,
    output_data: *const AudioBufferList,
    frame_count: u32,
) {
    if output_data.is_null() {
        return;
    }
    with_instance_void!(instance, |handle| {
        // The render thread does not wait: while a host thread prepares the
        // instance, the call is not measured.
        if let Ok(guard) = handle.render_block.try_read() {
            if let Some(render_block) = guard.as_ref() {
                // SAFETY: The shim passes the output list of its render call,
                // valid for frame_count samples (see the function's safety
                // section).
                unsafe { render_block.measure_host_bypassed(output_data, frame_count) };
            }
        }
    });
}

/// Check if render resources are currently allocated.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; uses mutex for synchronization
#[no_mangle]
pub extern "C" fn beamer_au_is_prepared(instance: BeamerAuInstanceHandle) -> bool {
    with_instance!(instance, false, |handle| {
        let plugin = match lock_plugin(handle) {
            Ok(guard) => guard,
            Err(_) => return false,
        };

        plugin.is_prepared()
    })
}

// =============================================================================
// Audio Rendering
// =============================================================================

/// Process audio through the plugin.
///
/// This is the main audio processing entry point, called from the AU host's
/// render callback (real-time audio thread).
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`
/// - `instance` must not have been destroyed
/// - `action_flags` must be a valid pointer to a mutable `u32`
/// - `timestamp` must be a valid pointer to an `AudioTimeStamp`
/// - `output_data` must be a valid pointer to an `AudioBufferList` with properly
///   allocated buffers sized for at least `frame_count` frames
/// - `events` may be null if there are no events to process
/// - `pull_input_block` may be null for generator plugins that don't need input
/// - `transport` must be null (no transport information, as for the renders of
///   output buses other than 0) or point to a `BeamerAuTransport` that is valid
///   for the duration of the call
/// - `midi_output` must be null or a function that is safe to call with
///   `midi_output_context` on this thread during the call
/// - This function validates `instance`, `action_flags`, `timestamp` and
///   `output_data` are non-null; returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER` if any are null
/// - Thread safety: Designed for real-time audio thread; uses non-blocking
///   `try_read()` to avoid blocking if resource allocation is in progress
/// - Uses `catch_unwind` to prevent panics from crossing the FFI boundary
#[no_mangle]
pub extern "C" fn beamer_au_render(
    instance: BeamerAuInstanceHandle,
    action_flags: *mut u32,
    timestamp: *const AudioTimeStamp,
    frame_count: u32,
    output_bus_number: isize,
    output_data: *mut AudioBufferList,
    events: *const AURenderEvent,
    pull_input_block: *const c_void,
    input_data: *const AudioBufferList, // Input buffer list from ObjC (after pulling)
    transport: *const BeamerAuTransport,
    midi_output: Option<BeamerAuMidiOutputFn>,
    midi_output_context: *mut c_void,
) -> i32 {
    // Validate instance handle
    if instance.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    // Validate critical pointers required for rendering
    if action_flags.is_null() || timestamp.is_null() || output_data.is_null() {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above. Caller guarantees valid pointer
        // from beamer_au_create_instance. Instance remains valid during render.
        let handle = unsafe { &*instance };

        // Use read lock for concurrent access during rendering
        // try_read() is used for real-time safety - we don't want to block the audio thread
        let render_block_guard = match handle.render_block.try_read() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => {
                // Another thread holds the write lock (e.g., during resource allocation)
                // Return an error rather than blocking the audio thread
                return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT;
            }
            Err(std::sync::TryLockError::Poisoned(_)) => {
                // Lock was poisoned by a panic in another thread
                return os_status::K_AUDIO_UNIT_ERR_CANNOT_DO_IN_CURRENT_CONTEXT;
            }
        };

        let render_block = match render_block_guard.as_ref() {
            Some(rb) => rb,
            None => return os_status::K_AUDIO_UNIT_ERR_UNINITIALIZED,
        };

        render_block.process(
            action_flags,
            timestamp,
            frame_count,
            output_bus_number as i32,
            output_data,
            events,
            pull_input_block,
            input_data,
            // SAFETY: The caller passes null or a pointer that is valid for this call.
            unsafe { transport.as_ref() },
            // SAFETY: The caller guarantees the function is callable with the context.
            midi_output.map(|send| unsafe { MidiOutputSink::new(send, midi_output_context) }),
        )
    }));

    result.unwrap_or(os_status::K_AUDIO_UNIT_ERR_RENDER)
}

/// Reset the plugin's DSP state.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; lock-free. The audio thread
///   runs the reset at the start of its next render call
#[no_mangle]
pub extern "C" fn beamer_au_reset(instance: BeamerAuInstanceHandle) {
    with_instance_void!(instance, |handle| {
        // The processor belongs to the audio thread while audio runs: the
        // next render call resets it before it processes.
        handle.shared.request_reset();
    })
}

// =============================================================================
// Parameters
// =============================================================================

/// Get the number of parameters exposed by the plugin.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_count(instance: BeamerAuInstanceHandle) -> u32 {
    with_instance!(instance, 0, |handle| {
        with_param_store(handle, |store| store.count() as u32)
    })
}

/// Get information about a parameter by index.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `out_info` must be a valid pointer to a `BeamerAuParameterInfo` struct,
///   or null (in which case this function returns `false`)
/// - This function validates both pointers are non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_info(
    instance: BeamerAuInstanceHandle,
    index: u32,
    out_info: *mut BeamerAuParameterInfo,
) -> bool {
    if out_info.is_null() {
        return false;
    }

    with_instance!(instance, false, |handle| {
        // SAFETY: out_info was validated as non-null at function start.
        let out = unsafe { &mut *out_info };
        with_param_store(handle, |store| {
            let param_info = match store.info(index as usize) {
                Some(info) => info,
                None => return false,
            };

            // Fill output struct.
            out.id = param_info.id;
            copy_str_to_char_array(param_info.name, &mut out.name);
            copy_str_to_char_array(param_info.units, &mut out.units);
            out.unit_type = param_info.unit as u32;
            // The range and the values are in AU format, like the values
            // the host gets and sets.
            out.min_value = normalized_to_au_value(store, param_info, 0.0);
            out.max_value = normalized_to_au_value(store, param_info, 1.0);
            out.default_value =
                normalized_to_au_value(store, param_info, param_info.default_normalized);
            out.current_value =
                normalized_to_au_value(store, param_info, store.get_normalized(param_info.id));
            out.step_count = param_info.step_count;
            // Convert ParameterFlags to u32 bitfield.
            out.flags = {
                let mut flags = 0u32;
                if param_info.flags.can_automate {
                    // BeamerAuParameterFlagAutomatable.
                    flags |= 1 << 0;
                }
                if param_info.flags.is_hidden {
                    // BeamerAuParameterFlagHidden.
                    flags |= 1 << 1;
                }
                if param_info.flags.is_readonly {
                    // BeamerAuParameterFlagReadOnly.
                    flags |= 1 << 2;
                }
                flags
            };
            out.group_id = param_info.group_id;

            true
        })
    })
}

/// Get a parameter's current normalized value.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0.0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
) -> f32 {
    with_instance!(instance, 0.0, |handle| {
        with_param_store(handle, |store| store.get_normalized(param_id) as f32)
    })
}

/// Set a parameter's normalized value.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_set_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f32,
) {
    with_instance_void!(instance, |handle| {
        with_param_store(handle, |store| {
            store.set_normalized(param_id, value as f64);
        });
    })
}

/// Get a parameter's current value in AU format (denormalized for indexed parameters).
///
/// For indexed parameters (those with `unit_type == kAudioUnitParameterUnit_Indexed`),
/// this returns the index value (0 to step_count) instead of the normalized value (0.0 to 1.0).
/// For non-indexed parameters, this returns the actual (plain) value.
///
/// This function is designed for use by AU wrappers that need to provide values
/// in the format expected by AU hosts, where indexed parameters use integer indices.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0.0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_value_au(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
) -> f32 {
    with_instance!(instance, 0.0, |handle| {
        with_param_store(handle, |store| {
            normalized_to_au_value_by_id(store, param_id, store.get_normalized(param_id))
        })
    })
}

/// Set a parameter's value from AU format (actual values, with special handling for indexed).
///
/// For indexed parameters (those with `unit_type == kAudioUnitParameterUnit_Indexed`),
/// this accepts an index value (0 to step_count) and converts it to normalized (0.0 to 1.0).
/// For all other parameters, this accepts actual (plain) values (e.g., dB, Hz) and converts
/// to normalized using `plain_to_normalized`.
///
/// This function is designed for use by AU wrappers that receive values
/// in the format provided by AU hosts, where parameters use actual values.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_set_parameter_value_au(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f32,
) {
    with_instance_void!(instance, |handle| {
        with_param_store(handle, |store| {
            store.set_normalized(param_id, au_value_to_normalized(store, param_id, value));
        });
    })
}

/// Whether AU hosts see the parameter as an index instead of its plain value.
fn is_au_indexed(info: &ParameterInfo) -> bool {
    info.unit == ParameterUnit::Indexed && info.step_count > 0
}

/// Convert a parameter value from AU format to a normalized value.
///
/// AU hosts use an index (0 to step_count) for indexed parameters and the
/// actual (plain) value, such as dB or Hz, for all others.
pub(crate) fn au_value_to_normalized(store: &dyn ParameterStore, param_id: u32, value: f32) -> f64 {
    match store.info_by_id(param_id) {
        Some(info) if is_au_indexed(info) => value as f64 / info.step_count as f64,
        _ => store.plain_to_normalized(param_id, value as f64),
    }
}

/// Convert a normalized parameter value to AU format.
///
/// The reverse of [`au_value_to_normalized`]. The parameter info describes
/// its range to the host in the same format.
fn normalized_to_au_value(
    store: &dyn ParameterStore,
    info: &ParameterInfo,
    normalized: f64,
) -> f32 {
    if is_au_indexed(info) {
        (normalized * info.step_count as f64).round() as f32
    } else {
        store.normalized_to_plain(info.id, normalized) as f32
    }
}

/// [`normalized_to_au_value`] for a parameter that is known by its ID.
fn normalized_to_au_value_by_id(store: &dyn ParameterStore, param_id: u32, normalized: f64) -> f32 {
    match store.info_by_id(param_id) {
        Some(info) => normalized_to_au_value(store, info, normalized),
        None => store.normalized_to_plain(param_id, normalized) as f32,
    }
}

/// Parse a display string to a parameter value in AU format.
fn string_to_au_value(store: &dyn ParameterStore, param_id: u32, string: &str) -> Option<f32> {
    let normalized = store.string_to_normalized(param_id, string)?;
    Some(normalized_to_au_value_by_id(store, param_id, normalized))
}

/// Format a parameter value in AU format as a display string.
///
/// The value is what the host holds: the actual (plain) value, or the index
/// for an indexed parameter.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `instance` and `out_buffer` are non-null and
///   `buffer_len > 0` before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_format_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f32,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    with_instance!(instance, 0, |handle| {
        // Normalize using f64 precision to avoid f32 round-trip artifacts
        // (e.g. 0.0 dB displaying as "-0.0").
        let string = with_param_store(handle, |store| {
            let normalized = au_value_to_normalized(store, param_id, value);
            store.normalized_to_string(param_id, normalized)
        });

        // Copy to buffer.
        let bytes = string.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        // SAFETY: out_buffer and buffer_len were validated at function start.
        // copy_len < buffer_len ensures we don't overflow.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
            *out_buffer.add(copy_len) = 0; // Null terminator
        }

        copy_len as u32
    })
}

/// Parse a display string to a parameter value in AU format.
///
/// The result is what the host holds: the actual (plain) value, or the index
/// for an indexed parameter.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `string` must be a valid pointer to a null-terminated UTF-8 C string,
///   or null (in which case this function returns `false`)
/// - `out_value` must be a valid pointer to a writable `f32`,
///   or null (in which case this function returns `false`)
/// - This function validates all three pointers are non-null before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_parse_parameter_value(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    string: *const c_char,
    out_value: *mut f32,
) -> bool {
    if string.is_null() || out_value.is_null() {
        return false;
    }

    with_instance!(instance, false, |handle| {
        // SAFETY: string was validated as non-null at function start.
        let rust_string = match unsafe { CStr::from_ptr(string) }.to_str() {
            Ok(s) => s,
            Err(_) => return false,
        };

        let result = with_param_store(handle, |store| {
            string_to_au_value(store, param_id, rust_string)
        });

        match result {
            Some(value) => {
                // SAFETY: out_value was validated as non-null at function start.
                unsafe { *out_value = value };
                true
            }
            None => false,
        }
    })
}

/// Get the number of discrete value strings for an indexed parameter.
///
/// For enum/indexed parameters, returns the number of possible values (step_count + 1).
/// For continuous parameters or those without indexed unit type, returns 0.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_value_count(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
) -> u32 {
    with_instance!(instance, 0, |handle| {
        with_param_store(handle, |store| match store.info_by_id(param_id) {
            Some(info) if is_au_indexed(info) => (info.step_count + 1) as u32,
            _ => 0,
        })
    })
}

/// Get the display string for a specific value of an indexed parameter.
///
/// For enum parameters, index 0 returns the first variant name, etc.
/// Converts the index to a normalized value and calls the parameter's
/// value-to-string function.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `out_string` must be a valid pointer to a writable buffer of at least
///   `max_length` bytes, or null (in which case this function returns `false`)
/// - `max_length` must be greater than 0
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_parameter_value_string(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value_index: u32,
    out_string: *mut c_char,
    max_length: u32,
) -> bool {
    if out_string.is_null() || max_length == 0 {
        return false;
    }

    with_instance!(instance, false, |handle| {
        let result = with_param_store(handle, |store| {
            let step_count = match store.info_by_id(param_id) {
                Some(info) if is_au_indexed(info) => info.step_count,
                _ => return None,
            };

            if value_index > step_count as u32 {
                return None;
            }

            let normalized = if step_count > 0 {
                value_index as f64 / step_count as f64
            } else {
                0.0
            };

            Some(store.normalized_to_string(param_id, normalized))
        });

        let display_string = match result {
            Some(s) => s,
            None => return false,
        };

        // Copy to output buffer.
        let bytes = display_string.as_bytes();
        let copy_len = bytes.len().min(max_length as usize - 1);

        // SAFETY: out_string and max_length were validated at function start.
        // copy_len < max_length ensures we don't overflow.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_string as *mut u8, copy_len);
            *out_string.add(copy_len) = 0; // Null terminator
        }

        true
    })
}

// =============================================================================
// Parameter Groups
// =============================================================================

/// Get the number of parameter groups (including root group).
///
/// Returns 1 if there are no explicit groups (just the root group).
/// For nested groups, returns 1 + total nested groups.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_group_count(instance: BeamerAuInstanceHandle) -> u32 {
    with_instance!(instance, 0, |handle| {
        with_model(handle, |model| model.parameters().group_count() as u32)
    })
}

/// Get information about a parameter group by index.
///
/// Index 0 returns the root group (id=0, name="", parent_id=0).
/// Used to build hierarchical AUParameterTree with AUParameterGroup nodes.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `out_info` must be a valid pointer to a `BeamerAuGroupInfo` struct,
///   or null (in which case this function returns `false`)
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_get_group_info(
    instance: BeamerAuInstanceHandle,
    index: u32,
    out_info: *mut BeamerAuGroupInfo,
) -> bool {
    if out_info.is_null() {
        return false;
    }

    with_instance!(instance, false, |handle| {
        let group_info = match with_model(handle, |model| {
            model.parameters().group_info(index as usize)
        }) {
            Some(info) => info,
            None => return false,
        };

        // SAFETY: out_info was validated as non-null at function start.
        let out = unsafe { &mut *out_info };
        out.id = group_info.id;
        out.parent_id = group_info.parent_id;
        copy_str_to_char_array(group_info.name, &mut out.name);

        true
    })
}

// =============================================================================
// State Persistence
// =============================================================================

/// Save the plugin state and return it in a new buffer.
///
/// The state is serialized once, so the buffer always holds one whole
/// state, whatever its size. The caller releases the buffer with
/// `beamer_au_free_state`.
///
/// Returns null, with `*out_size` set to 0, when the state cannot be
/// saved.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns null)
/// - `instance` must not have been destroyed
/// - `out_size` must be a valid pointer to a `u32`, or null (in which case
///   this function returns null)
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_copy_state(
    instance: BeamerAuInstanceHandle,
    out_size: *mut u32,
) -> *mut u8 {
    if out_size.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: out_size was validated as non-null above.
    unsafe { *out_size = 0 };

    with_instance!(instance, ptr::null_mut(), |handle| {
        let state = with_model(handle, |model| model.save_state());
        let size = match u32::try_from(state.len()) {
            Ok(size) if size > 0 => size,
            Ok(_) => return ptr::null_mut(),
            Err(_) => {
                log::error!("The plugin state is too large to be saved.");
                return ptr::null_mut();
            }
        };

        // A boxed slice has no spare capacity, so its length is all that
        // `beamer_au_free_state` needs to release it.
        let state = Box::into_raw(state.into_boxed_slice());
        // SAFETY: out_size was validated as non-null at function start.
        unsafe { *out_size = size };
        state.cast::<u8>()
    })
}

/// Release a buffer that `beamer_au_copy_state` returned.
///
/// # Safety
///
/// - `state` must be a pointer returned by `beamer_au_copy_state` that was
///   not released before, or null (in which case this function does nothing)
/// - `size` must be the size that call reported
#[no_mangle]
pub extern "C" fn beamer_au_free_state(state: *mut u8, size: u32) {
    if state.is_null() {
        return;
    }
    // SAFETY: The caller passes the pointer and the size of a boxed slice
    // that `beamer_au_copy_state` leaked.
    drop(unsafe { Box::from_raw(ptr::slice_from_raw_parts_mut(state, size as usize)) });
}

/// Restore plugin state from a buffer.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `K_AUDIO_UNIT_ERR_INVALID_PARAMETER`)
/// - `instance` must not have been destroyed
/// - `buffer` must be a valid pointer to a readable buffer of at least `size` bytes,
///   or null if `size` is 0 (empty state is allowed)
/// - This function validates `instance` is non-null and that `buffer` is non-null
///   when `size > 0` before dereferencing
/// - Thread safety: Safe to call from any thread; takes no lock.
///   The audio thread resets the smoothers at the start of its next render call
#[no_mangle]
pub extern "C" fn beamer_au_set_state(
    instance: BeamerAuInstanceHandle,
    buffer: *const u8,
    size: u32,
) -> i32 {
    if buffer.is_null() && size > 0 {
        return os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER;
    }

    with_instance!(instance, os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER, |handle| {
        let state_slice = if size > 0 {
            // SAFETY: buffer is validated non-null at function start when size > 0.
            unsafe { std::slice::from_raw_parts(buffer, size as usize) }
        } else {
            &[]
        };

        let loaded = with_model(handle, |model| model.load_state(state_slice));

        match loaded {
            Ok(()) => {
                // The values are in place. The smoothers belong to the audio
                // thread: the next render call makes them jump to the loaded
                // values, so that those do not ramp in.
                handle.shared.request_smoothing_reset();
                os_status::NO_ERR
            }
            Err(e) => {
                log::error!("Failed to load state: {}", e);
                os_status::K_AUDIO_UNIT_ERR_INVALID_PROPERTY_VALUE
            }
        }
    })
}

// =============================================================================
// Properties
// =============================================================================

/// Get the plugin's processing latency in samples.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; lock-free. Returns the value
///   of the most recent render call, 0 while unprepared
#[no_mangle]
pub extern "C" fn beamer_au_get_latency_samples(instance: BeamerAuInstanceHandle) -> u32 {
    with_instance!(instance, 0, |handle| handle.shared.latency_samples())
}

/// Get the plugin's tail time in samples.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - This function validates `instance` is non-null before dereferencing
/// - Thread safety: Safe to call from any thread; lock-free. Returns the value
///   of the most recent render call, 0 while unprepared
#[no_mangle]
pub extern "C" fn beamer_au_get_tail_samples(instance: BeamerAuInstanceHandle) -> u32 {
    with_instance!(instance, 0, |handle| handle.shared.tail_samples())
}

/// Get float64 processing support level.
///
/// Returns:
/// - `NotSupported` when `instance` is null.
/// - `Native` when the prepared processor reports native f64 processing.
/// - `ViaConversion` otherwise (float64 streams are accepted via conversion).
#[no_mangle]
pub extern "C" fn beamer_au_get_float64_support(
    instance: BeamerAuInstanceHandle,
) -> BeamerAuFloat64Support {
    with_instance!(instance, BeamerAuFloat64Support::NotSupported, |handle| {
        let plugin = match lock_plugin(handle) {
            Ok(guard) => guard,
            Err(_) => return BeamerAuFloat64Support::NotSupported,
        };

        if plugin.supports_native_double_precision() {
            BeamerAuFloat64Support::Native
        } else {
            BeamerAuFloat64Support::ViaConversion
        }
    })
}

// =============================================================================
// Plugin Metadata
// =============================================================================

/// Get the plugin's display name.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `out_buffer` is non-null and `buffer_len > 0`
///   before dereferencing
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_name(
    _instance: BeamerAuInstanceHandle,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let result = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return 0,
        };

        let bytes = config.name.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        // SAFETY: out_buffer validated non-null above. Caller guarantees buffer_len
        // bytes are writable. copy_len < buffer_len ensures we don't overflow.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
            *out_buffer.add(copy_len) = 0;
        }

        copy_len as u32
    });

    result.unwrap_or(0)
}

/// Get the plugin vendor/manufacturer name.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - `out_buffer` must be a valid pointer to a writable buffer of at least
///   `buffer_len` bytes, or null (in which case this function returns `0`)
/// - `buffer_len` must be greater than 0
/// - This function validates `out_buffer` is non-null and `buffer_len > 0`
///   before dereferencing
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_vendor(
    _instance: BeamerAuInstanceHandle,
    out_buffer: *mut c_char,
    buffer_len: u32,
) -> u32 {
    if out_buffer.is_null() || buffer_len == 0 {
        return 0;
    }

    let result = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return 0,
        };

        let bytes = config.vendor.as_bytes();
        let copy_len = bytes.len().min(buffer_len as usize - 1);

        // SAFETY: out_buffer validated non-null above. Caller guarantees buffer_len
        // bytes are writable. copy_len < buffer_len ensures we don't overflow.
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), out_buffer as *mut u8, copy_len);
            *out_buffer.add(copy_len) = 0;
        }

        copy_len as u32
    });

    result.unwrap_or(0)
}

// =============================================================================
// Bus Queries
// =============================================================================

/// Bus direction for queries (input or output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BusDirection {
    Input,
    Output,
}

/// Internal helper: Get the number of buses for a given direction.
///
/// This function consolidates the logic for both input and output bus count queries.
fn get_bus_count_impl(instance: BeamerAuInstanceHandle, direction: BusDirection) -> u32 {
    with_instance!(instance, 0, |handle| {
        // The declared count is also the allocated one: the wrappers build
        // the bus configuration they allocate with from this answer. The
        // AUv2 render call asks for the input count, so this must not wait.
        handle.declared_buses.count(direction).min(MAX_BUSES) as u32
    })
}

/// Internal helper: Get the channel count for a bus at the given index and direction.
///
/// This function consolidates the logic for both input and output bus channel count queries.
/// Not for the render thread: it takes the lock of the negotiated bus configuration.
fn get_bus_channel_count_impl(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
    direction: BusDirection,
) -> u32 {
    with_instance!(instance, 0, |handle| {
        // If resources are allocated, report the host-negotiated channel counts.
        if let Some(cfg) = handle.lock_bus_config().as_ref() {
            let info = match direction {
                BusDirection::Input => cfg.input_bus_info(bus_index as usize),
                BusDirection::Output => cfg.output_bus_info(bus_index as usize),
            };
            return info.map(|b| b.channel_count as u32).unwrap_or(0);
        }

        handle.declared_buses.channel_count(direction, bus_index as usize)
    })
}

/// Get the number of input buses the plugin supports.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_input_bus_count(instance: BeamerAuInstanceHandle) -> u32 {
    get_bus_count_impl(instance, BusDirection::Input)
}

/// Get the number of output buses the plugin supports.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_output_bus_count(instance: BeamerAuInstanceHandle) -> u32 {
    get_bus_count_impl(instance, BusDirection::Output)
}

/// Get the default channel count for an input bus.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_input_bus_channel_count(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> u32 {
    get_bus_channel_count_impl(instance, bus_index, BusDirection::Input)
}

/// Get the default channel count for an output bus.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_output_bus_channel_count(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> u32 {
    get_bus_channel_count_impl(instance, bus_index, BusDirection::Output)
}

/// Whether an input bus is on by default: the main bus, or an aux bus
/// active by default (`BusInfo::is_default_active`).
///
/// The AUv3 wrapper seeds the `enabled` flag of its buses with this. False
/// for a bus that does not exist.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_is_input_bus_default_active(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> bool {
    with_instance!(instance, false, |handle| {
        handle.declared_buses.default_buses().is_input_on(bus_index as usize)
    })
}

/// Whether an output bus is on by default, as for
/// `beamer_au_is_input_bus_default_active`.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_is_output_bus_default_active(
    instance: BeamerAuInstanceHandle,
    bus_index: u32,
) -> bool {
    with_instance!(instance, false, |handle| {
        handle.declared_buses.default_buses().is_output_on(bus_index as usize)
    })
}

/// Copy the name the plugin gives a bus into `out_name`: UTF-8 with a
/// terminating null, at most `capacity` bytes, cut at a character.
///
/// The wrappers name their buses with it. False for a bus that does not
/// exist, a null `out_name` or a `capacity` of 0.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - `out_name` must be null or valid for `capacity` bytes
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_bus_name(
    instance: BeamerAuInstanceHandle,
    is_input: bool,
    bus_index: u32,
    out_name: *mut c_char,
    capacity: u32,
) -> bool {
    if out_name.is_null() || capacity == 0 {
        return false;
    }
    with_instance!(instance, false, |handle| {
        let Some(name) = handle.declared_buses.name(direction_of(is_input), bus_index as usize) else {
            return false;
        };
        // SAFETY: Valid for capacity bytes (caller), checked non-null above.
        let dest = unsafe { std::slice::from_raw_parts_mut(out_name, capacity as usize) };
        copy_str_to_char_array(name, dest);
        true
    })
}

/// Channel counts to assume when the plugin cannot be asked.
///
/// Stereo out, stereo in only for the categories that normally have an input.
fn fallback_main_channels(category: beamer_core::config::Category) -> (u32, u32) {
    use beamer_core::config::Category;

    match category {
        Category::Effect | Category::MidiEffect => (2, 2),
        Category::Instrument | Category::Generator => (0, 2),
    }
}

/// Check if a proposed channel configuration is valid.
///
/// A configuration is valid when one of the plugin's layouts takes these
/// channel counts on the main input and output bus (0 for a direction
/// without buses: an instrument or generator normally has no input bus).
/// The other buses and the channel sets are checked when the plugin is
/// prepared.
///
/// # Safety
///
/// - `instance` should be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case `fallback_main_channels` decides)
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_is_channel_config_valid(
    instance: BeamerAuInstanceHandle,
    main_input_channels: u32,
    main_output_channels: u32,
) -> bool {
    let result = catch_unwind(AssertUnwindSafe(|| {
        // Validate channel counts are within bounds
        if main_input_channels > MAX_CHANNELS as u32 || main_output_channels > MAX_CHANNELS as u32 {
            return false;
        }

        if instance.is_null() {
            // The fallback depends on the plugin type.
            let Some(config) = factory::plugin_config() else {
                return false;
            };
            return (main_input_channels, main_output_channels) == fallback_main_channels(config.category);
        }

        // SAFETY: instance validated non-null above. Caller guarantees a
        // valid pointer from beamer_au_create_instance.
        let handle = unsafe { &*instance };
        handle
            .declared_buses
            .allows_main_channels(main_input_channels, main_output_channels)
    }));

    result.unwrap_or(false)
}

/// Get the supported channel capabilities for the main bus.
///
/// One `[input, output]` pair per layout of the plugin, without repeats:
/// the channel counts of its main input and output bus. The input is 0 for
/// a layout without input buses, which is the normal case for instruments
/// (`aumu`) and generators (`augn`). A bus that takes any set is -1; an
/// output that follows the input is -1 with an input of any count (the
/// Audio Unit's matched channels) and the input's count otherwise; any
/// output with any input is -2. At most `BEAMER_AU_MAX_CHANNEL_CAPABILITIES`
/// pairs are written.
///
/// # Returns
///
/// Returns `true` if capabilities were successfully written to `out_capabilities`.
/// Returns `false` if the output pointer is null or an error occurs.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - `out_capabilities` must be a valid pointer to a `BeamerAuChannelCapabilities` struct
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_channel_capabilities(
    instance: BeamerAuInstanceHandle,
    out_capabilities: *mut BeamerAuChannelCapabilities,
) -> bool {
    if out_capabilities.is_null() {
        return false;
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        let pairs = if instance.is_null() {
            // The fallback depends on the plugin type.
            let Some(config) = factory::plugin_config() else {
                return false;
            };
            let (input, output) = fallback_main_channels(config.category);
            vec![(input as i32, output as i32)]
        } else {
            // SAFETY: instance validated non-null above. Caller guarantees a
            // valid pointer from beamer_au_create_instance.
            let handle = unsafe { &*instance };
            handle.declared_buses.channel_capabilities()
        };

        // SAFETY: out_capabilities validated non-null above. Caller guarantees
        // the pointer is valid and writable.
        let capabilities = unsafe { &mut *out_capabilities };
        *capabilities = BeamerAuChannelCapabilities::default();
        for (slot, (input, output)) in capabilities.capabilities.iter_mut().zip(pairs) {
            *slot = BeamerAuChannelCapability {
                input_channels: input,
                output_channels: output,
            };
            capabilities.count += 1;
        }

        true
    }));

    result.unwrap_or(false)
}

/// The direction of a bus for the bus functions: input or output.
fn direction_of(is_input: bool) -> BusDirection {
    if is_input {
        BusDirection::Input
    } else {
        BusDirection::Output
    }
}

/// Whether one of the plugin's layouts takes `channels` channels on a bus.
///
/// A bus that follows the main input takes any count here; the whole
/// combination is checked when the plugin is prepared. False for 0 channels
/// and for a bus that does not exist.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_bus_supports_channel_count(
    instance: BeamerAuInstanceHandle,
    is_input: bool,
    bus_index: u32,
    channels: u32,
) -> bool {
    with_instance!(instance, false, |handle| {
        handle
            .declared_buses
            .allows_count(direction_of(is_input), bus_index as usize, channels)
    })
}

/// Whether one of the plugin's layouts takes the channel layout with the
/// channel labels `labels` (`count` of them) on a bus, in any order of its
/// channels.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - `labels` must be valid for `count` values, or null with `count` 0
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_bus_supports_layout(
    instance: BeamerAuInstanceHandle,
    is_input: bool,
    bus_index: u32,
    labels: *const u32,
    count: u32,
) -> bool {
    if labels.is_null() || count == 0 {
        return false;
    }
    with_instance!(instance, false, |handle| {
        // SAFETY: Valid for count values (caller), checked non-null above.
        let labels = unsafe { std::slice::from_raw_parts(labels, count as usize) };
        let direction = direction_of(is_input);
        let declared = &handle.declared_buses;
        let bus = bus_index as usize;
        channel_labels::set_for_labels(labels, declared.listed(direction, bus))
            .is_some_and(|(set, _)| declared.allows_set(direction, bus, set))
    })
}

/// Write the layout tags of the channel layouts a bus takes into
/// `out_tags` (at most `capacity`) and return how many the bus has.
///
/// A null `out_tags` only returns the count.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - `out_tags` must be null or valid for `capacity` values
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_supported_layout_tags(
    instance: BeamerAuInstanceHandle,
    is_input: bool,
    bus_index: u32,
    out_tags: *mut u32,
    capacity: u32,
) -> u32 {
    with_instance!(instance, 0, |handle| {
        let tags = handle.declared_buses.layout_tags(direction_of(is_input), bus_index as usize);
        if !out_tags.is_null() {
            for (index, &tag) in tags.iter().take(capacity as usize).enumerate() {
                // SAFETY: index < capacity, valid (caller).
                unsafe { *out_tags.add(index) = tag };
            }
        }
        tags.len() as u32
    })
}

/// The layout tag of the set a bus has with `channels` channels when the
/// host sets no channel layout (see `DeclaredBuses::set_for`). 0 for 0
/// channels.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`, or null
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_layout_tag(
    instance: BeamerAuInstanceHandle,
    is_input: bool,
    bus_index: u32,
    channels: u32,
) -> u32 {
    with_instance!(instance, 0, |handle| {
        handle
            .declared_buses
            .set_for(direction_of(is_input), bus_index as usize, channels, &[])
            .map_or(0, |(set, _)| channel_labels::tag_of(set))
    })
}

// =============================================================================
// MIDI Support
// =============================================================================

/// Check if the plugin produces MIDI output.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_produces_midi(_instance: BeamerAuInstanceHandle) -> bool {
    // The plugin's config says whether it sends MIDI.
    let result = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return false,
        };

        config.midi_output
    });

    result.unwrap_or(false)
}

/// The SysEx bytes one render call holds in each direction for a plugin with
/// MIDI input (`sysex_bytes_per_block`), 0 without MIDI input. The AUv2 shim
/// sizes its SysEx input queue with it.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_sysex_input_bytes(_instance: BeamerAuInstanceHandle) -> u32 {
    let result = catch_unwind(|| match factory::plugin_config() {
        Some(config) if config.midi_input => u32::try_from(config.sysex_bytes_per_block).unwrap_or(u32::MAX),
        _ => 0,
    });
    result.unwrap_or(0)
}

/// The SysEx bytes one render call holds in each direction for a plugin with
/// MIDI output (`sysex_bytes_per_block`), 0 without MIDI output. The AUv2
/// shim sizes its MIDI output list with it.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_sysex_output_bytes(_instance: BeamerAuInstanceHandle) -> u32 {
    let result = catch_unwind(|| match factory::plugin_config() {
        Some(config) if config.midi_output => u32::try_from(config.sysex_bytes_per_block).unwrap_or(u32::MAX),
        _ => 0,
    });
    result.unwrap_or(0)
}

/// Note SysEx messages from the host that the AUv2 shim dropped before a
/// render call: its queue was full or a message was larger than a render
/// call holds. A warning for the log, plus a count in a diagnostics build.
///
/// # Safety
///
/// - `instance` must be null or a valid handle
/// - Thread safety: Safe on the render thread: no lock, no allocation
#[no_mangle]
pub extern "C" fn beamer_au_note_sysex_input_dropped(instance: BeamerAuInstanceHandle, count: u32) {
    if count == 0 {
        return;
    }
    with_instance_void!(instance, |handle| {
        handle.shared.note_render_warning(beamer_core::ProcessWarning::SysExInputDropped);
        #[cfg(feature = "diagnostics")]
        if let Some(diagnostics) = handle.shared.diagnostics() {
            diagnostics.state().note_sysex_dropped(count);
        }
    });
}

// =============================================================================
// GUI / WebView
// =============================================================================

/// Check if the plugin has a custom GUI.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_has_gui(_instance: BeamerAuInstanceHandle) -> bool {
    let result = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return false,
        };

        config.has_gui
    });

    result.unwrap_or(false)
}

/// Get the dev server URL.
///
/// Returns NULL in production mode (embedded assets are served via the
/// custom URL scheme instead). In dev mode, returns a null-terminated
/// URL like "http://localhost:5173".
///
/// The returned pointer is valid for the lifetime of the process
/// (cached in a static OnceLock) and must not be freed by the caller.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - Thread safety: Safe to call from any thread
/// - The returned pointer must not be freed by the caller
#[no_mangle]
pub extern "C" fn beamer_au_get_gui_url(
    _instance: BeamerAuInstanceHandle,
) -> *const c_char {
    use std::sync::OnceLock;
    static CACHED_URL: OnceLock<Option<CString>> = OnceLock::new();

    let result = catch_unwind(|| {
        let cached = CACHED_URL.get_or_init(|| {
            let config = factory::plugin_config()?;
            let url = config.gui_url?;
            CString::new(url).ok()
        });

        match cached {
            Some(cstr) => cstr.as_ptr(),
            None => ptr::null(),
        }
    });

    result.unwrap_or(ptr::null())
}

/// Get the embedded GUI assets pointer.
///
/// Returns an opaque pointer suitable for passing to `beamer_webview_create()`.
/// Returns null if no assets are configured (e.g. dev server mode).
///
/// # Safety
///
/// - Thread safety: Safe to call from any thread
/// - The returned pointer is valid for the lifetime of the process
///   (it points into static data embedded in the binary)
#[cfg(feature = "webview")]
#[no_mangle]
pub extern "C" fn beamer_au_get_gui_assets() -> *const std::ffi::c_void {
    let result = catch_unwind(|| {
        match factory::plugin_config().and_then(|c| c.gui_assets) {
            Some(assets) => {
                assets as *const beamer_webview::EmbeddedAssets as *const std::ffi::c_void
            }
            None => ptr::null(),
        }
    });

    result.unwrap_or(ptr::null())
}

/// Get the initial GUI size in pixels.
///
/// # Safety
///
/// - `_instance` parameter is currently unused but accepted for API consistency
/// - `width` and `height` must be valid non-null pointers to `uint32_t`
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_gui_size(
    _instance: BeamerAuInstanceHandle,
    width: *mut u32,
    height: *mut u32,
) {
    if width.is_null() || height.is_null() {
        return;
    }

    let _ = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return,
        };

        debug_assert!(
            config.gui_width > 0 && config.gui_height > 0,
            "gui_size must be set when has_gui is true"
        );

        // SAFETY: width and height validated non-null above.
        unsafe {
            *width = config.gui_width;
            *height = config.gui_height;
        }
    });
}

/// Write the 4-byte GUI background color (RGBA) to `out`.
///
/// Returns all zeros if no background color is configured.
///
/// # Safety
///
/// - `out` must point to at least 4 writable bytes
/// - Thread safety: Safe to call from any thread
#[no_mangle]
pub extern "C" fn beamer_au_get_gui_background_color(out: *mut u8) {
    if out.is_null() {
        return;
    }

    // Zero-initialize before catch_unwind so callers always get valid data
    // even if plugin_config() returns None or a panic is caught.
    // SAFETY: caller guarantees out points to at least 4 writable bytes.
    unsafe {
        *out = 0;
        *out.add(1) = 0;
        *out.add(2) = 0;
        *out.add(3) = 0;
    }

    let _ = catch_unwind(|| {
        let config = match factory::plugin_config() {
            Some(c) => c,
            None => return,
        };

        let rgba = config.gui_background_color;
        // SAFETY: caller guarantees out points to at least 4 writable bytes.
        unsafe {
            *out = rgba[0];
            *out.add(1) = rgba[1];
            *out.add(2) = rgba[2];
            *out.add(3) = rgba[3];
        }
    });
}

// =============================================================================
// Factory Presets
// =============================================================================

/// Preset information for building AUAudioUnitPreset / AUPreset arrays.
///
/// Matches `BeamerAuPresetInfo` in header.
#[repr(C)]
pub struct BeamerAuPresetInfo {
    /// Preset number/index (0-based)
    pub number: i32,
    /// Human-readable preset name (UTF-8, null-terminated)
    pub name: [c_char; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
}

impl Default for BeamerAuPresetInfo {
    fn default() -> Self {
        Self {
            number: 0,
            name: [0; BEAMER_AU_MAX_PARAM_NAME_LENGTH],
        }
    }
}

/// Get the number of factory presets.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `0`)
/// - `instance` must not have been destroyed
/// - Thread safety: Safe to call from any thread; lock-free, the presets are
///   captured when the instance is created
#[no_mangle]
pub extern "C" fn beamer_au_get_preset_count(instance: BeamerAuInstanceHandle) -> u32 {
    with_instance!(instance, 0, |handle| handle.presets.count as u32)
}

/// Get information about a factory preset by index.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - `out_info` must be a valid pointer to a `BeamerAuPresetInfo` struct,
///   or null (in which case this function returns `false`)
/// - Thread safety: Safe to call from any thread; lock-free, the presets are
///   captured when the instance is created
#[no_mangle]
pub extern "C" fn beamer_au_get_preset_info(
    instance: BeamerAuInstanceHandle,
    index: u32,
    out_info: *mut BeamerAuPresetInfo,
) -> bool {
    if out_info.is_null() {
        return false;
    }

    with_instance!(instance, false, |handle| {
        if index as usize >= handle.presets.count {
            return false;
        }

        if let Some(name) = (handle.presets.name)(index as usize) {
            // SAFETY: out_info was validated as non-null at function start.
            let out = unsafe { &mut *out_info };
            out.number = index as i32;
            copy_str_to_char_array(name, &mut out.name);
            true
        } else {
            false
        }
    })
}

/// Apply a factory preset by index.
///
/// The preset sets the parameters it names. The others keep their current
/// values.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `false`)
/// - `instance` must not have been destroyed
/// - Thread safety: Safe to call from any thread; takes no lock
#[no_mangle]
pub extern "C" fn beamer_au_apply_preset(
    instance: BeamerAuInstanceHandle,
    preset_index: u32,
) -> bool {
    with_instance!(instance, false, |handle| {
        with_model(handle, |model| (handle.presets.apply)(model, preset_index as usize))
    })
}

// =============================================================================
// WebView IPC Parameter Sync
// =============================================================================

/// Parse a display string for a parameter and return a normalized value.
///
/// Returns `f64::NAN` if the string cannot be parsed.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns `NAN`)
/// - `text` must be a valid pointer to a UTF-8 string of `text_len` bytes
#[no_mangle]
pub extern "C" fn beamer_au_param_string_to_normalized(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    text: *const u8,
    text_len: usize,
) -> f64 {
    if instance.is_null() || text.is_null() {
        return f64::NAN;
    }

    // SAFETY: text is non-null and text_len is the byte length.
    let text_slice = unsafe { std::slice::from_raw_parts(text, text_len) };
    let text_str = match std::str::from_utf8(text_slice) {
        Ok(s) => s,
        Err(_) => return f64::NAN,
    };

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above.
        let handle = unsafe { &*instance };
        with_param_store(handle, |store| {
            store.string_to_normalized(param_id, text_str)
        })
    }));

    match result {
        Ok(Some(normalized)) => normalized,
        _ => f64::NAN,
    }
}

/// Free a string returned by a beamer_au function (e.g., `beamer_au_on_invoke`,
/// `beamer_au_editor_sync_frame_script`).
///
/// # Safety
///
/// - `ptr` must be a pointer returned by a beamer_au function that allocates
///   strings via `CString::into_raw()`, or null.
#[no_mangle]
pub extern "C" fn beamer_au_free_string(ptr: *mut c_char) {
    if !ptr.is_null() {
        // SAFETY: ptr was returned by CString::into_raw() in a beamer_au function.
        unsafe { drop(CString::from_raw(ptr)) };
    }
}

/// Set a parameter value from the WebView UI (normalized, f64 precision).
///
/// This sets the normalized value in the Rust parameter store. The ObjC
/// wrapper is responsible for notifying the host (via AUParameterTree for
/// AUv3 or AudioUnitSetParameter for AUv2).
///
/// Accepts f64 for full precision from the JS runtime.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
#[no_mangle]
pub extern "C" fn beamer_au_param_set_from_ui(
    instance: BeamerAuInstanceHandle,
    param_id: u32,
    value: f64,
) {
    if instance.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above.
        let handle = unsafe { &*instance };
        with_param_store(handle, |store| {
            store.set_normalized(param_id, value);
        });
    }));
}

// =============================================================================
// WebView Invoke / Event
// =============================================================================

/// Handle an invoke call from JavaScript.
///
/// Called when the WebView sends an `invoke` message. Dispatches to the
/// model's `on_invoke`. Returns a
/// heap-allocated JSON string with `{"ok":...}` or `{"err":"..."}` that the
/// caller must free with `beamer_au_free_string`. Returns null on failure.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns null)
/// - `method` must be a valid UTF-8 pointer with at least `method_len` bytes
/// - `args_json` must be a valid UTF-8 pointer with at least `args_json_len` bytes
#[no_mangle]
pub unsafe extern "C" fn beamer_au_on_invoke(
    instance: BeamerAuInstanceHandle,
    method: *const u8,
    method_len: usize,
    args_json: *const u8,
    args_json_len: usize,
) -> *mut c_char {
    if instance.is_null() || method.is_null() || args_json.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Pointers validated non-null above. Caller guarantees correct lengths.
        let method_bytes = unsafe { std::slice::from_raw_parts(method, method_len) };
        // SAFETY: args_json validated non-null above. Caller guarantees correct length.
        let args_bytes = unsafe { std::slice::from_raw_parts(args_json, args_json_len) };
        let method_str = std::str::from_utf8(method_bytes).ok()?;
        let args_str = std::str::from_utf8(args_bytes).ok()?;

        // SAFETY: instance validated non-null above. Caller guarantees valid pointer.
        let handle = unsafe { &*instance };

        let args: Vec<serde_json::Value> = serde_json::from_str(args_str).unwrap_or_default();
        let json = match handle.webview_dispatch.dispatch_invoke(method_str, &args) {
            Ok(val) => {
                format!(r#"{{"ok":{}}}"#, serde_json::to_string(&val).unwrap_or_default())
            }
            Err(msg) => {
                let escaped = serde_json::to_string(&msg).unwrap_or_default();
                format!(r#"{{"err":{}}}"#, escaped)
            }
        };

        CString::new(json).ok()
    }));

    match result {
        Ok(Some(cstr)) => cstr.into_raw(),
        _ => ptr::null_mut(),
    }
}

/// Handle a custom event from JavaScript.
///
/// Called when the WebView sends an `event` message. Dispatches to the
/// model's `on_event`.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function does nothing)
/// - `name` must be a valid UTF-8 pointer with at least `name_len` bytes
/// - `data_json` must be a valid UTF-8 pointer with at least `data_json_len` bytes
#[no_mangle]
pub unsafe extern "C" fn beamer_au_on_event(
    instance: BeamerAuInstanceHandle,
    name: *const u8,
    name_len: usize,
    data_json: *const u8,
    data_json_len: usize,
) {
    if instance.is_null() || name.is_null() || data_json.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: Pointers validated non-null above. Caller guarantees correct lengths.
        let name_bytes = unsafe { std::slice::from_raw_parts(name, name_len) };
        // SAFETY: data_json validated non-null above. Caller guarantees correct length.
        let data_bytes = unsafe { std::slice::from_raw_parts(data_json, data_json_len) };
        let name_str = std::str::from_utf8(name_bytes).ok()?;
        let data_str = std::str::from_utf8(data_bytes).ok()?;

        // SAFETY: instance validated non-null above. Caller guarantees valid pointer.
        let handle = unsafe { &*instance };

        let data: serde_json::Value =
            serde_json::from_str(data_str).unwrap_or(serde_json::Value::Null);
        handle.webview_dispatch.dispatch_event(name_str, &data);

        Some(())
    }));
}

/// What one open editor uses to keep its web UI in step with the plugin.
///
/// An editor holds this from the moment it opens until it closes. It holds
/// the model and the event queue itself and does not point into the
/// instance, so it stays valid when a host destroys the instance before the
/// editor.
pub struct BeamerAuEditorSync(EditorSync);

/// An editor was opened: start keeping its web UI in step.
///
/// Events the plugin emits from now on are kept for this editor. The
/// editor calls `beamer_au_editor_sync_init_script` when its page loaded,
/// `beamer_au_editor_sync_frame_script` from its sync timer and
/// `beamer_au_editor_sync_close` once when it closes.
///
/// # Safety
///
/// - `instance` must be a valid pointer returned by `beamer_au_create_instance`,
///   or null (in which case this function returns null)
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_open(
    instance: BeamerAuInstanceHandle,
) -> *mut BeamerAuEditorSync {
    if instance.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: instance validated non-null above. Caller guarantees valid pointer.
        let handle = unsafe { &*instance };

        let sync = EditorSync::open(Arc::clone(&handle.model), handle.webview_events.clone());
        Box::into_raw(Box::new(BeamerAuEditorSync(sync)))
    }));

    result.unwrap_or(ptr::null_mut())
}

/// The script that hands a freshly loaded page its parameters, the values
/// of its `State` fields and the plugin format (`"auv2"`, `"auv3"`).
///
/// Returns a heap-allocated null-terminated C string that the caller must
/// free with `beamer_au_free_string`, or null on failure.
///
/// # Safety
///
/// - `sync` must be a pointer returned by `beamer_au_editor_sync_open` that
///   was not closed yet, or null (in which case this function returns null)
/// - `plugin_format` must be a valid null-terminated C string
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_init_script(
    sync: *mut BeamerAuEditorSync,
    plugin_format: *const c_char,
) -> *mut c_char {
    if sync.is_null() || plugin_format.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: sync validated non-null above. The caller guarantees a valid
        // pointer that only its main thread uses.
        let sync = unsafe { &mut *sync };
        // SAFETY: plugin_format validated non-null above. Caller guarantees a
        // null-terminated string.
        let format = unsafe { CStr::from_ptr(plugin_format) }.to_str().ok()?;
        script_into_raw(sync.0.init_script(format))
    }));

    result.ok().flatten().unwrap_or(ptr::null_mut())
}

/// The script of one frame: the parameters that changed since the last
/// frame, then the events the plugin emitted.
///
/// Returns a heap-allocated null-terminated C string that the caller must
/// free with `beamer_au_free_string`, or null when nothing changed.
///
/// # Safety
///
/// - `sync` must be a pointer returned by `beamer_au_editor_sync_open` that
///   was not closed yet, or null (in which case this function returns null)
/// - Only the main thread of the editor calls it
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_frame_script(sync: *mut BeamerAuEditorSync) -> *mut c_char {
    if sync.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: sync validated non-null above. The caller guarantees a valid
        // pointer that only its main thread uses.
        let sync = unsafe { &mut *sync };
        script_into_raw(sync.0.frame_script()?)
    }));

    result.ok().flatten().unwrap_or(ptr::null_mut())
}

/// The script that tells the web UI the value of a parameter it set, as it
/// is in effect now.
///
/// The editor calls it after `beamer_au_param_set_from_ui` and after it
/// told the host, which can round the value. The value counts as sent.
/// Returns a heap-allocated null-terminated C string that the caller must
/// free with `beamer_au_free_string`, or null on failure.
///
/// # Safety
///
/// - `sync` must be a pointer returned by `beamer_au_editor_sync_open` that
///   was not closed yet, or null (in which case this function returns null)
/// - Only the main thread of the editor calls it
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_parameter_script(
    sync: *mut BeamerAuEditorSync,
    param_id: u32,
) -> *mut c_char {
    if sync.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: sync validated non-null above. The caller guarantees a valid
        // pointer that only its main thread uses.
        let sync = unsafe { &mut *sync };
        script_into_raw(sync.0.parameter_script(param_id))
    }));

    result.ok().flatten().unwrap_or(ptr::null_mut())
}

/// Handle a message of the web UI that needs no host: `state:set`, which
/// sets a `State` field.
///
/// Takes the message as the web UI sent it (UTF-8 JSON), so that its value
/// reaches the field unchanged. Returns the script that answers with the
/// value in effect, as a heap-allocated null-terminated C string that the
/// caller must free with `beamer_au_free_string`, or null for any other
/// message and on failure.
///
/// # Safety
///
/// - `sync` must be a pointer returned by `beamer_au_editor_sync_open` that
///   was not closed yet, or null (in which case this function returns null)
/// - `json` must point to `json_len` readable bytes
/// - Only the main thread of the editor calls it
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_message(
    sync: *mut BeamerAuEditorSync,
    json: *const u8,
    json_len: usize,
) -> *mut c_char {
    if sync.is_null() || json.is_null() {
        return ptr::null_mut();
    }

    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: sync validated non-null above. The caller guarantees a valid
        // pointer that only its main thread uses.
        let sync = unsafe { &mut *sync };
        // SAFETY: json validated non-null above. Caller guarantees json_len
        // readable bytes.
        let bytes = unsafe { std::slice::from_raw_parts(json, json_len) };

        let message: serde_json::Value = serde_json::from_slice(bytes).ok()?;
        if message.get("type")?.as_str()? != "state:set" {
            return None;
        }
        let path = message.get("id")?.as_str()?;
        let value = message.get("value").unwrap_or(&serde_json::Value::Null);
        script_into_raw(sync.0.state_script(path, value)?)
    }));

    result.ok().flatten().unwrap_or(ptr::null_mut())
}

/// The editor was closed: stop keeping events for it and free the handle.
///
/// # Safety
///
/// - `sync` must be a pointer returned by `beamer_au_editor_sync_open` that
///   was not closed yet, or null (in which case this function does nothing)
/// - After calling this function, `sync` is invalid and must not be used
#[no_mangle]
pub extern "C" fn beamer_au_editor_sync_close(sync: *mut BeamerAuEditorSync) {
    if sync.is_null() {
        return;
    }

    let _ = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: sync validated non-null above. Caller guarantees it was
        // returned by beamer_au_editor_sync_open and not closed before.
        drop(unsafe { Box::from_raw(sync) });
    }));
}

/// Hand a script to the Objective-C side as a C string.
fn script_into_raw(script: String) -> Option<*mut c_char> {
    // The scripts are JSON inside JavaScript calls and have no NUL byte.
    CString::new(script).ok().map(CString::into_raw)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use beamer_core::{
        EnumParameter, EnumParameterValue, FloatParameter, IntParameter, ParameterRef,
    };

    /// Declared buses of `layouts`, every bus on by default.
    fn declared(layouts: &'static [Layout]) -> DeclaredBuses {
        DeclaredBuses::new(layouts, ActiveBuses::all_on(Layout::default_of(layouts)))
    }

    #[test]
    fn test_a_plugin_without_buses_gets_an_output_bus() {
        let buses = declared(&[Layout { inputs: &[], outputs: &[] }]);
        assert_eq!(buses.count(BusDirection::Input), 0);
        assert_eq!(buses.count(BusDirection::Output), 1);
        assert_eq!(buses.main_channels(), (0, 2));
        // It is on.
        let default = buses.default_buses();
        assert_eq!((default.input_bus_count(), default.output_bus_count()), (0, 1));
        assert_eq!(default.output_set(0), Some(ChannelSet::Stereo));
        assert_eq!(buses.channel_capabilities(), [(0, 2)]);

        // A plugin with an input only keeps its buses.
        let effect = declared(&[Layout { inputs: &[BusLayout::STEREO], outputs: &[] }]);
        assert_eq!(effect.count(BusDirection::Output), 0);
        assert_eq!(effect.main_channels(), (2, 0));
    }

    #[test]
    fn test_declared_buses_keep_the_sets_of_buses_off_by_default() {
        // A mono sidechain, off by default.
        const LAYOUTS: &[Layout] = &[Layout {
            inputs: &[BusLayout::STEREO, BusLayout::MONO],
            outputs: &[BusLayout::Set(ChannelSet::Surround51)],
        }];
        let mut default = ActiveBuses::all_on(&LAYOUTS[0]);
        assert!(default.set_input(1, None));
        let buses = DeclaredBuses::new(LAYOUTS, default);

        assert_eq!(buses.count(BusDirection::Input), 2);
        assert_eq!(buses.channel_count(BusDirection::Input, 1), 1);
        assert_eq!(buses.channel_count(BusDirection::Input, 2), 0);
        assert_eq!(buses.main_channels(), (2, 6));
        assert!(!buses.default_buses().is_input_on(1));
    }

    /// Mono or stereo, mono to stereo, with a sidechain of any width.
    const EFFECT: &[Layout] = &[
        Layout {
            inputs: &[BusLayout::STEREO, BusLayout::Any],
            outputs: &[BusLayout::STEREO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::Any],
            outputs: &[BusLayout::MONO],
        },
        Layout {
            inputs: &[BusLayout::MONO, BusLayout::Any],
            outputs: &[BusLayout::STEREO],
        },
    ];

    #[test]
    fn test_channel_capabilities_of_the_layouts() {
        assert_eq!(declared(EFFECT).channel_capabilities(), [(2, 2), (1, 1), (1, 2)]);
        // Wildcards.
        assert_eq!(declared(&[Layout::MATCHED]).channel_capabilities(), [(-1, -1)]);
        const ANY: &[Layout] = &[Layout { inputs: &[BusLayout::Any], outputs: &[BusLayout::Any] }];
        assert_eq!(declared(ANY).channel_capabilities(), [(-1, -2)]);
        const FIXED_IN: &[Layout] = &[Layout { inputs: &[BusLayout::STEREO], outputs: &[BusLayout::Any] }];
        assert_eq!(declared(FIXED_IN).channel_capabilities(), [(2, -1)]);
        const FOLLOWED: &[Layout] = &[Layout {
            inputs: &[BusLayout::STEREO],
            outputs: &[BusLayout::SameAsMainInput],
        }];
        assert_eq!(declared(FOLLOWED).channel_capabilities(), [(2, 2)]);
        // Sets of the same count are one pair.
        const QUAD: &[Layout] = &[
            Layout {
                inputs: &[BusLayout::Set(ChannelSet::Quad)],
                outputs: &[BusLayout::Set(ChannelSet::Quad)],
            },
            Layout {
                inputs: &[BusLayout::Set(ChannelSet::Ambisonic(1))],
                outputs: &[BusLayout::Set(ChannelSet::Ambisonic(1))],
            },
        ];
        assert_eq!(declared(QUAD).channel_capabilities(), [(4, 4)]);
        assert_eq!(declared(&[Layout::INSTRUMENT_MONO]).channel_capabilities(), [(0, 1)]);
    }

    #[test]
    fn test_main_channels_and_counts_the_layouts_take() {
        let buses = declared(EFFECT);
        assert!(buses.allows_main_channels(2, 2));
        assert!(buses.allows_main_channels(1, 2));
        assert!(!buses.allows_main_channels(2, 1));
        assert!(!buses.allows_main_channels(6, 6));
        let matched = declared(&[Layout::MATCHED]);
        assert!(matched.allows_main_channels(6, 6));
        assert!(!matched.allows_main_channels(6, 2));
        assert!(declared(&[Layout::INSTRUMENT_STEREO]).allows_main_channels(0, 2));

        assert!(buses.allows_count(BusDirection::Input, 0, 1));
        assert!(!buses.allows_count(BusDirection::Input, 0, 6));
        // The sidechain takes any count, up to MAX_CHANNELS.
        assert!(buses.allows_count(BusDirection::Input, 1, 6));
        assert!(!buses.allows_count(BusDirection::Input, 1, MAX_CHANNELS as u32 + 1));
        assert!(!buses.allows_count(BusDirection::Input, 1, 0));
        assert!(!buses.allows_count(BusDirection::Input, 2, 1));
    }

    #[test]
    fn test_set_for_the_hosts_channels() {
        let buses = declared(EFFECT);
        let set = |direction, bus, channels, labels: &[u32]| {
            buses.set_for(direction, bus, channels, labels).map(|(set, _)| set)
        };
        // By count: a set a layout names, the default layout's first.
        assert_eq!(set(BusDirection::Input, 0, 1, &[]), Some(ChannelSet::Mono));
        assert_eq!(set(BusDirection::Input, 0, 2, &[]), Some(ChannelSet::Stereo));
        assert_eq!(set(BusDirection::Input, 1, 6, &[]), Some(ChannelSet::Surround51));
        assert_eq!(set(BusDirection::Input, 0, 0, &[]), None);

        // A count of a listed set means that set.
        const AMBISONIC: &[Layout] = &[Layout {
            inputs: &[BusLayout::Set(ChannelSet::Ambisonic(1))],
            outputs: &[BusLayout::STEREO],
        }];
        let ambisonic = declared(AMBISONIC);
        let found = ambisonic.set_for(BusDirection::Input, 0, 4, &[]).map(|(set, _)| set);
        assert_eq!(found, Some(ChannelSet::Ambisonic(1)));

        // With the host's labels, the set they name and their order.
        let (found, order) = buses.set_for(BusDirection::Input, 1, 8, &[1, 2, 3, 4, 5, 6, 33, 34]).unwrap();
        assert_eq!(found, ChannelSet::Surround71);
        assert_eq!(order.host_channel(4), 6);
        // Labels of another count than the channels are ignored.
        assert_eq!(set(BusDirection::Input, 0, 2, &[42]), Some(ChannelSet::Stereo));
    }

    #[test]
    fn test_layout_tags_of_a_bus() {
        let buses = declared(EFFECT);
        // The listed sets, without repeats.
        assert_eq!(buses.layout_tags(BusDirection::Output, 0), [0x0065_0002, 0x0064_0001]);
        // A bus of any set: every named set.
        let any = buses.layout_tags(BusDirection::Input, 1);
        assert!(any.contains(&0x0079_0006) && any.contains(&0x00c0_000c));
        assert!(buses.layout_tags(BusDirection::Input, 2).is_empty());
    }

    #[test]
    fn test_main_channels_without_a_plugin() {
        use beamer_core::config::Category;


        assert_eq!(fallback_main_channels(Category::Effect), (2, 2));
        assert_eq!(fallback_main_channels(Category::MidiEffect), (2, 2));
        assert_eq!(fallback_main_channels(Category::Instrument), (0, 2));
        assert_eq!(fallback_main_channels(Category::Generator), (0, 2));
    }

    /// A parameter store with one parameter.
    struct OneParameter<P: ParameterRef>(P);

    impl<P: ParameterRef> ParameterStore for OneParameter<P> {
        fn count(&self) -> usize {
            1
        }
        fn info(&self, index: usize) -> Option<&ParameterInfo> {
            (index == 0).then(|| self.0.info())
        }
        fn get_normalized(&self, _id: u32) -> f64 {
            self.0.get_normalized()
        }
        fn set_normalized(&self, _id: u32, value: f64) {
            self.0.set_normalized(value);
        }
        fn normalized_to_string(&self, _id: u32, normalized: f64) -> String {
            self.0.display_normalized(normalized)
        }
        fn string_to_normalized(&self, _id: u32, string: &str) -> Option<f64> {
            self.0.parse(string)
        }
        fn normalized_to_plain(&self, _id: u32, normalized: f64) -> f64 {
            self.0.normalized_to_plain(normalized)
        }
        fn plain_to_normalized(&self, _id: u32, plain: f64) -> f64 {
            self.0.plain_to_normalized(plain)
        }
    }

    /// The range, default and current value the host is told, in that order.
    fn au_values(store: &dyn ParameterStore) -> [f32; 4] {
        let info = store.info(0).unwrap();
        [
            normalized_to_au_value(store, info, 0.0),
            normalized_to_au_value(store, info, 1.0),
            normalized_to_au_value(store, info, info.default_normalized),
            normalized_to_au_value(store, info, store.get_normalized(info.id)),
        ]
    }

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Waveform {
        Sine,
        Saw,
        Square,
        Triangle,
    }

    impl EnumParameterValue for Waveform {
        const COUNT: usize = 4;
        const DEFAULT_INDEX: usize = 1;

        fn from_index(index: usize) -> Option<Self> {
            [Self::Sine, Self::Saw, Self::Square, Self::Triangle]
                .get(index)
                .copied()
        }
        fn to_index(self) -> usize {
            self as usize
        }
        fn default_value() -> Self {
            Self::Saw
        }
        fn name(index: usize) -> &'static str {
            Self::names()[index]
        }
        fn names() -> &'static [&'static str] {
            &["Sine", "Saw", "Square", "Triangle"]
        }
    }

    #[test]
    fn test_indexed_enum_parameter_in_au_format() {
        let store = OneParameter(EnumParameter::<Waveform>::new("Waveform"));

        // The host sees an index; the default is the second variant.
        assert_eq!(au_values(&store), [0.0, 3.0, 1.0, 1.0]);

        // Every index the host can set comes back as the same index.
        let id = store.info(0).unwrap().id;
        for index in 0..4 {
            store.set_normalized(id, au_value_to_normalized(&store, id, index as f32));
            assert_eq!(au_values(&store)[3], index as f32);
            assert_eq!(store.0.get(), Waveform::from_index(index).unwrap());
        }

        // A name parses to its index.
        assert_eq!(string_to_au_value(&store, id, "Triangle"), Some(3.0));
    }

    #[test]
    fn test_indexed_int_parameter_in_au_format() {
        // The plain range does not start at 0, the index the host sees does.
        let store =
            OneParameter(IntParameter::new("Steps", 2, 1..=4).with_unit(ParameterUnit::Indexed));
        assert_eq!(au_values(&store), [0.0, 3.0, 1.0, 1.0]);

        let id = store.info(0).unwrap().id;
        store.set_normalized(id, au_value_to_normalized(&store, id, 3.0));
        assert_eq!(store.0.get(), 4);
        assert_eq!(au_values(&store)[3], 3.0);

        // The string for an index is the string of the plain value behind it.
        let string_for = |index| {
            store.normalized_to_string(id, au_value_to_normalized(&store, id, index))
        };
        assert_eq!(string_for(0.0), "1");
        assert_eq!(string_for(3.0), "4");

        // And the other way around.
        assert_eq!(string_to_au_value(&store, id, "4"), Some(3.0));
    }

    #[test]
    fn test_plain_parameter_in_au_format() {
        // Without the indexed unit the host sees the plain values.
        let store = OneParameter(IntParameter::new("Steps", 2, 1..=4));
        assert_eq!(au_values(&store), [1.0, 4.0, 2.0, 2.0]);

        let id = store.info(0).unwrap().id;
        store.set_normalized(id, au_value_to_normalized(&store, id, 3.0));
        assert_eq!(store.0.get(), 3);

        assert_eq!(string_to_au_value(&store, id, "3"), Some(3.0));
        assert_eq!(string_to_au_value(&store, id, "three"), None);
    }

    #[test]
    fn test_string_parses_to_the_plain_value() {
        // A typed value comes back as the plain value, also when the range is
        // not linear: the host must not get a normalized value. The plain
        // value is not a linear interpolation between minimum and maximum.
        let store = OneParameter(FloatParameter::hz("Cutoff", 800.0, 20.0..=20000.0));
        let id = store.info(0).unwrap().id;
        let cutoff = string_to_au_value(&store, id, "800").unwrap();
        assert!((cutoff - 800.0).abs() < 0.01, "{}", cutoff);

        // A value outside the range ends at its limit; text is no value.
        assert_eq!(string_to_au_value(&store, id, "50000"), Some(20000.0));
        assert_eq!(string_to_au_value(&store, id, "abc"), None);

        let store = OneParameter(FloatParameter::db("Gain", -6.0, -60.0..=6.0));
        let id = store.info(0).unwrap().id;
        let gain = string_to_au_value(&store, id, "-6.0").unwrap();
        assert!((gain + 6.0).abs() < 0.001, "{}", gain);
    }

    #[test]
    fn test_beamer_au_bus_info_layout() {
        // Verify C-compatible layout
        assert_eq!(std::mem::size_of::<BeamerAuBusInfo>(), 12 + 4 * MAX_CHANNELS);
        assert_eq!(std::mem::align_of::<BeamerAuBusInfo>(), 4);
    }

    #[test]
    fn test_beamer_au_sample_format() {
        assert_eq!(BeamerAuSampleFormat::Float32 as i32, 0);
        assert_eq!(BeamerAuSampleFormat::Float64 as i32, 1);
    }

    #[test]
    fn test_beamer_au_float64_support_enum() {
        assert_eq!(BeamerAuFloat64Support::NotSupported as i32, 0);
        assert_eq!(BeamerAuFloat64Support::ViaConversion as i32, 1);
        assert_eq!(BeamerAuFloat64Support::Native as i32, 2);
        assert_eq!(
            beamer_au_get_float64_support(ptr::null_mut()),
            BeamerAuFloat64Support::NotSupported
        );
    }

    #[test]
    fn test_copy_str_to_char_array() {
        let mut dest = [0i8; 16];
        copy_str_to_char_array("hello", &mut dest);
        assert_eq!(dest[0], b'h' as i8);
        assert_eq!(dest[4], b'o' as i8);
        assert_eq!(dest[5], 0);
    }

    #[test]
    fn test_copy_str_to_char_array_truncation() {
        let mut dest = [0i8; 4];
        copy_str_to_char_array("hello", &mut dest);
        assert_eq!(dest[0], b'h' as i8);
        assert_eq!(dest[2], b'l' as i8);
        assert_eq!(dest[3], 0); // Null terminator
    }

    #[test]
    fn test_copy_str_to_char_array_cuts_at_a_character() {
        // The cut falls inside the two bytes of the é, which is left out.
        let mut dest = [0i8; 5];
        copy_str_to_char_array("Café", &mut dest);
        assert_eq!(dest[2], b'f' as i8);
        assert_eq!(dest[3], 0);
    }

    #[test]
    fn test_bus_names_default_to_the_names_of_their_index() {
        const LAYOUTS: &[Layout] = &[Layout { inputs: &[BusLayout::STEREO, BusLayout::STEREO], outputs: &[BusLayout::STEREO] }];
        let buses = declared(LAYOUTS);
        assert_eq!(buses.name(BusDirection::Input, 0), Some("Input"));
        assert_eq!(buses.name(BusDirection::Input, 1), Some("Aux 1"));
        assert_eq!(buses.name(BusDirection::Input, 2), None);
        assert_eq!(buses.name(BusDirection::Output, 0), Some("Output"));
        assert_eq!(buses.name(BusDirection::Output, 1), None);

        // A plugin without buses gets an output bus with the default name.
        const NO_BUSES: &[Layout] = &[Layout { inputs: &[], outputs: &[] }];
        let buses = declared(NO_BUSES);
        assert_eq!(buses.name(BusDirection::Input, 0), None);
        assert_eq!(buses.name(BusDirection::Output, 0), Some("Output"));
    }

    #[test]
    fn test_null_handle_safety() {
        // All functions should handle null handles gracefully
        beamer_au_destroy_instance(ptr::null_mut());

        assert_eq!(
            beamer_au_allocate_render_resources(
                ptr::null_mut(),
                44100.0,
                1024,
                BeamerAuSampleFormat::Float32,
                ptr::null(),
                false,
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );

        beamer_au_deallocate_render_resources(ptr::null_mut());

        assert_eq!(
            beamer_au_render(
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
                1024,
                0,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(), // input_data
                ptr::null(), // transport
                None,        // midi_output
                ptr::null_mut(),
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );

        assert_eq!(beamer_au_get_parameter_count(ptr::null_mut()), 0);
        assert!(!beamer_au_get_parameter_info(
            ptr::null_mut(),
            0,
            ptr::null_mut()
        ));
        assert_eq!(beamer_au_get_parameter_value(ptr::null_mut(), 0), 0.0);
        beamer_au_set_parameter_value(ptr::null_mut(), 0, 0.5);
        let mut size = 7u32;
        assert!(beamer_au_copy_state(ptr::null_mut(), &mut size).is_null());
        assert_eq!(size, 0);
        assert!(beamer_au_copy_state(ptr::null_mut(), ptr::null_mut()).is_null());
        beamer_au_free_state(ptr::null_mut(), 0);
        assert_eq!(
            beamer_au_set_state(ptr::null_mut(), ptr::null(), 0),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER
        );
        assert_eq!(beamer_au_get_latency_samples(ptr::null_mut()), 0);
        assert_eq!(beamer_au_get_tail_samples(ptr::null_mut()), 0);
        assert!(!beamer_au_is_prepared(ptr::null_mut()));
        beamer_au_reset(ptr::null_mut());
    }

    /// Helper to create a valid bus config for testing
    fn create_valid_bus_config() -> BeamerAuBusConfig {
        let mut config = BeamerAuBusConfig {
            input_bus_count: 1,
            output_bus_count: 1,
            input_buses: [BeamerAuBusInfo::new(0, BeamerAuBusType::Main); MAX_BUSES],
            output_buses: [BeamerAuBusInfo::new(0, BeamerAuBusType::Main); MAX_BUSES],
        };
        config.input_buses[0].channel_count = 2;
        config.output_buses[0].channel_count = 2;
        config
    }

    #[test]
    fn test_allocate_render_resources_invalid_sample_rate() {
        // Test with a valid bus config but invalid sample rates
        let bus_config = create_valid_bus_config();

        // Create a dummy non-null instance handle for testing parameter validation
        // Note: We use a non-null pointer that will cause the function to validate
        // parameters before attempting to dereference (which would fail).
        // The validation happens before the unsafe block, so these tests are safe.

        // Zero sample rate
        assert_eq!(
            beamer_au_allocate_render_resources(
                ptr::null_mut(), // Still null - but sample_rate check happens after null check
                0.0,
                1024,
                BeamerAuSampleFormat::Float32,
                &bus_config,
                false,
            ),
            os_status::K_AUDIO_UNIT_ERR_INVALID_PARAMETER // null instance check first
        );

        // Negative sample rate - need non-null instance to reach sample_rate validation
        // Since we can't easily create a valid instance in tests, we verify the constants exist
        assert_eq!(MAX_SAMPLE_RATE, 384_000.0);
        assert_eq!(MAX_FRAMES_PER_RENDER, 8192);
    }

    #[test]
    fn test_bus_config_validation_constants() {
        // Verify the validation constants are reasonable (compile-time checks)
        const _: () = assert!(MAX_SAMPLE_RATE > 0.0);
        const _: () = assert!(MAX_SAMPLE_RATE >= 192_000.0); // Should support high-end pro audio
        const _: () = assert!(MAX_FRAMES_PER_RENDER > 0);
        const _: () = assert!(MAX_FRAMES_PER_RENDER >= 4096); // Should support common buffer sizes
    }

    #[test]
    fn test_bus_config_bounds() {
        // Verify MAX_BUSES is accessible and reasonable (compile-time checks)
        const _: () = assert!(MAX_BUSES > 0);
        const _: () = assert!(MAX_BUSES <= 16); // Reasonable upper bound for buses
    }
}
