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
/// * `$config` - A static reference to [`beamer_core::Config`] containing shared plugin metadata
/// * `$vst3_config` - A static reference to [`Vst3Config`] containing VST3-specific configuration
/// * `$plugin` - The plugin type implementing the [`beamer_core::Descriptor`] trait
/// * `$presets` - (Optional) The presets type implementing [`FactoryPresets`]. If omitted, `NoPresets` is used.
///
/// # Example
///
/// ```rust,ignore
/// use beamer_core::Config;
/// use beamer_vst3::{export_vst3, Vst3Config, vst3};
///
/// // Shared plugin configuration
/// static CONFIG: Config = Config::new("My Plugin")
///     .with_vendor("My Company");
///
/// // VST3-specific configuration
/// static VST3_CONFIG: Vst3Config = Vst3Config::new(
///     vst3::uid(0x12345678, 0x9ABCDEF0, 0xABCDEF12, 0x34567890),
/// );
///
/// // Without presets
/// export_vst3!(CONFIG, VST3_CONFIG, MyPlugin);
///
/// // With presets
/// export_vst3!(CONFIG, VST3_CONFIG, MyPlugin, MyPresets);
/// ```
///
/// [`Vst3Config`]: crate::Vst3Config
/// [`FactoryPresets`]: beamer_core::FactoryPresets
#[macro_export]
macro_rules! export_vst3 {
    // With explicit presets type
    ($config:expr, $vst3_config:expr, $plugin:ty, $presets:ty) => {
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

        #[cfg(target_os = "linux")]
        #[no_mangle]
        extern "system" fn ModuleEntry(_library_handle: *mut std::ffi::c_void) -> bool {
            true
        }

        #[cfg(target_os = "linux")]
        #[no_mangle]
        extern "system" fn ModuleExit() -> bool {
            true
        }

        // Plugin factory export
        #[no_mangle]
        extern "system" fn GetPluginFactory() -> *mut std::ffi::c_void {
            use $crate::vst3::ComWrapper;
            use $crate::Factory;

            let factory = Factory::<$crate::Vst3Processor<$plugin, $presets>>::new(&$config, &$vst3_config);
            let wrapper = ComWrapper::new(factory);

            wrapper
                .to_com_ptr::<$crate::vst3::Steinberg::IPluginFactory>()
                .unwrap()
                .into_raw() as *mut std::ffi::c_void
        }
    };

    // Without presets (default to NoPresets)
    ($config:expr, $vst3_config:expr, $plugin:ty) => {
        $crate::export_vst3!($config, $vst3_config, $plugin, $crate::NoPresets<<$plugin as $crate::HasParameters>::Parameters>);
    };
}
