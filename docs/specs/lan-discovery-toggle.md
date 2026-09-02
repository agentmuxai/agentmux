# Spec: LAN Discovery Toggle (HostPopover)

**Branch:** `agenty/governance-widget-spec`
**Status:** Draft — design
**Date:** 2026-05-25
**Author:** AgentY
**Related:** `specs/lan-awareness-and-embedded-jekt-api.md`, `specs/windows-firewall-fix.md`, `specs/hostname-popover.md`, `specs/SPEC_WARDEN_WIDGET_2026-05-25.md`

---

## TL;DR

Add a **toggle switch** to the **HostPopover** (bottom-right status bar, the hostname
chip). The toggle controls the existing `network:lan_discovery` setting and
**starts/stops the mDNS daemon live** — no restart. The setting **stays opt-in**
(`false` by default) so Windows users don't get a surprise firewall prompt on
launch. Discovery becomes a discoverable, one-click affordance instead of a
hidden settings.json key.

---

## Why

Today the feature is gated behind a settings.json edit:

- Setting: `network:lan_discovery: bool` — default `false`
  (`agentmux-srv/src/backend/wconfig/types.rs:206-207`)
- The HostPopover already detects the disabled state and shows the literal text
  `Enable via "network:lan_discovery": true`
  (`frontend/app/statusbar/HostPopover.tsx:144`)
- mDNS daemon is started **once at boot** (`agentmux-srv/src/main.rs:601-620`).
  Changing the setting requires a restart today.

This is bad for two reasons:

1. **Discoverability.** Operators see "LAN discovery disabled" and a string they
   have to copy into a JSON file. Most never bother.
2. **The opt-in *reason* (firewall) is invisible.** Users don't know why it's
   off, only that it is.

Why we keep it opt-in (and don't just flip the default):

> `specs/windows-firewall-fix.md:117-118` — "Disable mDNS LAN discovery by default
> → firewall popup gone. One config flag, zero risk, the feature already handles
> failure gracefully."

mDNS binds `0.0.0.0:5353 UDP`, which Windows Firewall intercepts. Flipping the
default to `true` would resurrect the popup on every fresh Windows install. A
UI toggle is the right middle ground: the user **initiates** the opt-in, so the
firewall popup is the **expected consequence**, not a surprise.

---

## Where it lives

The HostPopover is the rightmost item in the status bar (`frontend/app/statusbar/StatusBar.tsx:61-65`).
Clicking the hostname opens a popover that already has a **Network** section
between Instance/Host info and Ports.

Current "disabled" state (HostPopover.tsx lines 139-147):

```
LAN discovery disabled
Enable via "network:lan_discovery": true
```

This is exactly where the toggle goes — same lines, real control instead of
literal text.

---

## UX

### Disabled state (default)

```
┌─ desk-mac.local ─────────────────────────────┐
│ OS    Windows 11                             │
│ IP    192.168.1.42                           │
│ ───────────────────────────────────────────  │
│ Instance  a3f9-...                           │
│ Host      cef                                │
│ PID       18432                              │
│ Data      C:\Users\asaf\.agentmux            │
│ ───────────────────────────────────────────  │
│ LAN discovery       [ off ⚪ ]               │
│ Discover other AgentMux instances via mDNS.  │
│ May prompt Windows Firewall on first enable. │
│ ───────────────────────────────────────────  │
│ IPC       64321                              │
│ Backend   64322                              │
│ ...                                          │
└──────────────────────────────────────────────┘
```

### Enabled state, no peers yet

```
│ LAN discovery       [ on  🟢 ]               │
│ No peers found yet                           │
```

### Enabled state, peers discovered

```
│ LAN discovery       [ on  🟢 ]               │
│ ◆ 2 instances on LAN                         │
│   pi-lab          v0.38.2                    │
│   asaf-laptop     v0.38.4                    │
```

### After a fresh enable (first-time UX on Windows)

The firewall prompt fires *immediately* when the daemon starts. Since the user
just clicked the toggle, the prompt is intelligible: "AgentMux wants to access
the network → Allow / Block." No mystery.

If they **Block**, mDNS fails. We already handle that gracefully (`main.rs:613-615`
`tracing::warn!("LAN discovery unavailable: {e}")`). The toggle UI should
detect daemon-failure and surface it:

```
│ LAN discovery       [ on  🟢 ]               │
│ ⚠ Blocked — check firewall settings          │
```

---

## Behavior

### Toggle semantics

| User action | Setting write | Backend action |
|------------|---------------|----------------|
| OFF → ON | `set("network:lan_discovery", true)` | Start `LanDiscovery` daemon |
| ON → OFF | `set("network:lan_discovery", false)` | Stop daemon, drop peer list |

### Live daemon lifecycle (no restart required)

This is the non-trivial part. Today, the daemon is constructed in `main.rs` and
moved into `AppState.lan_discovery: Option<Arc<LanDiscovery>>` once at boot.

**Change:** wrap the daemon slot in something that can be swapped at runtime,
and subscribe to setting changes:

```rust
// AppState (agentmux-srv/src/server/mod.rs:69)
pub lan_discovery: Arc<RwLock<Option<Arc<LanDiscovery>>>>,
```

ConfigWatcher already exists for hot-reload. Add a hook:

```rust
// When network:lan_discovery flips:
config_watcher.on_change("network:lan_discovery", |new_val: bool, ctx| {
    let mut slot = ctx.app_state.lan_discovery.write();
    if new_val && slot.is_none() {
        // OFF → ON
        match LanDiscovery::start(ctx.instance_id.clone(),
                                  ctx.hostname.clone(),
                                  ctx.version.clone(),
                                  ctx.port,
                                  ctx.event_bus.clone()) {
            Ok(d) => *slot = Some(d),
            Err(e) => {
                tracing::warn!("LAN discovery start failed: {e}");
                ctx.event_bus.broadcast_event(&WSEventType {
                    eventtype: "laninstances:error".to_string(),
                    oref: String::new(),
                    data: Some(json!({"error": e.to_string()})),
                });
            }
        }
    } else if !new_val && slot.is_some() {
        // ON → OFF
        *slot = None;  // Drop impl unregisters mDNS + shuts down daemon
        // Broadcast empty list so frontend clears peers
        ctx.event_bus.broadcast_event(&WSEventType {
            eventtype: "laninstances".to_string(),
            oref: String::new(),
            data: Some(json!([])),
        });
    }
});
```

The `Drop` impl on `LanDiscovery` (lines 218-228) already handles graceful
unregister + shutdown. Re-creating is a clean re-run of `start()`.

### Boot semantics (unchanged from spec's existing description)

Boot still reads `network:lan_discovery` and starts the daemon if `true`. The
only addition is the hot-reload hook. If the setting is missing from
`settings.json`, default stays `false` (no behavior change).

---

## Implementation

### Files changed

| File | Change |
|------|--------|
| `frontend/app/statusbar/HostPopover.tsx` | Add toggle component to disabled-state block; keep peer list rendering for enabled-state |
| `frontend/app/statusbar/StatusBar.scss` (or `_instance-panel.scss`) | Toggle styles (small switch, matches existing status-bar-item visuals) |
| `frontend/app/store/global.ts` | Add `setLanDiscoveryEnabled(enabled: boolean)` helper (writes via SetConfigCommand) |
| `agentmux-srv/src/server/mod.rs` | `AppState.lan_discovery: Arc<RwLock<Option<Arc<LanDiscovery>>>>` |
| `agentmux-srv/src/server/mod.rs` | `handle_lan_instances` reads through the RwLock |
| `agentmux-srv/src/main.rs` | Move initial start into a shared function; wire ConfigWatcher hook |
| `agentmux-srv/src/backend/wconfig/config_watcher_fs.rs` (or wherever the watcher dispatches) | Add `network:lan_discovery` change hook |
| `schema/settings.json` | Add missing `network:lan_discovery` entry (also fixes a current schema gap) |

No new dependencies. `mdns-sd` is already in `Cargo.toml`. Setting write API
(`SetConfigCommand`) already exists.

### Frontend toggle component

Small SolidJS component. No new library — reuse plain checkbox styled as a
switch (matches the codebase's minimal-UI norm; existing styling lives in
`_instance-panel.scss`).

```tsx
const LanDiscoveryToggle = (): JSX.Element => {
    const enabled = () => !!settingsAtom()?.["network:lan_discovery"];
    const handleToggle = async (e: Event) => {
        const next = (e.target as HTMLInputElement).checked;
        await RpcApi.SetConfigCommand(TabRpcClient, { "network:lan_discovery": next } as any);
    };
    return (
        <label class="status-bar-toggle">
            <span class="status-bar-popover-label">LAN discovery</span>
            <input type="checkbox" checked={enabled()} onChange={handleToggle} />
        </label>
    );
};
```

`RpcApi.SetConfigCommand(TabRpcClient, {...})` is the established settings-write
path used elsewhere (e.g. `frontend/app/menu/base-menus.ts:71`,
`frontend/app/window/action-widgets.tsx:85`,
`frontend/app/tab/tabbar.tsx:614`). It wraps the `setconfig` RPC defined in
`frontend/app/store/rpc-api.ts:396`.

### State sync

`settingsAtom` already mirrors the backend settings (via `wave:setsettings`
events from ConfigWatcher). Toggle reads from `settingsAtom`, writes via RPC,
and `settingsAtom` re-syncs on the WS broadcast. No new state plumbing.

---

## Edge cases

1. **Firewall blocked.** Daemon `start()` fails. Backend broadcasts
   `laninstances:error` with the error message. Frontend renders an inline
   warning under the toggle. Setting stays `true` so the user can retry without
   re-toggling.
2. **Toggle clicked rapidly.** Setting writes are atomic. Daemon
   start/stop are serialized by the `RwLock` on the slot. Worst case: one extra
   start/stop cycle.
3. **Setting edited externally** (someone edits `settings.json` while AgentMux
   is running). ConfigWatcher already fires; the same hook handles it. UI
   reflects the change automatically because `settingsAtom` is updated.
4. **Multi-instance on same host.** Two AgentMux processes on the same Windows
   box: the first to enable triggers the firewall once. The second uses the
   same allowed rule. Both discover each other via loopback mDNS (mdns-sd
   advertises on all interfaces including localhost).
5. **Setting absent vs. `false`.** Same behavior — discovery off. UI shows
   toggle off in both cases.
6. **Network change** (Wi-Fi switch). mdns-sd handles interface changes
   internally; no special UI behavior needed.

---

## Why not other approaches

| Approach | Why not |
|----------|---------|
| Flip default to `true` | Reverses `windows-firewall-fix.md`. Surprises Windows users with firewall popup on every fresh install. |
| First-run nudge modal | Heavyweight for a one-line setting. Modal fatigue. Also harder to revisit later. |
| Settings widget only | Already exists (`/settings`), but it just opens `settings.json` in an external editor. Not in-app, not discoverable, not one-click. |
| New top-level network widget | Premature — once the Warden widget lands, this lives there too. For now, HostPopover is the right scope. |

---

## Future work / open questions

1. **Warden integration.** Once `SPEC_WARDEN_WIDGET_2026-05-25.md`
   ships, the Warden's L2 section will also expose this toggle.
   HostPopover's toggle remains as a quick-access affordance; the Warden is the
   deep-dive surface. Both write the same setting.
2. **Per-interface enable.** A future enhancement could let the user pick which
   network interface mDNS advertises on (e.g., enable on Wi-Fi but not on a VPN
   tunnel). Out of scope here.
3. **Same-host vs LAN split.** Loopback-only mDNS for same-host discovery
   (no firewall trigger) plus opt-in LAN — discussed in the governance spec.
   Bigger change, separate PR.
4. **Visual indicator of firewall-pending state.** On Windows, after the user
   toggles ON, the daemon `start()` may succeed at bind but actual mDNS traffic
   may still be blocked pending the user's firewall decision. We can't easily
   detect this. v1 just trusts `start()`'s return value.

---

## Test plan

- [ ] Default settings: HostPopover shows toggle in OFF state
- [ ] Click toggle ON: setting persists in `settings.json`, daemon starts, no
      app restart
- [ ] Within ~5s, peer instance on same host shows in the popover peer list
- [ ] Click toggle OFF: daemon stops, peer list clears within ~1s
- [ ] On Windows, first-time ON triggers firewall prompt; clicking Allow lets
      mDNS proceed
- [ ] On Windows, first-time ON + Block surfaces "Blocked — check firewall
      settings" in the popover
- [ ] Edit `settings.json` externally to flip the value: UI updates without
      manual refresh
- [ ] Cargo check / cargo build pass with `Arc<RwLock<Option<Arc<LanDiscovery>>>>`
      lifetime changes
- [ ] Existing `/api/lan-instances` endpoint still returns correct list

---

## Acceptance criteria

- [ ] Toggle visible in HostPopover's Network section, regardless of enabled
      state
- [ ] Toggle persists state across app restarts
- [ ] No app restart required to apply the change
- [ ] Default behavior unchanged on fresh install (opt-in)
- [ ] `schema/settings.json` includes `network:lan_discovery`
- [ ] Failure modes (firewall blocked, daemon start error) are visible in the
      popover, not silent
