//! Export macro for Audio Unit plugins.
//!
//! This module provides the `export_au!` macro that generates the registration
//! code for Audio Units, for AUv2 `.component` bundles and AUv3 app extensions
//! alike.
//!
//! ## Hybrid Objective-C/Rust Architecture
//!
//! - **Native wrappers**: `cargo xtask bundle` generates them for each plugin. The AUv2 wrapper
//!   implements `AudioComponentPlugInInterface`, the AUv3 wrapper is an `AUAudioUnit` subclass
//!   (`Beamer<Name>AuWrapper_<codes>`)
//! - **Rust**: All DSP, parameters and plugin logic via C-ABI bridge functions

/// Generate Audio Unit export entry points.
///
/// This macro registers the plugin factory, enabling the AU host to instantiate
/// your plugin. It works in conjunction with the native wrappers that xtask
/// generates for the plugin, which implement the AUv2 and AUv3 APIs.
///
/// Plugins normally do not call this macro themselves: `beamer::export_plugin!`
/// calls it for plugins built with the `au` feature.
///
/// # Arguments
///
/// * `$config` - The [`beamer_core::Config`] static containing all plugin metadata
/// * `$plugin` - The plugin type implementing the [`beamer_core::Descriptor`] trait
/// * `$presets` - (Optional) The presets type implementing [`FactoryPresets`]. If omitted, `NoPresets` is used.
///
/// # Example
///
/// ```rust,ignore
/// use beamer_core::{Config, config::Category};
///
/// static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
///     .with_vendor("My Company");
///
/// // Without presets
/// export_au!(CONFIG, MyPlugin);
///
/// // With presets
/// export_au!(CONFIG, MyPlugin, MyPresets);
/// ```
///
/// # Generated Symbols
///
/// ## `__beamer_au_manual_init()` (public function)
///
/// A public function that registers the plugin factory. This is called automatically
/// via a `#[ctor]`-style initializer when the plugin library loads, but is also available
/// for explicit use in testing scenarios.
///
/// # How It Works
///
/// The plugin library loads with the `.component` bundle or the app extension. The
/// initialization flow is:
///
/// 1. Plugin library loads
/// 2. Static initializer runs, registering the factory via [`OnceLock`]
/// 3. Host asks the generated wrapper for an Audio Unit (AUv2: the component's factory
///    function and `Open`, AUv3: `createAudioUnitWithComponentDescription:error:` of the
///    extension or the extension's factory function)
/// 4. The wrapper calls `beamer_au_create_instance()` which uses the registered factory
/// 5. `AuProcessor<YourPlugin, Presets>::new()` creates the actual plugin instance
///
/// # Initialization Guarantees
///
/// - **Single registration**: The factory uses [`OnceLock`] storage, ensuring the factory
///   can only be registered once. Subsequent calls are no-ops (idempotent).
/// - **Thread safety**: [`OnceLock`] provides thread-safe initialization.
/// - **One plugin per binary**: Only one `export_au!` invocation is supported per binary.
///
/// [`OnceLock`]: std::sync::OnceLock
/// [`AuProcessor`]: crate::AuProcessor
/// [`FactoryPresets`]: beamer_core::FactoryPresets
#[macro_export]
macro_rules! export_au {
    // With explicit presets type
    ($config:expr, $plugin:ty, $presets:ty) => {
        // Factory registration function
        fn __beamer_au_do_register() {
            $crate::factory::register_factory(
                || {
                    Box::new($crate::AuProcessor::<$plugin, $presets>::new())
                        as Box<dyn $crate::AuPluginInstance>
                },
                &$config,
            );
        }

        // Static initializer using ctor crate pattern.
        // This runs when the plugin library loads.
        #[used]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        static __BEAMER_AU_INIT: extern "C" fn() = {
            extern "C" fn __beamer_au_register() {
                __beamer_au_do_register();
            }
            __beamer_au_register
        };

        /// Manual initialization function for testing.
        ///
        /// In test binaries, the `__mod_init_func` section may not be processed.
        /// Call this function explicitly to register the factory in tests.
        #[doc(hidden)]
        pub fn __beamer_au_manual_init() {
            __beamer_au_do_register();
        }
    };

    // Without presets (default to NoPresets)
    ($config:expr, $plugin:ty) => {
        $crate::export_au!($config, $plugin, $crate::NoPresets<<$plugin as $crate::HasParameters>::Parameters>);
    };
}
