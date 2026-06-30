---
patch: "fix(ci): stage CEF DLLs from OUT_DIR on rust-cache hit (Windows artifacts)"
---

On a rust-cache hit the cef-dll-sys build script does not re-run, so its
side-effect of copying libcef.dll/icudtl.dat to target/release/ never
happens — causing the bundle:windows guard to fail with "incomplete CEF
runtime". The CEF SDK is still intact in the build OUT_DIR
(target/release/build/cef-dll-sys-*/out/), which rust-cache preserves.
bundle:windows now falls back to that dir and copies the DLLs when
target/release/ is missing them.
