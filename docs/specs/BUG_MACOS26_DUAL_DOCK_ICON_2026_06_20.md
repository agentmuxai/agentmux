# BUG: Dual Dock Icon on macOS 26 Tahoe

**Date:** 2026-06-20  
**Severity:** P2 — visual regression, no functional impact  
**Platform:** macOS 26.5.1 (Build 25F80) / Darwin 25.5.0 arm64  
**Affects:** All builds on macOS 26 Tahoe; confirmed on v0.46.3

---

## Symptom

Two separate AgentMux icons appear in the Dock simultaneously: one full-size
(the CEF host, correct) and one smaller (the launcher, incorrect). Nothing is
pinned — both are live processes.

---

## Evidence

### 1. `lsappinfo list` — both processes registered as `type="Foreground"`

```
bundleID="ai.agentmux.cef.stable"
bundle path="/Volumes/AgentMux/AgentMux.app"
executable path=".../agentmux-launcher"
pid = 90285  !signalled  type="Foreground"  flavor=3  Version="0.46.3"

bundleID="ai.agentmux.cef.stable"
bundle path="/Volumes/AgentMux/AgentMux.app"
executable path=".../agentmux-cef"
pid = 90609  type="Foreground"  flavor=3  Version="0.46.3"
```

Both carry the same bundle ID and the same bundle path.  The `type` field
reflects the *current* activation policy (not the initial one from plist),
confirming the runtime `setActivationPolicy` call is not working as expected.

### 2. Intended design (code)

**`agentmux-launcher/src/splash_mac.rs:246–249`**
```rust
// NSApplication, as an accessory app: no Dock tile (the host sets .regular
// and owns the single tile). Accessory == 1.
let app = send(class(b"NSApplication\0"), sel(b"sharedApplication\0"));
send_void_i64(app, sel(b"setActivationPolicy:\0"), 1);
```

The launcher was designed to call `setActivationPolicy(.accessory)` so it
disappears from the Dock immediately, leaving the single slot for CEF.

**`agentmux-cef/src/main.rs:835–844`**
```rust
set_macos_app_display_name();
set_macos_activation_policy_regular();   // <-- CEF claims the Dock tile
set_macos_dock_icon();
```

CEF then claims a `Regular` (Dock-visible) activation policy.

### 3. Info.plist — no `LSUIElement` declared

**`scripts/package-macos.sh:210–241`** — the generated Info.plist contains:
```xml
<key>CFBundleExecutable</key><string>agentmux-launcher</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<!-- NO LSUIElement key -->
```

Because `NSPrincipalClass=NSApplication` is set without `LSUIElement`, macOS
registers the launcher as a **Foreground** app at launch time. On macOS ≤15
Sequoia, `setActivationPolicy(.accessory)` would then remove the Dock tile
quickly enough to be invisible. On macOS 26 Tahoe, this runtime downgrade no
longer takes effect — `lsappinfo` still reports `type="Foreground"` for PID
90285 even though the code called `setActivationPolicy(1)`.

---

## Root Cause

macOS 26 Tahoe changed (or tightened) the runtime `setActivationPolicy`
behavior for apps initially registered as `Foreground` via `NSPrincipalClass`.
The downgrade from Regular → Accessory is silently ignored; the launcher retains
its Dock slot indefinitely alongside CEF's own Regular slot, producing two icons.

---

## Fix

Add `<key>LSUIElement</key><true/>` to the main `Info.plist` in
`scripts/package-macos.sh`. This prevents the launcher from ever being
registered as a Foreground app at the OS level — no runtime downgrade needed.
CEF's `setActivationPolicy(.regular)` call then creates the sole Dock entry as
intended.

**Why `LSUIElement` rather than `LSBackgroundOnly`:**
- `LSUIElement` apps can show windows (needed for the splash screen).
- `LSBackgroundOnly` apps are shown as "(Background)" in Force Quit, which
  would be confusing for users.
- The existing `setActivationPolicy(1)` call in `splash_mac.rs` becomes
  redundant but harmless.

**No functional impact:** CEF still calls `setActivationPolicy(.regular)` and
claims its own Dock tile. The splash window displayed by the launcher works
correctly for UIElement apps — NSWindow creation, `makeKeyAndOrderFront`, and
alpha animation all function identically.
