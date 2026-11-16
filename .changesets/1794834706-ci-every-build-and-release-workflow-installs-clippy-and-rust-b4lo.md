---
type: patch
---

CI: every build and release workflow installs clippy and rustfmt with the Rust toolchain, so rustup no longer adds them in the middle of a build (the 0.60.30 Linux build failed there)
