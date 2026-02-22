//! Asset types for the scheme handler.
//!
//! Re-exports `EmbeddedAssets` from `beamer_core`. Assets are passed
//! directly to each scheme handler instance rather than stored globally.

pub use beamer_core::{EmbeddedAsset, EmbeddedAssets};
