---
name: no-direct-libc
description: User prefers not using libc crate directly - use higher-level Rust abstractions or nix crate instead
type: feedback
---

Do not use libc directly in this project. Use higher-level Rust abstractions (std, nix crate, etc.) instead.

**Why:** User explicitly asked not to use libc directly. Likely wants safer, more idiomatic Rust code.

**How to apply:** Replace all `libc::` calls with nix crate equivalents or std library functions. For operations nix doesn't cover, find other safe Rust crates.
