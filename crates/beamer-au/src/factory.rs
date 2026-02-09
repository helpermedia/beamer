//! Plugin factory registration for Audio Unit.
//!
//! This module provides the factory registration system that enables the AU
//! runtime to create plugin instances. The factory is registered at module
//! initialization time via the `export_au!` macro.

use std::sync::OnceLock;

use beamer_core::Config;

use crate::instance::AuPluginInstance;

/// Factory function type for creating plugin instances.
pub type PluginFactory = fn() -> Box<dyn AuPluginInstance>;

/// Global factory storage (set by export_au! macro).
static PLUGIN_FACTORY: OnceLock<PluginFactory> = OnceLock::new();

/// Global configuration storage.
static FACTORY_CONFIG: OnceLock<&'static Config> = OnceLock::new();

/// Register factory and config.
///
/// Called by the `export_au!` macro during module initialization.
///
/// # Panics
///
/// Panics if called more than once (which would indicate multiple
/// plugins in the same binary, which is not supported).
pub fn register_factory(
    factory: PluginFactory,
    plugin_config: &'static Config,
) {
    PLUGIN_FACTORY
        .set(factory)
        .expect("AU factory already registered - only one plugin per binary is supported");

    FACTORY_CONFIG
        .set(plugin_config)
        .expect("AU factory config already registered");

    log::debug!(
        "AU factory registered: {} ({} {})",
        plugin_config.name,
        plugin_config.manufacturer,
        plugin_config.subtype
    );
}

/// Create a new plugin instance using the registered factory.
///
/// Returns `None` if no factory has been registered.
pub fn create_instance() -> Option<Box<dyn AuPluginInstance>> {
    PLUGIN_FACTORY.get().map(|factory| factory())
}

/// Get the plugin configuration.
pub fn plugin_config() -> Option<&'static Config> {
    FACTORY_CONFIG.get().copied()
}

/// Check if a factory has been registered.
pub fn is_registered() -> bool {
    PLUGIN_FACTORY.get().is_some()
}
