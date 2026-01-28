//! Plugin factory registration for Audio Unit.
//!
//! This module provides the factory registration system that enables the AU
//! runtime to create plugin instances. The factory is registered at module
//! initialization time via the `export_au!` macro.

use std::sync::OnceLock;

use beamer_core::Config;

use crate::config::AuConfig;
use crate::instance::AuPluginInstance;

/// Factory function type for creating plugin instances.
pub type PluginFactory = fn() -> Box<dyn AuPluginInstance>;

/// Configuration bundle stored with the factory.
#[derive(Debug)]
pub struct FactoryConfig {
    pub plugin_config: &'static Config,
    pub au_config: &'static AuConfig,
}

/// Global factory storage (set by export_au! macro).
static PLUGIN_FACTORY: OnceLock<PluginFactory> = OnceLock::new();

/// Global configuration storage.
static FACTORY_CONFIG: OnceLock<FactoryConfig> = OnceLock::new();

/// Register factory and configs.
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
    au_config: &'static AuConfig,
) {
    PLUGIN_FACTORY
        .set(factory)
        .expect("AU factory already registered - only one plugin per binary is supported");

    FACTORY_CONFIG
        .set(FactoryConfig {
            plugin_config,
            au_config,
        })
        .expect("AU factory config already registered");

    log::debug!(
        "AU factory registered: {} ({} {})",
        plugin_config.name,
        au_config.manufacturer,
        au_config.subtype
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
    FACTORY_CONFIG.get().map(|c| c.plugin_config)
}

/// Get the AU-specific configuration.
pub fn au_config() -> Option<&'static AuConfig> {
    FACTORY_CONFIG.get().map(|c| c.au_config)
}

/// Check if a factory has been registered.
pub fn is_registered() -> bool {
    PLUGIN_FACTORY.get().is_some()
}
