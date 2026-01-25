//! Export macro for Audio Unit plugins.
//!
//! This module provides the `export_au!` macro that generates the necessary
//! entry points and registration code for Audio Unit `.component` bundles.
//!
//! ## Hybrid v2/v3 Architecture
//!
//! The AU wrapper uses AUv2-style `.component` bundles with a modern v3 `AUAudioUnit` internally:
//! - **AUv2 bundle**: Simple `.component` distribution, works with ad-hoc code signing
//! - **v3 AUAudioUnit**: Native `AUAudioUnit` subclass (`BeamerAuWrapper`) for modern API
//! - **Rust**: All DSP, parameters, and plugin logic via C-ABI bridge functions

/// Generate Audio Unit export entry points.
///
/// This macro registers the plugin factory, enabling the AU host to instantiate
/// your plugin. It works in conjunction with the native Objective-C wrapper
/// (`BeamerAuWrapper`) that implements the actual `AUAudioUnit` subclass.
///
/// # Arguments
///
/// * `$config` - A static reference to [`beamer_core::PluginConfig`] containing shared plugin metadata
/// * `$au_config` - A static reference to [`AuConfig`] containing AU-specific configuration
/// * `$plugin` - The plugin type implementing the [`beamer_core::Plugin`] trait
/// * `$presets` - (Optional) The presets type implementing [`FactoryPresets`]. If omitted, `NoPresets` is used.
///
/// # Example
///
/// ```rust,ignore
/// use beamer_core::PluginConfig;
/// use beamer_au::{export_au, AuConfig, ComponentType, fourcc};
///
/// static CONFIG: PluginConfig = PluginConfig::new("My Plugin")
///     .with_vendor("My Company");
///
/// static AU_CONFIG: AuConfig = AuConfig::new(
///     ComponentType::Effect,
///     fourcc!(b"Demo"),  // subtype
///     fourcc!(b"Mfgr"),  // manufacturer
/// );
///
/// // Without presets
/// export_au!(CONFIG, AU_CONFIG, MyPlugin);
///
/// // With presets
/// export_au!(CONFIG, AU_CONFIG, MyPlugin, MyPresets);
/// ```
///
/// # Generated Symbols
///
/// ## `__beamer_au_manual_init()` (public function)
///
/// A public function that registers the plugin factory. This is called automatically
/// via a `#[ctor]`-style initializer when the extension loads, but is also available
/// for explicit use in testing scenarios.
///
/// # How It Works
///
/// When the `.component` bundle loads, the initialization flow is:
///
/// 1. Component bundle loads
/// 2. Static initializer runs, registering the factory via [`OnceLock`]
/// 3. Host calls `+[BeamerAuWrapper createAudioUnitWithComponentDescription:error:]`
/// 4. That method calls `beamer_au_create_instance()` which uses the registered factory
/// 5. `AuProcessor<YourPlugin, Presets>::new()` creates the actual plugin instance
///
/// # Initialization Guarantees
///
/// - **Single registration**: The factory uses [`OnceLock`] storage, ensuring the factory
///   can only be registered once. Subsequent calls are no-ops (idempotent).
/// - **Thread safety**: [`OnceLock`] provides thread-safe initialization.
/// - **One plugin per binary**: Only one `export_au!` invocation is supported per binary.
///
/// # See Also
///
/// - [`factory::register_factory`] - The underlying registration function
/// - [`AuProcessor`] - The processor wrapper that bridges to your plugin
/// - [`beamer_au_create_instance`](crate::bridge::beamer_au_create_instance) - The FFI entry point
///
/// [`OnceLock`]: std::sync::OnceLock
/// [`AuConfig`]: crate::AuConfig
/// [`AuProcessor`]: crate::AuProcessor
/// [`FactoryPresets`]: beamer_core::FactoryPresets
#[macro_export]
macro_rules! export_au {
    // With explicit presets type
    ($config:expr, $au_config:expr, $plugin:ty, $presets:ty) => {
        // Factory registration function
        fn __beamer_au_do_register() {
            $crate::factory::register_factory(
                || {
                    Box::new($crate::AuProcessor::<$plugin, $presets>::new())
                        as Box<dyn $crate::AuPluginInstance>
                },
                &$config,
                &$au_config,
            );
        }

        // Static initializer using ctor crate pattern.
        // This runs when the .component/.appex bundle binary loads.
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
    ($config:expr, $au_config:expr, $plugin:ty) => {
        $crate::export_au!($config, $au_config, $plugin, $crate::NoPresets<<$plugin as $crate::HasParameters>::Parameters>);
    };
}
