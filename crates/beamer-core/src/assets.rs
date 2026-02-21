//! Embedded web asset types for WebView GUIs.
//!
//! These types live in beamer-core so that [`Config`](crate::Config) can reference
//! them without creating a circular dependency with beamer-webview.

/// A single file embedded at compile time.
#[derive(Debug)]
pub struct EmbeddedAsset {
    /// Relative path within the webview directory (e.g. "index.html", "assets/style.css").
    pub path: &'static str,
    /// File contents.
    pub data: &'static [u8],
}

/// Collection of embedded web assets.
#[derive(Debug)]
pub struct EmbeddedAssets {
    assets: &'static [EmbeddedAsset],
}

impl EmbeddedAssets {
    /// Create a new asset collection.
    pub const fn new(assets: &'static [EmbeddedAsset]) -> Self {
        Self { assets }
    }

    /// Look up a file by path (e.g. "index.html", "assets/style.css").
    pub fn get(&self, path: &str) -> Option<&'static [u8]> {
        self.assets.iter().find(|a| a.path == path).map(|a| a.data)
    }
}
