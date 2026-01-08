# Alternative Roadmap: Multi-Format macOS-First

> **Status:** Under consideration
>
> **Supersedes:** The WebView-first approach in WEBVIEW_ROADMAP.md

---

## Vision

A format-agnostic Rust plugin framework with complete macOS support first, expanding to other platforms later.

**Core principle:** The framework should not be "VST3 with extras bolted on" — it should be format-agnostic at its foundation, with format-specific wrappers.

---

## Target State (macOS Complete)

| Component | Format | Status |
|-----------|--------|--------|
| Audio/MIDI processing | VST3 | Done |
| Audio/MIDI processing | AU | Planned |
| WebView GUI | VST3 + AU | Planned |
| Parameters/State | Both | Done (needs rename) |

**Result:** Plugins that work in Logic Pro (AU), Ableton Live (VST3), Cubase (VST3), Bitwig (VST3), and any other macOS DAW — with modern web-based UIs.

---

## What's Deferred

| Item | Reason |
|------|--------|
| Windows | Focus on macOS first; add later |
| Linux | Small market, WebKitGTK conflicts |
| CLAP | Evaluate after AU; may not be needed |
| AAX | Pro Tools market; significant effort |

These can be revisited once the macOS story is complete.

---

## Phases

### Phase 1: Format-Agnostic Core

**Goal:** Remove VST3-specific naming from `beamer-core` so it can cleanly support multiple formats.

**Key changes:**
- Rename `Vst3Parameters` → `Parameters`
- Audit and rename any other VST3-isms
- Update derive macros
- Update all examples and tests

**Why first:** This is a breaking change. Easier to do now at 0.1.x than after wider adoption.

**Deliverable:** `beamer-core` has no VST3-specific naming; `beamer-vst3` contains all VST3-specific code.

**See:** [TODO_FORMAT_AGNOSTIC_REFACTOR.md](TODO_FORMAT_AGNOSTIC_REFACTOR.md)

---

### Phase 2: AU Support

**Goal:** Plugins load and run correctly in Logic Pro and other AU hosts.

**Approach:** Native Rust implementation using `objc2` bindings (not Steinberg's C++ wrapper).

**Key work:**
- Create `beamer-au` crate
- `AUAudioUnit` subclass via `declare_class!`
- Map `Parameters` trait to `AUParameterTree`
- Audio render block calling `process()`
- MIDI event translation
- Bundle structure (.appex) in xtask
- Pass `auval` validation

**Deliverable:** The gain example loads in Logic Pro, parameters automate, audio processes correctly.

**See:** [TODO_AU_SUPPORT.md](TODO_AU_SUPPORT.md) | [AU_SUPPORT_ANALYSIS.md](AU_SUPPORT_ANALYSIS.md)

---

### Phase 3: WebView GUI (macOS)

**Goal:** Plugins can display web-based UIs in macOS hosts.

**Key work:**
- Create `beamer-webview` crate
- WKWebView embedding via `objc2`
- `IPlugView` implementation for VST3
- `AUViewController` implementation for AU
- Static HTML loading
- Basic IPC (JS ↔ Rust)

**Deliverable:** A plugin with a WebView showing "Hello World" that loads in both VST3 and AU hosts.

**See:** [WEBVIEW_PHASE2A.md](WEBVIEW_PHASE2A.md) (adapt for AU support)

---

### Phase 4: WebView IPC & Parameter Binding

**Goal:** Bidirectional communication between web UI and plugin.

**Key work:**
- `window.__BEAMER__` JavaScript API
- Invoke pattern (JS → Rust commands)
- Event emission (Rust → JS updates)
- Parameter synchronization
- Automation gesture support (beginEdit/performEdit/endEdit)

**Deliverable:** A plugin with a functional web UI that controls parameters with proper DAW automation.

---

### Phase 5: Windows Support

**Goal:** Extend VST3 + WebView to Windows.

**Key work:**
- Test existing VST3 on Windows (may already work)
- WebView2 integration via `windows` crate
- xtask updates for Windows bundling

**Deliverable:** Plugins build and run on Windows with WebView UIs.

---

### Phase 6: Evaluate CLAP

**Goal:** Decide if CLAP support is worth adding.

**Considerations:**
- Market adoption of CLAP by then
- Effort required (the core is already format-agnostic)
- Whether Bitwig/other CLAP hosts matter to users

**Possible outcomes:**
- Add `beamer-clap` crate
- Document as "not planned" with rationale
- Community contribution welcome

---

## Crate Structure (Target)

```
beamer/
├── crates/
│   ├── beamer/              # Facade (re-exports)
│   ├── beamer-core/         # Format-agnostic traits (no VST3/AU specifics)
│   ├── beamer-vst3/         # VST3 wrapper
│   ├── beamer-au/           # AU wrapper (Phase 2)
│   ├── beamer-webview/      # Platform WebView (Phase 3)
│   ├── beamer-macros/       # Derive macros
│   └── beamer-utils/        # Shared utilities
```

---

## Why This Order?

| Phase | Rationale |
|-------|-----------|
| 1. Format-agnostic | Breaking change — do early. Prerequisite for AU. |
| 2. AU | Smaller than WebView. Validates the refactor. Unlocks Logic Pro. |
| 3. WebView macOS | Uses same objc2 skills. Applies to both formats. |
| 4. IPC | Builds on WebView foundation. |
| 5. Windows | Independent of macOS work. Can parallelize if needed. |
| 6. CLAP | Evaluate based on ecosystem at that point. |

---

## Success Criteria

**Phase 1 complete when:**
- No "Vst3" or "vst3" appears in `beamer-core` public API
- All examples compile and pass tests
- ARCHITECTURE.md and REFERENCE.md updated

**Phase 2 complete when:**
- gain, delay, synth examples load in Logic Pro
- Parameters visible and automatable
- Audio processes correctly
- `auval -v aufx` passes

**Phase 3 complete when:**
- WebView-based plugin loads in both VST3 and AU hosts
- Window resizes correctly
- No crashes on open/close cycles

---

## Risks

| Risk | Mitigation |
|------|------------|
| objc2 learning curve | Both AU and WebView need it — learning transfers |
| auval validation failures | Use Steinberg wrapper as reference for quirks |
| Breaking changes upset users | We're at 0.1.x — now is the time |
| Scope creep | Strict phase boundaries; defer aggressively |

---

## Timeline

No time estimates. Each phase is complete when it's complete. The phases are ordered by dependency and value, not by calendar.

---

## References

- [AU_SUPPORT_ANALYSIS.md](AU_SUPPORT_ANALYSIS.md) — Detailed AU implementation analysis
- [CLAP_SUPPORT_ANALYSIS.md](CLAP_SUPPORT_ANALYSIS.md) — CLAP feasibility analysis
- [WEBVIEW_PHASE2A.md](WEBVIEW_PHASE2A.md) — Original WebView planning
- [TODO_FORMAT_AGNOSTIC_REFACTOR.md](TODO_FORMAT_AGNOSTIC_REFACTOR.md) — Phase 1 implementation tasks
