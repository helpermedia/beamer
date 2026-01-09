//! Export macro for Audio Unit plugins.
//!
//! This module provides the `export_au!` macro that generates the necessary
//! entry points and registration code for Audio Unit plugins.

/// Generate Audio Unit export entry points.
///
/// This macro sets up:
/// 1. A module initializer that registers the plugin factory at load time
/// 2. The `BeamerAudioUnitFactory` C function that macOS uses to instantiate the AU
///
/// The factory function name matches the `factoryFunction` key in Info.plist.
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
///     fourcc!(b"Demo"),
///     fourcc!(b"mypg"),
/// );
///
/// export_au!(CONFIG, AU_CONFIG, MyPlugin);
/// ```
///
/// # Generated Code
///
/// The macro generates:
/// 1. `__BEAMER_AU_INIT` - Module initializer that registers factory at load time
/// 2. `BeamerAudioUnitFactory` - C factory function called by macOS to create AU instances
/// 3. `__beamer_au_manual_init` - Manual initialization for testing
#[macro_export]
macro_rules! export_au {
    ($config:expr, $au_config:expr, $plugin:ty) => {
        // Module initializer to register factory
        // This uses the __mod_init_func section on macOS to run at load time
        #[used]
        #[cfg_attr(target_os = "macos", link_section = "__DATA,__mod_init_func")]
        static __BEAMER_AU_INIT: extern "C" fn() = {
            extern "C" fn __beamer_au_register() {
                // Register the plugin factory
                $crate::factory::register_factory(
                    || {
                        Box::new($crate::AuProcessor::<$plugin>::new())
                            as Box<dyn $crate::AuPluginInstance>
                    },
                    &$config,
                    &$au_config,
                );

                // Register the BeamerAudioUnit class
                $crate::audio_unit::register_class();
            }
            __beamer_au_register
        };

        /// Audio Unit factory function called by macOS to create plugin instances.
        ///
        /// This is the entry point that macOS uses to instantiate the AU. The function
        /// name must match the `factoryFunction` key in the Info.plist AudioComponents
        /// array.
        ///
        /// # Safety
        ///
        /// This function is called by the macOS Audio Unit host with a valid
        /// AudioComponentDescription pointer. The returned AUAudioUnit is retained
        /// and ownership is transferred to the caller.
        #[no_mangle]
        pub unsafe extern "C" fn BeamerAudioUnitFactory(
            desc: *const $crate::audio_unit::AudioComponentDescription,
        ) -> *mut std::ffi::c_void {
            $crate::create_audio_unit_instance(desc)
        }

        // Also provide a manual initialization function for testing
        #[doc(hidden)]
        pub fn __beamer_au_manual_init() {
            $crate::factory::register_factory(
                || {
                    Box::new($crate::AuProcessor::<$plugin>::new())
                        as Box<dyn $crate::AuPluginInstance>
                },
                &$config,
                &$au_config,
            );
            $crate::audio_unit::register_class();
        }
    };
}
