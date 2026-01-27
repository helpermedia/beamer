# Roadmap: Multi-Format Support

## Vision

A format-agnostic Rust plugin framework with complete macOS support first, expanding to other platforms and formats.

**Core principle:** The framework is format-agnostic at its foundation, with format-specific wrappers.

---

## Current State

| Component | Format | Status |
|-----------|--------|--------|
| Audio/MIDI processing | VST3 | ✅ Complete |
| Audio/MIDI processing | AU | ✅ Complete |
| Audio/MIDI processing | CLAP | Planned |
| Parameters/State | All | ✅ Complete |
| WebView GUI | VST3 + AU | Planned |

**Result:** Plugins work in Logic Pro (AU), Ableton Live (VST3), Cubase (VST3), Bitwig (VST3), and other macOS DAWs.

---

## Crate Structure

```
beamer/
├── crates/
│   ├── beamer/              # Facade (re-exports)
│   ├── beamer-core/         # Format-agnostic traits
│   ├── beamer-vst3/         # VST3 wrapper ✅
│   ├── beamer-au/           # AU wrapper ✅
│   ├── beamer-clap/         # CLAP wrapper (planned)
│   ├── beamer-webview/      # Platform WebView (planned)
│   └── beamer-macros/       # Derive macros
```

---

## Completed Phases

### Phase 1: Format-Agnostic Core ✅

**Goal:** Remove VST3-specific naming from `beamer-core`.

**Completed:**
- Renamed `Vst3Parameters` → `Parameters`
- `beamer-core` has no VST3-specific naming
- All format-specific code in respective wrapper crates

---

### Phase 2: AU Support ✅

**Goal:** Plugins load and run correctly in Logic Pro and other AU hosts.

**Completed:**
- `beamer-au` crate with native Rust implementation using `objc2`
- `AUAudioUnit` subclass via `declare_class!`
- `Parameters` trait mapped to `AUParameterTree`
- Audio render block calling `process()`
- MIDI event translation
- Bundle structure in xtask

---

## Planned Phases

### Phase 3: CLAP Support

**Goal:** Plugins load and run correctly in CLAP hosts (Bitwig, REAPER, FL Studio).

**Key work:**
- Create `beamer-clap` crate
- Implement `clap_plugin_factory` and `clap_plugin`
- Map `ParameterStore` to `clap_plugin_params`
- Audio processing via `clap_process`
- State save/load
- xtask bundling for CLAP

**See:** [CLAP_SUPPORT_ANALYSIS.md](CLAP_SUPPORT_ANALYSIS.md)

---

### Phase 4: WebView GUI (macOS)

**Goal:** Plugins can display web-based UIs in macOS hosts.

**Key work:**
- Create `beamer-webview` crate
- WKWebView embedding via `objc2`
- `IPlugView` implementation for VST3
- `AUViewController` implementation for AU
- Static HTML loading
- Basic IPC (JS ↔ Rust)

**Deliverable:** A plugin with a WebView that loads in VST3, AU, and CLAP hosts.

---

### Phase 5: WebView IPC & Parameter Binding

**Goal:** Bidirectional communication between web UI and plugin.

**Key work:**
- `window.__BEAMER__` JavaScript API
- Invoke pattern (JS → Rust commands)
- Event emission (Rust → JS updates)
- Parameter synchronization
- Automation gesture support (beginEdit/performEdit/endEdit)

---

### Phase 6: Windows Support

**Goal:** Extend VST3 + CLAP + WebView to Windows.

**Key work:**
- Test existing VST3 on Windows
- WebView2 integration via `windows` crate
- CLAP on Windows
- xtask updates for Windows bundling

---

## Deferred

| Item | Reason |
|------|--------|
| Linux | Small market, WebKitGTK complexity |
| AAX | Pro Tools only, requires Avid approval |

---

## References

- [CLAP_SUPPORT_ANALYSIS.md](CLAP_SUPPORT_ANALYSIS.md) - CLAP implementation analysis
- [AU_SUPPORT_ANALYSIS.md](AU_SUPPORT_ANALYSIS.md) - AU implementation analysis
