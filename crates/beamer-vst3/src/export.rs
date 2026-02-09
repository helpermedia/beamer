//! VST3 export macros and entry points.

/// Generate VST3 entry points for a plugin.
///
/// This macro generates the platform-specific entry points and the
/// `GetPluginFactory` function required by the VST3 host.
///
/// Uses combined component architecture where processor and controller
/// are implemented by the same object.
///
/// # Arguments
///
/// * `$config` - A static reference to [`beamer_core::Config`] containing all plugin metadata
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
/// export_vst3!(CONFIG, MyPlugin);
///
/// // With presets
/// export_vst3!(CONFIG, MyPlugin, MyPresets);
/// ```
///
/// [`FactoryPresets`]: beamer_core::FactoryPresets
#[macro_export]
macro_rules! export_vst3 {
    // With explicit presets type
    ($config:expr, $plugin:ty, $presets:ty) => {
        // Platform-specific entry points

        #[cfg(target_os = "windows")]
        #[no_mangle]
        extern "system" fn InitDll() -> bool {
            true
        }

        #[cfg(target_os = "windows")]
        #[no_mangle]
        extern "system" fn ExitDll() -> bool {
            true
        }

        // CRITICAL: Must be lowercase on macOS!
        #[cfg(target_os = "macos")]
        #[no_mangle]
        extern "system" fn bundleEntry(_bundle_ref: *mut std::ffi::c_void) -> bool {
            true
        }

        #[cfg(target_os = "macos")]
        #[no_mangle]
        extern "system" fn bundleExit() -> bool {
            true
        }

        // Plugin factory export
        #[no_mangle]
        extern "system" fn GetPluginFactory() -> *mut std::ffi::c_void {
            use $crate::vst3::ComWrapper;
            use $crate::Factory;

            let factory = Factory::<$crate::Vst3Processor<$plugin, $presets>>::new(&$config);
            let wrapper = ComWrapper::new(factory);

            wrapper
                .to_com_ptr::<$crate::vst3::Steinberg::IPluginFactory>()
                .unwrap()
                .into_raw() as *mut std::ffi::c_void
        }
    };

    // Without presets (default to NoPresets)
    ($config:expr, $plugin:ty) => {
        $crate::export_vst3!($config, $plugin, $crate::NoPresets<<$plugin as $crate::HasParameters>::Parameters>);
    };
}
