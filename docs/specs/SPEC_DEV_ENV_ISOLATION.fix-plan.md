# Fix Plan: Dev Environment Isolation — macOS LaunchServices Gap

**Spec:** `SPEC_DEV_ENV_ISOLATION.md`  
**Date:** 2026-06-24

---

## Root cause

`set_macos_app_display_name()` in `agentmux-cef/src/lib.rs:1265` mutates the
main bundle's info dictionary to set `CFBundleName` and `CFBundleDisplayName`.
It **never sets `CFBundleIdentifier`**. The dev build runs as a raw binary
without an `Info.plist`, so its main bundle has no identifier.

macOS LaunchServices routes Dock clicks and Finder double-clicks using
`CFBundleIdentifier`. Without one on the dev instance, the OS falls back to
path-based matching. The packaged stable `.app` has
`ai.agentmux.stable.<version>` — a different identity — so a Finder
double-click of the stable `.app` should launch fresh. But Dock behavior is
less predictable with an unidentified app: macOS may coalesce both instances
under the same Dock tile, letting a click on either one focus whichever was
most recently active.

---

## Fix — inject `CFBundleIdentifier` into the dev build's main bundle

**File:** `agentmux-cef/src/lib.rs`  
**Function:** `set_macos_app_display_name()` (line ~1332)

In the `if !info.is_null() && … is_kind(…)` block where `CFBundleName` and
`CFBundleDisplayName` are already set, add `CFBundleIdentifier`:

```rust
// After setting CFBundleName / CFBundleDisplayName:

// Set CFBundleIdentifier so LaunchServices treats dev and stable instances
// as distinct apps. Without this the dev build has no identifier and the
// OS may coalesce it with the stable .app under the same Dock tile.
// AGENTMUX_CHANNEL is set by the launcher before the host starts and is
// always present; fall back to "dev" only for unmanaged direct invocations.
let channel = std::env::var("AGENTMUX_CHANNEL")
    .unwrap_or_else(|_| "dev".to_string());
let version = env!("CARGO_PKG_VERSION");
let bundle_id = format!("ai.agentmux.{}.{}", channel, version);
if let Ok(c_id) = std::ffi::CString::new(bundle_id) {
    let ns_id = make(cls_str, sel_with, c_id.as_ptr());
    if !ns_id.is_null() {
        let k_id = make(cls_str, sel_with, b"CFBundleIdentifier\0".as_ptr() as _);
        set_obj(info, sel_set_obj, ns_id, k_id);
        tracing::info!("macOS: set CFBundleIdentifier on main bundle");
    }
}
```

**Result:**

| Instance | `CFBundleIdentifier` |
|----------|----------------------|
| Stable `.app` (packaged) | `ai.agentmux.stable.0.49.1` (from `Info.plist`) |
| `task dev:local` (branch X, clone Y) | `ai.agentmux.dev-X-Y.0.49.2` (injected at runtime) |

LaunchServices now sees them as distinct apps. The stable `.app`'s Dock tile
and the dev instance's Dock tile are separate entries. Clicking one never
focuses the other.

**Scope:** 1 file, ~10 lines inside an existing `unsafe` block.  
**Risk:** Low. The mutable-dictionary guard is already in place. The set only
runs for dev builds (the function is called unconditionally but the bundle
id is unique per channel — for packaged builds, the `Info.plist` already
provides the identifier and the in-process set is a no-op / redundant write).

---

## Fix B — Launcher "already running" message — dev hint  *(minor, optional)*

When a second `task dev` invocation finds the first dev instance's socket,
the message `"AgentMux is already running for this data directory"` gives no
hint that `task dev:local` is the escape hatch.

**File:** `agentmux-launcher/src/main.rs`

Pass `channel: &str` into `bind_socket_with_recovery` (called at lines 856,
~1345). At the two `eprintln!` sites (lines 662, 695):

```rust
if channel.starts_with("dev-") {
    eprintln!(
        "AgentMux dev instance already running (channel: {}).\n\
         Use `task dev:local` to launch a second isolated session.\n\
         Socket: {}",
        channel, socket_path
    );
} else {
    eprintln!("AgentMux is already running for this data directory.\n\nSocket: {}", socket_path);
}
```

**Scope:** 1 file, ~20 lines. Message-only.

---

## Implementation order

1. **Fix A** — `agentmux-cef/src/lib.rs` — the actual isolation gap, ~10 lines
2. **Fix B** — `agentmux-launcher/src/main.rs` — UX improvement, independent

Both can go in the same PR as the browser-pane fix or land separately.
