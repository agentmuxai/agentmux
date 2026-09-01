# SPEC — Terminal scroll wheel sensitivity setting

**Status:** proposed → implementing
**Date:** 2026-08-31
**Author:** agent3
**Related:**
- `frontend/app/view/term/termwrap.ts:137` (`new Terminal({...})` construction)
- `frontend/app/view/term/termwrap.ts:206-210` (`attachCustomWheelEventHandler` — macOS momentum-scroll debounce + Ctrl+Wheel passthrough)
- `frontend/app/view/settings/sections/terminal-section.tsx` (Terminal settings UI section)
- `agentmux-srv/src/backend/wconfig/types.rs` (`SettingsType`, Rust-side settings struct)
- `schema/settings.json`, `frontend/types/gotypes.d.ts`, `settings-template.jsonc` (settings schema/type/template, hand-maintained in parallel per existing convention)

---

## 1. Why

A user asked whether AgentMux lets them control how many lines the mouse wheel
scrolls inside a terminal pane, and whether that can override the OS-level
setting (Windows Control Panel → Mouse → "lines to scroll").

Investigation found:

- Terminal wheel scrolling is handled entirely by xterm.js's own internal
  wheel-to-line logic. AgentMux's only customization
  (`attachCustomWheelEventHandler`, `termwrap.ts:206-210`) exists purely to
  suppress macOS trackpad momentum-scroll jitter and to let Ctrl+Wheel fall
  through to the zoom handler — it never touches line count.
- xterm.js's `scrollSensitivity` option (multiplier on scroll-wheel delta,
  library default `1`) is never set anywhere in this codebase, so every
  terminal pane scrolls at xterm.js's built-in default rate.
- The app does **not** read the OS "lines per scroll" setting
  (`SPI_GETWHEELSCROLLLINES` or equivalent) at all — terminal scroll speed is
  already fully independent of the OS setting, just not configurable by the
  user today.
- No existing settings key or UI control exists for this.

This spec adds a user-facing setting that maps directly onto xterm.js's
`scrollSensitivity`, so the user can make terminal-pane scrolling faster or
slower than both xterm.js's default and whatever the OS is configured to.

## 2. Goals

- A new setting, `term:scrollsensitivity`, a positive number multiplier
  (default `1`, matching xterm.js's own default so existing behavior is
  unchanged out of the box).
- Applied to `scrollSensitivity` in the `new Terminal({...})` options passed
  in `termwrap.ts`'s constructor.
- A control in the Terminal settings section, next to the other terminal
  tuning knobs (font size, scrollback, transparency).
- Since this is an `ITerminalOptions` (not `ITerminalInitOnlyOptions`) field,
  xterm.js supports updating it live via `terminal.options.scrollSensitivity
  = ...` without recreating the terminal — but plumbing a live-update path
  for every open pane is out of scope for this pass (see §4). The setting
  takes effect for panes opened/reloaded after the change, consistent with
  how `term:fontfamily`/`term:theme` already behave (checked at construction
  time only).

## 3. Non-goals

- No attempt to read or mirror the OS-level "lines per scroll" setting. The
  app already doesn't consult it; this spec doesn't change that, it only adds
  an independent app-level knob.
- No separate control for `fastScrollSensitivity` (the modifier-held fast
  scroll multiplier) — one setting is enough to answer the user's ask; a
  second knob can be added later if requested.
- No live-reload of already-open terminal panes when the setting changes
  (see §2's note) — same limitation as other construction-time terminal
  settings already in this file.

## 4. Design

### 4.1 Setting

`term:scrollsensitivity` — `number`, optional, default `1` when absent.
Range enforced in the settings UI: `0.1`–`10` (mirrors the shape of
`term:transparency`'s bounded slider-adjacent number input, though this uses
a plain number input like `term:fontsize` since the useful range spans more
than 0–1).

### 4.2 Frontend wiring

`termwrap.ts`'s constructor reads the setting via `getSettingsKeyAtom` (the
same pattern used for `term:predictiveecho` in `init()`) and passes it
through to the `Terminal` constructor:

```ts
const scrollSensitivity = getSettingsKeyAtom("term:scrollsensitivity")();
this.terminal = new Terminal({
    ...options,
    cursorBlink: false,
    scrollOnUserInput: false,
    smoothScrollDuration: 0,
    scrollSensitivity: typeof scrollSensitivity === "number" && scrollSensitivity > 0 ? scrollSensitivity : 1,
});
```

### 4.3 Settings UI

Add a `SettingRow` in `terminal-section.tsx`, placed after "Terminal
transparency" (grouped with the other numeric tuning controls, before the
predictive-echo toggle group):

```tsx
<SettingRow
    label="Scroll sensitivity"
    description="Scroll wheel speed multiplier for terminal panes (0.1–10, default 1). Independent of the OS scroll-speed setting."
    control={
        <input
            class="setting-number setting-number--wide"
            type="number" min={0.1} max={10} step={0.1}
            value={(s()["term:scrollsensitivity"] as number) ?? 1}
            onBlur={(e) => {
                const v = parseFloat(e.currentTarget.value);
                if (!isNaN(v) && v >= 0.1 && v <= 10) set("term:scrollsensitivity", v);
            }}
        />
    }
/>
```

### 4.4 Schema / type / template updates

Per this repo's existing convention (no generator ties these together — each
is hand-maintained and kept in sync manually), add the new key to all of:

- `agentmux-srv/src/backend/wconfig/types.rs` — `pub term_scroll_sensitivity: Option<f64>` with `#[serde(rename = "term:scrollsensitivity", ...)]`.
- `frontend/types/gotypes.d.ts` — `"term:scrollsensitivity"?: number;` in `SettingsType`.
- `schema/settings.json` — `"term:scrollsensitivity": { "type": "number", "minimum": 0.1, "maximum": 10, "default": 1, "description": "..." }`.
- `settings-template.jsonc` — commented-out example line under `-- Terminal --`.

## 5. Alternatives considered

- **Reading `SPI_GETWHEELSCROLLLINES` on Windows and deriving a default from
  it.** Rejected: adds a platform-specific API call for a value that xterm.js
  doesn't consume in the same units (xterm.js's sensitivity is a multiplier
  on wheel delta, not a discrete line count), and the user's actual ask was
  for an in-app override, not OS mirroring.
- **Exposing `fastScrollSensitivity` too.** Deferred — keeps this change
  minimal; can be added the same way later if wanted.

## 6. Testing

- Manual: set `term:scrollsensitivity` to `0.3` and `3` in Settings, open a
  new terminal/agent pane, verify wheel scroll is visibly slower/faster than
  default.
- Verify absent setting (fresh install / unset) still scrolls at the
  unchanged xterm.js default rate (regression check).
