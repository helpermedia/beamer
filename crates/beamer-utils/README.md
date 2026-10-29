# beamer-utils

Internal utilities for the Beamer audio plugin (AU, VST3) framework.

This crate provides low-level, zero-dependency utilities shared between
`beamer-core`, `beamer-macros` and the bundler (`xtask`).

## Contents

- **Hash functions** - FNV-1a for stable parameter ID generation
- **Web UI detection** - Where the web UI of a plugin comes from, so that the
  export macro and the bundler agree whether a plugin has an editor

## Usage

This crate is an internal implementation detail. Plugin authors should use
the `beamer` facade crate instead:

```rust
use beamer::prelude::*;
```

## Design Principles

1. **Zero dependencies** - Pure Rust implementations only
2. **Minimal scope** - Only utilities genuinely needed by multiple crates
3. **Well-documented** - Comprehensive docs with examples and rationale

## License

MIT
