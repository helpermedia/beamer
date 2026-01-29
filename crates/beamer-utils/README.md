# beamer-utils

Internal utilities for the Beamer audio plugin framework.

This crate provides low-level, zero-dependency utilities shared between
`beamer-core` and `beamer-macros`. All functions are compile-time safe
(`const fn` where possible) and usable in both proc-macro and runtime contexts.

## Contents

- **Hash functions** - FNV-1a for stable parameter ID generation

## Usage

This crate is an internal implementation detail. Plugin authors should use
the `beamer` facade crate instead:

```rust
use beamer::prelude::*;
```

## Design Principles

1. **Zero dependencies** - Pure Rust implementations only
2. **Compile-time safe** - All utilities work in `const` contexts
3. **Minimal scope** - Only utilities genuinely needed by multiple crates
4. **Well-documented** - Comprehensive docs with examples and rationale

## License

MIT
