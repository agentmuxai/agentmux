---
type: feat
scope: linux
title: Restore --features patched-libcef on CEF 148 via cef-dll-sys fork
---
The Linux native window-drag patch (`CefWindow::BeginWindowDrag`) lived in our
forked CEF 146 binding (`cef-dll-sys 146.7.0+146.0.12`). When the workspace bumped to
CEF 148 in PR #1221, `cef-dll-sys 148.3.0+148.0.9` from crates.io regenerated against
upstream CEF 148 headers, which don't expose the patched slot — so any build with
`--features patched-libcef` failed with `error[E0609]: no field begin_window_drag on
_cef_window_t`. That made the feature impossible to enable, leaving Linux on a no-op
drag implementation since the 148 bump.

This PR adds a workspace `[patch.crates-io]` entry pointing `cef-dll-sys` at the
`AgentU-asaf/cef-rs#agentmux/148-begin-window-drag` fork, which appends the
`begin_window_drag` field to the linux_x86_64 binding (the same mechanical edit
published in `cef-dll-sys 146.7.0+146.0.12`).

Verification: `cargo build --release -p agentmux-cef --features patched-libcef`
completes against CEF 148, and the resulting binary contains the patched-libcef
path's runtime string (`[start_window_drag] BeginWindowDrag returned …`).

Macros are unchanged: the patch transparently overrides the same crate name + version,
so macOS and Windows builds (which don't enable `patched-libcef`) link unaffected.

See SPEC §5: docs/specs/SPEC_CEF_148_LINUX_FORWARD_PORT_2026_06_04.md
