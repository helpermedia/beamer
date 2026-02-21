//! Platform-specific WebView implementations.

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub mod macos_scheme;

#[cfg(target_os = "windows")]
pub mod windows;

#[cfg(target_os = "macos")]
pub use macos::MacosWebView as PlatformWebView;

#[cfg(target_os = "windows")]
pub use windows::WindowsWebView as PlatformWebView;
