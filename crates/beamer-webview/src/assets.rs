//! Asset registration for the scheme handler.
//!
//! Types are defined in [`beamer_core::assets`] and re-exported here.
//! This module provides the global registration that the scheme handler
//! uses to look up assets at runtime.

use std::sync::OnceLock;

pub use beamer_core::{EmbeddedAsset, EmbeddedAssets};

static GLOBAL_ASSETS: OnceLock<&'static EmbeddedAssets> = OnceLock::new();

/// Register the embedded assets for the scheme handler.
/// Called once during plugin initialization.
pub fn register_assets(assets: &'static EmbeddedAssets) {
    GLOBAL_ASSETS.set(assets).ok();
}

/// Look up a file by path. Used by the scheme handler.
pub fn get_asset(path: &str) -> Option<&'static [u8]> {
    GLOBAL_ASSETS.get()?.get(path)
}
