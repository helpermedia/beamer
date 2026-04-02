# beamer-webview

WebView GUI system for the Beamer framework (macOS).

This crate provides web-based plugin interfaces using WKWebView, with bidirectional IPC between Rust and JavaScript and a custom URL scheme for embedded asset serving.

## Features

- **WKWebView integration**: Native macOS WebView with no external dependencies
- **Bidirectional IPC**: Parameter sync and custom message passing between Rust and JavaScript
- **Custom URL scheme**: Embedded asset serving (HTML, CSS, JS) without a local HTTP server
- **Per-plugin class isolation**: Unique ObjC class names prevent collisions when multiple Beamer plugins load in the same host process
- **Background color**: Configurable background color to prevent white flash during content load

## Platform Requirements

- **macOS 10.11+** (WKWebView minimum)
- **Apple Silicon and Intel** supported

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything you need.

Use `beamer-webview` directly only if you're:
- Implementing a custom WebView wrapper
- Building tooling that needs WebView-specific functionality

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)

## License

MIT
