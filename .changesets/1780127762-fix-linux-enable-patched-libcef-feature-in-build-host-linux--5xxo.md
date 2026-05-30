---
type: patch
---

fix(linux): enable patched-libcef feature in build:host:linux so window drag works

PR #1131 (`fix(cef): unbreak agentmux-cef compile on macOS with public
cef-rs 146`) introduced a `patched-libcef` cargo feature on
`agentmux-cef` to gate the `_cef_window_t::begin_window_drag` FFI call
(used by `StartWindowDragTask` for native left-click window drag on
Wayland). It defaulted **off** so macOS could compile against the
public crates.io `cef-rs 146` before the macOS libcef story was sorted.
The Linux `build:host:linux` task in `Taskfile.yml` was not updated to
pass `--features patched-libcef`, so every Linux build after that PR
compiled the FFI call site OUT and the warn-and-no-op branch IN:

    [start_window_drag] patched-libcef feature disabled — native drag
    is a no-op. Rebuild with --features patched-libcef ...

User-visible result: clicking-and-dragging the AgentMux title bar on
Linux did nothing.

Fix: append `--features patched-libcef` to the cargo build in
`build:host:linux`. The two other pieces that needed to line up are
both already fine on Linux:

- The `begin_window_drag` slot has been upstreamed into public
  `cef-dll-sys-146.7.0+146.0.12` (currently pinned in `Cargo.lock`),
  so no `[patch.crates-io]` override is needed.
- `task package:linux` already bundles the right `libcef.so` via
  `scripts/resolve-cef-runtime.sh` — picks the locally-built
  `~/cef-build/.../libcef.so` (a5af/cef
  `agentmux/7680-drag-rightclick-and-transparency`) and strips it
  from ~642 MB → ~422 MB. The runtime ABI guard in
  `agentmux-cef/src/ui_tasks.rs` still catches mismatch.

macOS and Windows tasks unchanged.
