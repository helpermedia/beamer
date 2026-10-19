//! VST3 export macros and entry points.

/// Generate VST3 entry points for a plugin.
///
/// This macro generates the platform-specific entry points and the
/// `GetPluginFactory` function required by the VST3 host.
///
/// Uses combined component architecture where processor and controller
/// are implemented by the same object.
///
/// Plugins normally do not call this macro themselves: `beamer::export_plugin!`
/// calls it for plugins built with the `vst3` feature.
///
/// # Arguments
///
/// * `$config` - The [`beamer_core::Config`] static containing all plugin metadata
/// * `$plugin` - The model type implementing the [`beamer_core::Model`] trait,
///   with its factory presets in `Model::PRESETS`
///
/// # Example
///
/// ```rust,ignore
/// use beamer_core::{Config, config::Category};
///
/// static CONFIG: Config = Config::new("My Plugin", Category::Effect, "Mfgr", "plgn")
///     .with_vendor("My Company");
///
/// export_vst3!(CONFIG, MyPlugin);
/// ```
#[macro_export]
macro_rules! export_vst3 {
    ($config:expr, $plugin:ty) => {
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

            let factory = Factory::<$crate::Vst3Processor<$plugin>>::new(&$config);
            let wrapper = ComWrapper::new(factory);

            wrapper
                .to_com_ptr::<$crate::vst3::Steinberg::IPluginFactory>()
                .unwrap()
                .into_raw() as *mut std::ffi::c_void
        }
    };
}
