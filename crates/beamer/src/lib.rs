//! # Beamer
//!
//! Audio Plugin Framework for Rust.
//!
//! Beamer is a framework for building audio plugins (AU, VST3).
//! It provides safe Rust abstractions that work with multiple plugin formats.
//!
//! ## Quick Start
//!
//! ```rust,ignore
//! use beamer::prelude::*;
//!
//! // Define your plugin
//! struct MyGain { parameters: MyParameters }
//!
//! impl Processor for MyGain {
//!     fn setup(&mut self, _: f64, _: usize) {}
//!     fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, _context: &ProcessContext) {
//!         // Your DSP here
//!     }
//! }
//!
//! impl Descriptor for MyGain {
//!     type Parameters = MyParameters;
//!     fn parameters(&self) -> &Self::Parameters { &self.parameters }
//!     fn create() -> Self { Self { parameters: MyParameters::new() } }
//! }
//!
//! // Export
//! static CONFIG: Config = Config::new("MyGain", Category::Effect, "Mfgr", "gain")
//!     .with_vendor("My Company");
//!
//! export_plugin!(CONFIG, MyGain);
//! ```

// Re-export sub-crates
pub use beamer_core as core;

/// Plugin setup types for declaring host information requirements.
///
/// See [`beamer_core::setup`] for documentation and examples.
pub use beamer_core::setup;

#[cfg(feature = "vst3")]
pub use beamer_vst3 as vst3_impl;

/// Re-export of vst3 types needed for plugin configuration.
///
/// This allows examples and plugins to use `beamer::vst3::Steinberg::TUID`
/// without adding a direct dependency on the vst3 crate.
#[cfg(feature = "vst3")]
pub mod vst3 {
    pub use ::vst3::Steinberg;
    pub use ::vst3::Steinberg::TUID;
}

#[cfg(feature = "au")]
pub use beamer_au as au_impl;

// Re-export derive macros when feature is enabled
#[cfg(feature = "derive")]
pub use beamer_macros::Parameters;
#[cfg(feature = "derive")]
pub use beamer_macros::EnumParameter;
#[cfg(feature = "derive")]
pub use beamer_macros::HasParameters;
#[cfg(feature = "derive")]
pub use beamer_macros::export;

/// Generate plugin entry points for all enabled formats (AU, VST3).
///
/// This is the primary export macro for Beamer plugins. It generates the
/// necessary entry points for whichever plugin formats are enabled via
/// Cargo features.
///
/// # Arguments
///
/// * `$config` - A static reference to [`Config`](beamer_core::Config) containing all plugin metadata
/// * `$plugin` - The plugin type implementing the [`Descriptor`](beamer_core::Descriptor) trait
/// * `$presets` - (Optional) The presets type implementing [`FactoryPresets`](beamer_core::FactoryPresets)
///
/// # Example
///
/// ```rust,ignore
/// use beamer::prelude::*;
///
/// pub static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
///     .with_vendor("My Company")
///     .with_version(env!("CARGO_PKG_VERSION"));
///
/// export_plugin!(CONFIG, MyPlugin);
///
/// // With presets:
/// export_plugin!(CONFIG, MyPlugin, MyPresets);
/// ```
#[macro_export]
macro_rules! export_plugin {
    // With explicit presets type
    ($config:expr, $plugin:ty, $presets:ty) => {
        // === AU entry points ===
        #[cfg(feature = "au")]
        fn __beamer_au_do_register() {
            $crate::au_impl::factory::register_factory(
                || {
                    Box::new($crate::au_impl::AuProcessor::<$plugin, $presets>::new())
                        as Box<dyn $crate::au_impl::AuPluginInstance>
                },
                &$config,
            );
        }

        #[cfg(feature = "au")]
        #[used]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        static __BEAMER_AU_INIT: extern "C" fn() = {
            extern "C" fn __beamer_au_register() {
                __beamer_au_do_register();
            }
            __beamer_au_register
        };

        #[cfg(feature = "au")]
        #[doc(hidden)]
        pub fn __beamer_au_manual_init() {
            __beamer_au_do_register();
        }

        // === VST3 entry points ===
        #[cfg(all(feature = "vst3", target_os = "windows"))]
        #[no_mangle]
        extern "system" fn InitDll() -> bool {
            true
        }

        #[cfg(all(feature = "vst3", target_os = "windows"))]
        #[no_mangle]
        extern "system" fn ExitDll() -> bool {
            true
        }

        #[cfg(all(feature = "vst3", target_os = "macos"))]
        #[no_mangle]
        extern "system" fn bundleEntry(_bundle_ref: *mut std::ffi::c_void) -> bool {
            true
        }

        #[cfg(all(feature = "vst3", target_os = "macos"))]
        #[no_mangle]
        extern "system" fn bundleExit() -> bool {
            true
        }

        #[cfg(feature = "vst3")]
        #[no_mangle]
        extern "system" fn GetPluginFactory() -> *mut std::ffi::c_void {
            use $crate::vst3_impl::vst3::ComWrapper;
            use $crate::vst3_impl::Factory;

            let factory = Factory::<$crate::vst3_impl::Vst3Processor<$plugin, $presets>>::new(&$config);
            let wrapper = ComWrapper::new(factory);

            wrapper
                .to_com_ptr::<$crate::vst3_impl::vst3::Steinberg::IPluginFactory>()
                .unwrap()
                .into_raw() as *mut std::ffi::c_void
        }
    };

    // Without presets (default to NoPresets)
    ($config:expr, $plugin:ty) => {
        $crate::export_plugin!(
            $config,
            $plugin,
            $crate::core::NoPresets<<$plugin as $crate::core::HasParameters>::Parameters>
        );
    };
}

/// Prelude module for convenient imports.
///
/// Import everything you need to build a plugin:
/// ```rust,ignore
/// use beamer::prelude::*;
/// ```
pub mod prelude {
    // Core traits and types
    pub use beamer_core::{
        // Buffer types
        AuxiliaryBuffers, AuxInput, AuxOutput, Buffer,
        // Bypass handling
        BypassAction, BypassHandler, BypassState, CrossfadeCurve,
        // Sample trait for generic f32/f64 processing
        Sample,
        // Traits
        Descriptor, GuiDelegate, HasParameters, Processor,
        // Plugin setup types (composable)
        PluginSetup, SampleRate, MaxBufferSize, MainInputChannels, MainOutputChannels,
        AuxInputCount, AuxOutputCount, ProcessMode,
        // Bus configuration
        BusInfo, BusType,
        // GUI types
        GuiConstraints, NoGui,
        // Parameter metadata
        NoParameters, ParameterFlags, ParameterInfo,
        // Factory presets
        FactoryPresets, NoPresets, PresetInfo, PresetValue,
        // Parameter types
        BoolParameter, EnumParameter, EnumParameterValue, FloatParameter, IntParameter, Formatter, ParameterRef, Parameters,
        // MIDI CC configuration (framework manages runtime state)
        MidiCcConfig,
        // Parameter smoothing
        Smoother, SmoothingStyle,
        // Parameter group system
        GroupId, GroupInfo, ParameterGroups, ROOT_GROUP_ID,
        // Range mapping
        LinearMapper, LogMapper, LogOffsetMapper, PowerMapper, RangeMapper,
        // Error types
        PluginError, PluginResult,
        // Geometry
        Rect, Size,
        // MIDI types
        ChannelPressure, ControlChange, MidiBuffer, MidiChannel, MidiEvent, MidiEventKind,
        MidiNote, NoteId, NoteOff, NoteOn, PitchBend, PolyPressure, ProgramChange,
        // Process context and transport
        FrameRate, ProcessContext, Transport,
        // FourCharCode
        FourCharCode,
    };

    // Plugin configuration
    pub use beamer_core::{Config, config::Category, config::Subcategory};

    // Unified export macro
    pub use crate::export_plugin;

    // Derive macros for parameters (when feature enabled)
    // These share names with the traits/types they implement, which is allowed
    // because traits and derive macros live in different namespaces.
    #[cfg(feature = "derive")]
    pub use beamer_macros::{EnumParameter, HasParameters, Parameters};
}
