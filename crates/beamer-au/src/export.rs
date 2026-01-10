//! Export macro for Audio Unit plugins.
//!
//! This module provides the `export_au!` macro that generates the necessary
//! entry points and registration code for Audio Unit plugins.
//!
//! ## Hybrid Architecture
//!
//! The AU wrapper uses a hybrid Objective-C/Rust architecture:
//! - **Objective-C**: Native `AUAudioUnit` subclass (`BeamerAuWrapper`) handles macOS integration
//! - **Rust**: All DSP, parameters, and plugin logic via C-ABI bridge functions
//!
//! The `BeamerAudioUnitFactory` function is implemented in native Objective-C
//! (`objc/BeamerAuWrapper.m`) and linked via the `cc` crate build script.

/// Generate Audio Unit export entry points.
///
/// This macro sets up a module initializer that registers the plugin factory at load time,
/// enabling the macOS Audio Unit host to instantiate your plugin. It works in conjunction
/// with the native Objective-C wrapper (`BeamerAuWrapper`) that implements the actual
/// `AUAudioUnit` subclass.
///
/// # Arguments
///
/// * `$config` - A static reference to [`beamer_core::PluginConfig`] containing shared plugin metadata
/// * `$au_config` - A static reference to [`AuConfig`] containing AU-specific configuration
/// * `$plugin` - The plugin type implementing the [`beamer_core::Plugin`] trait
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
/// export_au!(CONFIG, AU_CONFIG, MyPlugin);
/// ```
///
/// # Generated Symbols
///
/// The macro generates the following symbols:
///
/// ## `__BEAMER_AU_INIT` (static)
///
/// A function pointer placed in the `__DATA,__mod_init_func` section on macOS.
/// This causes the dynamic linker (`dyld`) to execute the function automatically
/// when the plugin bundle is loaded. The initializer registers the plugin factory
/// with the global factory registry, making it available for instance creation.
///
/// ## `__beamer_au_manual_init()` (public function)
///
/// A public function that performs the same registration as `__BEAMER_AU_INIT`.
/// This is primarily useful for testing scenarios where the automatic module
/// initializer may not run (e.g., in Rust test binaries that don't load the
/// plugin as a dynamic library).
///
/// # Platform-Specific Behavior
///
/// ## macOS (`__DATA,__mod_init_func` section)
///
/// On macOS, the `#[link_section = "__DATA,__mod_init_func"]` attribute places
/// the initializer function pointer in a special section that `dyld` processes
/// during library load. This is the standard mechanism for C/C++ `__attribute__((constructor))`
/// functions and ensures the factory is registered before any AU host APIs are called.
///
/// The initialization order is:
/// 1. Plugin bundle is loaded by the AU host
/// 2. `dyld` executes all `__mod_init_func` entries (including `__BEAMER_AU_INIT`)
/// 3. Factory is registered in global [`OnceLock`] storage
/// 4. Host calls `BeamerAudioUnitFactory` (Objective-C) to create instances
/// 5. `BeamerAudioUnitFactory` calls `beamer_au_create_instance()` which uses the registered factory
///
/// ## Other Platforms
///
/// This macro is macOS-only. The `#[cfg_attr(target_os = "macos", ...)]` ensures
/// the link section is only applied on macOS. On other platforms, the static is
/// still created (due to `#[used]`) but won't be automatically executed.
///
/// # Relationship with `BeamerAudioUnitFactory`
///
/// The actual AU factory function (`BeamerAudioUnitFactory`) is implemented in native
/// Objective-C (`objc/BeamerAuWrapper.m`) and linked via the `cc` build script. This
/// function is what the AU host calls (as specified in `Info.plist`'s `factoryFunction` key).
///
/// The data flow is:
///
/// ```text
/// AU Host
///    │
///    ▼
/// BeamerAudioUnitFactory (Objective-C)
///    │
///    ▼
/// beamer_au_create_instance() (Rust FFI, in bridge.rs)
///    │
///    ▼
/// factory::create_instance() (uses factory registered by this macro)
///    │
///    ▼
/// AuProcessor<YourPlugin>::new()
/// ```
///
/// # Initialization Guarantees and Potential Pitfalls
///
/// ## Guarantees
///
/// - **Single registration**: The factory uses [`OnceLock`] storage, ensuring the factory
///   can only be registered once. Subsequent calls panic, preventing accidental double-init.
/// - **Thread safety**: [`OnceLock`] provides thread-safe initialization. If multiple threads
///   somehow call the initializer simultaneously, only one will succeed.
/// - **Deterministic timing**: On macOS, `dyld` guarantees all `__mod_init_func` entries
///   run before `main()` or before any exported symbols are called.
///
/// ## Potential Pitfalls
///
/// - **One plugin per binary**: Only one `export_au!` invocation is supported per binary.
///   Attempting to register multiple plugins will panic on the second registration.
/// - **Static initialization order**: If your plugin's static configuration (`CONFIG`,
///   `AU_CONFIG`) depends on other statics, ensure those are initialized first. Using
///   `const` or simple static initialization avoids this issue.
/// - **Testing**: In test binaries, the `__mod_init_func` section may not be processed
///   by the Rust test runner. Use `__beamer_au_manual_init()` to register the factory
///   explicitly in tests.
/// - **Debug vs Release**: The initializer runs in both debug and release builds.
///   Ensure your plugin can be instantiated even if logging or other debug
///   infrastructure isn't fully initialized.
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
                // The ObjC BeamerAuWrapper calls beamer_au_create_instance() which uses this factory
                $crate::factory::register_factory(
                    || {
                        Box::new($crate::AuProcessor::<$plugin>::new())
                            as Box<dyn $crate::AuPluginInstance>
                    },
                    &$config,
                    &$au_config,
                );
            }
            __beamer_au_register
        };

        // Note: BeamerAudioUnitFactory is now implemented in native Objective-C
        // (objc/BeamerAuWrapper.m) and linked via the cc crate build script.
        // It calls beamer_au_create_instance() from the Rust bridge.

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
        }
    };
}
