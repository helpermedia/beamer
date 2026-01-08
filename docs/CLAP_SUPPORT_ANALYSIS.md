# CLAP Support Analysis

This document analyzes the feasibility of adding CLAP format support to Beamer, given the current VST3-focused architecture.

## Executive Summary

Adding CLAP support requires **moderate refactoring, not a rewrite**. The core abstractions (two-phase lifecycle, buffers, MIDI) are genuinely format-agnostic. The parameter system has the right *shape* but wrong *names* - VST3 terminology is embedded in what should be the abstraction layer.

**Recommendation:** Rename `Vst3Parameters` → `PluginParameters` before wider adoption to minimize breaking changes.

---

## Current State

### What's Format-Agnostic (Good)

| Component | Status |
|-----------|--------|
| Two-phase lifecycle (`Plugin` → `AudioProcessor`) | ✅ Works for any format |
| `Buffer`, `AuxiliaryBuffers` | ✅ Generic audio I/O |
| `Sample` trait (f32/f64) | ✅ Format-independent |
| `MidiEvent`, `MidiBuffer` | ✅ Generic MIDI types |
| `ProcessContext`, `Transport` | ✅ Generic context |
| `BusInfo`, `BusLayout` | ✅ Generic bus config |

### What's VST3-Specific (Needs Work)

| Component | Issue | Fix |
|-----------|-------|-----|
| `Vst3Parameters` trait | Name implies VST3, but interface is generic | Rename to `PluginParameters` |
| `HasParameters` bound | Requires `Vst3Parameters` | Use renamed trait |
| `Units` / `UnitId` | VST3's parameter grouping system | Make optional or abstract |
| `PluginConfig` with TUIDs | VST3 component IDs | Format-specific configs |
| FNV-1a hash to u32 | VST3's parameter ID scheme | CLAP also uses u32, compatible |

---

## The `Vst3Parameters` Trait

Despite its name, the trait interface is mostly format-agnostic:

```rust
pub trait Vst3Parameters: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParameterInfo>;
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);
    fn normalized_to_string(&self, id: ParameterId, normalized: ParameterValue) -> String;
    fn string_to_normalized(&self, id: ParameterId, string: &str) -> Option<ParameterValue>;
    fn normalized_to_plain(&self, id: ParameterId, normalized: ParameterValue) -> ParameterValue;
    fn plain_to_normalized(&self, id: ParameterId, plain: ParameterValue) -> ParameterValue;
}
```

Compare to CLAP's `clap_plugin_params`:

| Beamer | CLAP | Match |
|--------|------|-------|
| `count()` | `count()` | ✅ |
| `info()` | `get_info()` | ✅ |
| `get_normalized()` | `get_value()` | ✅ |
| `set_normalized()` | (via events) | ⚠️ Different mechanism |
| `normalized_to_string()` | `value_to_text()` | ✅ |
| `string_to_normalized()` | `text_to_value()` | ✅ |

The core parameter interface translates directly.

---

## Required Changes

### Phase 1: Core Refactoring

1. **Rename `Vst3Parameters` → `PluginParameters`**
   - Location: `beamer-core/src/parameters.rs`
   - Update all references in `beamer-core`
   - Update `HasParameters` trait bound

2. **Make `Units` trait optional**
   - CLAP doesn't have the same unit hierarchy
   - Could be a VST3-specific extension or use a feature flag

3. **Abstract `PluginConfig`**
   - Create format-agnostic `PluginMetadata` (name, vendor, version, categories)
   - Format-specific configs extend with their requirements (TUIDs for VST3, etc.)

4. **Update derive macros**
   - `#[derive(Parameters)]` already generates generic code
   - Minimal changes needed after trait rename

### Phase 2: CLAP Wrapper

1. **Create `beamer-clap` crate**
   - Structure mirrors `beamer-vst3`
   - Implement CLAP plugin entry points
   - Map `PluginParameters` to `clap_plugin_params`

2. **CLAP-specific considerations**
   - Event-based parameter changes (vs. direct `set_normalized`)
   - CLAP extensions (state, GUI, etc.)
   - Different threading model documentation

3. **Export macro**
   - `export_clap!(CONFIG, ClapProcessor<MyPlugin>)`
   - Or unified: `export_plugin!(CONFIG, MyPlugin, formats: [vst3, clap])`

---

## Effort Estimate

| Task | Scope |
|------|-------|
| Rename `Vst3Parameters` | Small - find/replace + update bounds |
| Abstract `Units` | Small - make optional or feature-gated |
| Create `PluginMetadata` | Medium - design format-agnostic config |
| Create `beamer-clap` | Large - new wrapper crate |
| Update macros | Small - mostly naming changes |
| Testing & validation | Medium - verify both formats work |

**Total:** Medium

---

## Alternatives Considered

### Option A: Separate Trait Per Format

```rust
// beamer-core
pub trait PluginParameters { ... }

// beamer-vst3
pub trait Vst3Parameters: PluginParameters { ... }

// beamer-clap
pub trait ClapParameters: PluginParameters { ... }
```

**Pros:** Clean separation
**Cons:** Plugins need to implement both if targeting multiple formats

### Option B: Feature Flags

```rust
#[cfg(feature = "vst3")]
impl Vst3Parameters for MyParams { ... }

#[cfg(feature = "clap")]
impl ClapParameters for MyParams { ... }
```

**Pros:** Compile-time format selection
**Cons:** Complexity, harder to test both formats together

### Option C: Single Unified Trait (Recommended)

```rust
// beamer-core
pub trait PluginParameters { ... }  // Current Vst3Parameters, renamed

// Format wrappers adapt this trait to their specific APIs
```

**Pros:** Simple, minimal duplication, macros generate once
**Cons:** May need escape hatches for format-specific features

---

## CLAP-Specific Features to Consider

| Feature | Notes |
|---------|-------|
| Polyphonic modulation | CLAP's killer feature, needs explicit support |
| Voice info | Per-voice parameter modulation |
| Remote controls | CLAP's surface control API |
| Preset discovery | CLAP's preset browser integration |
| GUI scaling | Different from VST3's approach |

Some of these could be added as optional traits/extensions without affecting the core API.

---

## Recommendation

1. **Do the rename now** - `Vst3Parameters` → `PluginParameters` while at version 0.1.x
2. **Keep `Units` as-is** - it's harmless for CLAP (just returns empty)
3. **Defer CLAP wrapper** - until core WebView UI is complete (higher priority)
4. **Design for extensibility** - format-specific features as optional trait extensions

The architecture is sound. The main issue is naming, not structure.
