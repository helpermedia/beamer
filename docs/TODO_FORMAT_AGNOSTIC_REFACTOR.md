# TODO: Make Core Format-Agnostic

> **Goal:** Remove VST3-specific naming from `beamer-core` so the framework can cleanly support multiple plugin formats (VST3, AU, CLAP).
>
> **Why now:** This is a breaking change. Easier at 0.1.x than after wider adoption.

---

## Overview

The core abstractions (Plugin, AudioProcessor, Buffer, MIDI) are already format-agnostic in design. The problem is **naming** — VST3 terminology leaked into what should be the abstraction layer.

**Primary change:** `Vst3Parameters` → `Parameters`

---

## Tasks

### 1. Audit beamer-core for VST3-isms

Search for any type, trait, or function with "Vst3" or "vst3" in the name within `beamer-core`.

**Known items to rename:**

| Current Name | New Name | Location |
|--------------|----------|----------|
| `Vst3Parameters` | `Parameters` | `beamer-core/src/parameters.rs` |

**Search commands:**
```bash
# Find all occurrences
grep -rn "Vst3" crates/beamer-core/
grep -rn "vst3" crates/beamer-core/

# Also check for VST3-specific concepts that should be abstracted
grep -rn "TUID" crates/beamer-core/
grep -rn "UnitId" crates/beamer-core/
```

**Document any additional items found here before proceeding.**

---

### 2. Rename Vst3Parameters → Parameters

**Files to modify:**

- [ ] `crates/beamer-core/src/parameters.rs` — Trait definition
- [ ] `crates/beamer-core/src/lib.rs` — Re-exports
- [ ] `crates/beamer-core/src/plugin.rs` — Trait bounds (if any)
- [ ] `crates/beamer-core/src/processor.rs` — Trait bounds (if any)

**The trait currently looks like:**
```rust
pub trait Vst3Parameters: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParameterInfo>;
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);
    // ... etc
}
```

**Should become:**
```rust
pub trait Parameters: Send + Sync {
    fn count(&self) -> usize;
    fn info(&self, index: usize) -> Option<&ParameterInfo>;
    fn get_normalized(&self, id: ParameterId) -> ParameterValue;
    fn set_normalized(&self, id: ParameterId, value: ParameterValue);
    // ... etc
}
```

---

### 3. Update HasParameters trait bound

The `HasParameters` trait likely has a bound on `Vst3Parameters`. Update it.

**Current (likely):**
```rust
pub trait HasParameters: Send + 'static {
    type Parameters: Vst3Parameters + Units;
    // ...
}
```

**New:**
```rust
pub trait HasParameters: Send + 'static {
    type Parameters: Parameters + Units;
    // ...
}
```

---

### 4. Update derive macros

**Files:**
- [ ] `crates/beamer-macros/src/lib.rs`
- [ ] `crates/beamer-macros/src/parameters.rs` (or similar)

The `#[derive(Parameters)]` macro generates a `Vst3Parameters` impl. Update to generate `Parameters` impl.

**Search for:**
```bash
grep -rn "Vst3Parameters" crates/beamer-macros/
```

---

### 5. Update beamer-vst3

The VST3 wrapper uses the trait. Update all references.

**Files:**
- [ ] `crates/beamer-vst3/src/processor.rs`
- [ ] `crates/beamer-vst3/src/controller.rs` (if exists)
- [ ] Any other files referencing the trait

**Search:**
```bash
grep -rn "Vst3Parameters" crates/beamer-vst3/
```

---

### 6. Update beamer facade crate

**Files:**
- [ ] `crates/beamer/src/lib.rs`
- [ ] `crates/beamer/src/prelude.rs`

Update re-exports if the trait is exposed through the facade.

---

### 7. Update all examples

Each example may reference the trait or its derives.

**Examples to check:**
- [ ] `examples/gain/src/lib.rs`
- [ ] `examples/delay/src/lib.rs`
- [ ] `examples/synth/src/lib.rs`
- [ ] `examples/compressor/src/lib.rs`
- [ ] `examples/midi-transform/src/lib.rs`

**Likely no changes needed** if examples only use `#[derive(Parameters)]` and don't reference the trait directly. But verify compilation.

---

### 8. Update documentation

- [ ] `ARCHITECTURE.md` — Any references to Vst3Parameters
- [ ] `docs/REFERENCE.md` — API documentation
- [ ] `README.md` — If trait is mentioned
- [ ] Rustdoc comments in the code

**Search:**
```bash
grep -rn "Vst3Parameters" *.md
grep -rn "Vst3Parameters" docs/
```

---

### 9. Consider Units trait

The `Units` trait is for VST3's parameter grouping system. Evaluate:

1. Is this concept VST3-specific or general?
2. Does AU have equivalent functionality?
3. Should it remain as-is, be renamed, or be made optional?

**Decision options:**
- Keep as `Units` (if the concept is general enough)
- Rename to `ParameterGroups` or similar
- Make it an optional trait extension
- Keep it but document it may not apply to all formats

**For now:** Keep as-is. AU can implement it as no-op if needed. Revisit during AU implementation.

---

### 10. Run tests and verify

```bash
# Build everything
cargo build --workspace

# Run tests
cargo test --workspace

# Check examples compile
cargo build --examples

# Run clippy
cargo clippy --workspace

# Build and bundle an example to verify end-to-end
cargo xtask bundle gain --release
```

---

## Verification Checklist

After all changes:

- [ ] `grep -rn "Vst3" crates/beamer-core/` returns no results
- [ ] `grep -rn "vst3" crates/beamer-core/` returns no results (except maybe comments explaining the history)
- [ ] All crates compile
- [ ] All tests pass
- [ ] All examples compile
- [ ] At least one example loads successfully in a DAW
- [ ] Documentation is updated

---

## Breaking Change Notice

This is a breaking change for any external users. When releasing:

1. Bump version appropriately (0.1.x → 0.2.0 or similar)
2. Add migration note to CHANGELOG (if exists) or README:

```markdown
## Migration from 0.1.x

### Trait Rename
`Vst3Parameters` has been renamed to `Parameters`.

If you implemented the trait manually (rare), update your impl:
```rust
// Before
impl Vst3Parameters for MyParams { ... }

// After
impl Parameters for MyParams { ... }
```

If you only use `#[derive(Parameters)]`, no changes needed.
```

---

## Notes

- The core already IS format-agnostic in design — this is just fixing the naming
- The actual trait interface doesn't need to change, just the name
- This unlocks clean AU and CLAP support without "importing VST3 stuff"
