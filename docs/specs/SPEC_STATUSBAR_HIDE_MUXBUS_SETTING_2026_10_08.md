# SPEC: a setting to hide the MuxBus cloud dot and sign-in info in the status bar

**Author:** Clamk
**Date:** 2026-10-08
**Status:** implemented — PR #4495

---

## 1. Problem

The host item in the status bar (hostname + LAN diamond) carries two pieces of
AgentMux Cloud (MuxBus) UI that a user who does not use the cloud, or does not
want it on screen, cannot turn off:

1. **The cloud dot** — a 6 px circle after the LAN diamond, green when the
   cloud session is good, red otherwise (`HostPopover.tsx:459-467`,
   `.status-muxbus-dot` in `StatusBar.scss:575`). It renders whenever the
   cloud is configured and a status has been read.
2. **The "MuxBus Cloud" block in the host popover** (`HostPopover.tsx:247-331`):
   the signed-in email, the **Sign in** / **Sign in again** / **Expired —
   re-login** button, **Disconnect**, the macOS Keychain and Linux keyring
   notices, and the sign-in error line.

Both are gated only on `muxbus.isConfigured()`, which is true on every
production build (the cloud config is discovered at runtime,
SPEC_CLOUD_SETTINGS_DISCOVERY_2026_09_27.md). A user who never signs in
therefore sees a permanent red dot and a "Sign in" prompt.

## 2. Goal

One setting, **shown by default**, for users who have no interest in MuxBus at
all and don't want its status in their bar. Anyone who does nothing sees the
status bar exactly as it is today; hiding is an explicit opt-out. It hides both
items. Nothing else about the cloud
changes: the session, the subscriber, WAN jekts, presence and per-agent
credentials all keep running exactly as they do today.

**Non-goals**

- Disabling the cloud connection. This only controls what the status bar shows.
- **Anything LAN.** This is MuxBus **Cloud** only. The LAN diamond, the LAN
  peers list, LAN discovery and its toggle, and the Pair a device panel all
  stay exactly as they are, whatever the setting says. The key and the Settings
  label say "MuxBus Cloud" so the two are never confused.
- Hiding any other host-popover row.
- Changing the Accounts view or Settings, which remain the place to sign in.

## 3. Design

### 3.1 The setting

| | |
|---|---|
| Key | `statusbar:showmuxbuscloud` |
| Type | boolean, `Option<bool>` in Rust |
| Default | absent = **true** (shown) |
| Effect when `false` | no dot, no "MuxBus Cloud" block in the host popover |
| Takes effect | live, no restart |

Absent-means-shown is the `voice:enabled` / `widget:showhelp` shape: only the
explicit opt-out is stored, so `settings.json` stays empty for the default and
nobody's current layout changes.

`statusbar:` is a new key prefix. It follows the existing `widget:` and
`window:` prefixes. If the settings audit prefers an existing prefix, the
alternative is `window:showmuxbuscloud`; nothing below depends on the choice.

### 3.2 Backend

- `crates/srv/src/backend/wconfig/types.rs`: add

  ```rust
  #[serde(rename = "statusbar:showmuxbuscloud", default, skip_serializing_if = "Option::is_none")]
  pub statusbar_show_muxbus_cloud: Option<bool>,
  ```

  next to the other display settings, and a `statusbar:*` clear key
  (`statusbar_clear: bool`, `skip_serializing_if = "is_false"`) if the
  `widget:*` / `window:*` pattern is to be kept for the new group.
- `schema/settings.json`: a `statusbar:showmuxbuscloud` boolean with a description
  that names the default.
- `frontend/types/srv-types.d.ts` (hand-maintained): add
  `"statusbar:showmuxbuscloud"?: boolean;` and, if added, `"statusbar:*"?: boolean;`.

No new RPC. `SetConfigCommand` already persists arbitrary typed keys, and
`settingsAtom` already updates every window when it changes.

### 3.3 Frontend: the status bar

In `HostPopover.tsx`:

```ts
const showMuxbusCloud = () => settingsAtom()?.["statusbar:showmuxbuscloud"] !== false;
```

- **Trigger dot** (line 459): `<Show when={showMuxbusCloud() && muxbus.isConfigured() && muxbus.status() !== null}>`.
- **Popover block** (line 248): `<Show when={showMuxbusCloud() && muxbus.isConfigured()}>`. The two dividers that
  bracket it (lines 249 and 332) must go with the block, or the popover shows a
  doubled divider. Line 249 is already inside the `Show`; the one at 332 is
  shared with the Ports section, so it stays.
- **Polling** (lines 369-372): the 60 s `muxbus.refresh()` exists only to keep
  the dot current. With the setting off, skip the interval; the existing
  `void muxbus.refresh()` on popover open (line 427) is then also unnecessary.
  Both are guarded by `showMuxbusCloud()` and re-run when it flips on, so turning it
  back on shows a correct dot within one refresh. This is an optimisation, not
  a requirement; if it complicates the effect lifecycle, leave polling as is.

`HostPopoverPanel` receives `muxbus` as a prop; give it `showMuxbusCloud` the same
way rather than reading the atom a second time.

### 3.4 Frontend: the Settings control

A toggle row, "Show MuxBus Cloud status in the status bar", with description
"The cloud dot next to the host name, and the sign-in block in the host menu.
You can still sign in from Accounts." Keywords: `muxbus`, `cloud`, `dot`,
`status bar`, `sign in`, `statusbar:showmuxbuscloud`.

Placement: **Devices** (`devices-section.tsx`), under the existing Cloud
presence line (`cloud-presence.tsx`), because that is where a user looking at
cloud behaviour already is. Register it in that section's `*_SETTINGS`
registry so Settings search finds it (SPEC_SETTINGS_PANE_SEARCH_2026_09_21.md
§3.2). Use `ToggleControl` and `set("statusbar:showmuxbuscloud", v)` as the other
rows do.

### 3.5 What a user with it off loses

This is the real trade-off, so it is stated rather than left implicit. The dot
was made visible on the trigger so that a dead session shows at a glance
(comment at `HostPopover.tsx:365-368`: a popover-only state let a rollout sit
with WAN jekt delivery silently dead for hours). With the setting off, that
signal is gone from the status bar.

Decision: **respect the setting fully**, with no "show anyway when the
session is dead" override (see §6.2). The setting targets users who don't use
MuxBus, and an override would make it unreliable. The signal is not lost everywhere:

- Settings > Devices > Cloud presence already states "Sign in to publish" when
  signed out, and shows retry and refusal states.
- Sign-in, sign-out and the dead-session "Sign in again" state remain in the
  Accounts view (`AgentMuxConnectPanel`, mounted by `accounts-manager.tsx`).

The setting's description should say so in a short clause so nobody turns it
off believing a dead session will still be flagged. See §6 for the alternative.

## 4. Testing

Frontend (`HostPopover.test.tsx` already covers the dot and the block):

- Setting absent: dot and "MuxBus Cloud" block render as today (regression).
- `statusbar:showmuxbuscloud = false`: no `.status-muxbus-dot`; no "MuxBus Cloud"
  label, no Sign in button, no Disconnect, no keychain notice in the popover;
  the Ports section and its divider are still there, and there is no doubled
  divider.
- Flipping the setting at runtime shows or hides both without remounting.
- Disconnected and `NeedsReauth` states are both hidden when it is off.
- If polling is guarded: no `refresh()` call within 60 s while off; one call
  after it flips on.
- Settings: the toggle appears in Devices, reflects the stored value (absent =
  on), writes `statusbar:showmuxbuscloud`, and is found by searching "muxbus" and
  "cloud dot" (`settings-index.test.ts` pattern).

Backend:

- `types.rs` round-trip: `{"statusbar:showmuxbuscloud": false}` deserialises to
  `Some(false)`, and the default serialises to nothing.
- `SetConfigCommand` with the key persists and reaches `settingsAtom`.

Manual: toggle in a `task dev` build and watch the dot and popover change in
every open window.

## 5. Rollout

- One PR: Rust type, schema, `srv-types.d.ts`, `HostPopover.tsx`, the Devices
  row, tests, and a changeset (`scripts/changeset.sh`).
- No migration: absent already means shown.
- Public repo: this spec and the PR describe only the desktop UI. They name no
  cloud endpoints, account details or private repos.

## 6. Open questions

1. **Scope.** Resolved 2026-10-08: MuxBus **Cloud** only, i.e. the cloud dot and
   the cloud sign-in block (§1). LAN stays, and so does the Pair a device panel
   and its certificate fingerprint (`PairDevicePanel.tsx`), which is mobile
   pairing rather than the cloud.
2. **Dead-session override.** Resolved 2026-10-08: none. The setting is for
   users who don't use MuxBus at all, so a dead-session warning is noise to
   them; and an override would make the setting not do what it says.
3. **Key prefix.** Resolved: `statusbar:` (no `statusbar:*` clear key added; nothing needs it yet). Original question: `statusbar:showmuxbuscloud` (new prefix, recommended) or
   `window:showmuxbuscloud`.
4. **One setting or two.** Resolved: one. One keeps it simple; two (dot, popover block) lets
   someone keep the sign-in button in the menu and lose only the colour. Nothing
   in the request asks for the split, so one is the default.
