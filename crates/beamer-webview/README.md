# beamer-webview

WebView editor for the Beamer framework (macOS).

This crate provides web-based plugin editors using WKWebView, with bidirectional IPC between Rust and JavaScript and a custom URL scheme for embedded asset serving.

## Features

- **The web UI's protocol in one place**: the `window.__BEAMER__` runtime, the event queue with its senders for the audio thread, `WebViewHandler` for the calls of the page and the editor sync that every format wrapper uses
- **WKWebView integration**: Native macOS WebView with no external dependencies
- **Bidirectional IPC**: Parameter sync and custom message passing between Rust and JavaScript
- **Custom URL scheme**: Embedded asset serving (HTML, CSS, JS) without a local HTTP server
- **Class isolation per plugin binary**: Unique ObjC class names prevent collisions when multiple Beamer plugins load in the same host process
- **Background color**: Configurable background color to prevent white flash during content load

## Platform Requirements

- **macOS 11.0+**
- **Apple Silicon and Intel** supported

## Usage

**Most users should use the [`beamer`](https://crates.io/crates/beamer) crate instead**, which re-exports everything you need.

Use `beamer-webview` directly only if you're:
- Implementing a custom WebView wrapper
- Building tooling that needs WebView-specific functionality

## Documentation

See the [main repository](https://github.com/helpermedia/beamer) for:
- [Getting Started Guide](https://github.com/helpermedia/beamer#quick-start)
- [WebView Editor Guide](https://github.com/helpermedia/beamer/blob/main/docs/WEBVIEW.md)
- [API Reference](https://github.com/helpermedia/beamer/blob/main/docs/REFERENCE.md)

## License

MIT
