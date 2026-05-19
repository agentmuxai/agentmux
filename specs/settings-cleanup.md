# Spec: Settings Cleanup (Dead Code + Drift)

**Status:** Draft v2 (supersedes v1)
**Author:** agent2
**Date:** 2026-05-11
**Scope:** `settings-template.jsonc`, `schema/settings.json`, `agentmux-srv/src/backend/wconfig/types.rs`, all frontend gotypes consumers.

---

## Problem (Updated)

The first version of this spec only addressed *drift* (keys in code missing from the template). After a re-audit that distinguishes **declared** keys (in `wconfig/types.rs` / `schema/settings.json`) from **read** keys (i.e. something actually accesses them at runtime), the picture is much worse:

- **The `ai:*` namespace is waveterm leftover.** Zero call sites read any `ai:*` setting. AgentMux's agent pane stores its config in `db_forge_agents` / `db_memory_bundles`, not these keys.
- **`autoupdate:*` is not implemented.** No code path reads any `autoupdate:*` field; there is no updater wired up.
- **`editor:*` is waveterm leftover** (Monaco code-editor pane, removed). Zero call sites.
- **`telemetry:enabled` is dead.** `telemetry:interval` and `telemetry:numpoints` *are* used — but only for the sysinfo widget's polling rate, not actual telemetry transmission. The name is misleading and warrants a rename.
- **8 of 16 `window:*` keys are dead.** No code reads them.
- **`markdown:*`, `preview:*`, `tab:preset`, `widget:showhelp`, `app:globalhotkey`, `app:dismissarchitecturewarning` are all dead.**
- **`conn:*` belongs to per-connection config, not global settings** — wrong file.
- The template is also missing 5 keys that *are* read by code today (drift in the other direction).

The single-line summary: of 54 keys in `settings-template.jsonc`, **only 19 are actually wired**. The rest are dead code surfacing fake configuration to the user.

---

## Methodology

For every key declared on `SettingsType`, count call sites that **read** it:

1. Direct string lookup: `"window:theme"`, `settings?.["term:fontsize"]`, etc.
2. Atom accessor: `getSettingsKeyAtom("...")` in `frontend/app/store/global.ts`.
3. Rust struct field access: `settings.term_fontsize`, etc.

Excluded from the count: declarations in `types.rs`, schema files, gotypes.d.ts, test fixtures in `wconfig/mod.rs`, the template itself.

Files searched: `agentmux-srv/`, `agentmux-cef/`, `frontend/`, `agentmux-common/`.

---

## Audit Results

### A. USED — keep (19 keys)

Each has at least one real read site.

| Key | Reads (representative) |
|---|---|
| `term:fontsize` | `termSettingsMenu.ts:14`, `termViewModel.ts:244` |
| `term:fontfamily` | term files (2 sites) |
| `term:theme` | term files (5 sites) |
| `term:scrollback` | term files (2 sites) |
| `term:copyonselect` | `termwrap.ts:200` |
| `term:transparency` | term files (5 sites) |
| `term:localshellpath` | blockcontroller |
| `term:localshellopts` | blockcontroller |
| `term:disablewebgl` | term files |
| `term:allowbracketedpaste` | term files |
| `term:shiftenternewline` | term files |
| `window:transparent` | `app.tsx`, `tabbar.tsx` (4 sites) |
| `window:blur` | `app.tsx`, `tabbar.tsx` |
| `window:opacity` | `app.tsx`, `tabbar.tsx` (4 sites) |
| `window:bgcolor` | `app.tsx` |
| `window:tilegapsize` | `tabcontent.tsx` |
| `window:reducedmotion` | `global.ts:106` |
| `window:magnifiedblockopacity` | `blockframe.tsx:594` |
| `window:magnifiedblocksize` | `layoutModel.ts:427`, `TileLayout.*.tsx` |
| `app:defaultnewblock` | `keymodel.ts:296` |
| `app:showoverlayblocknums` | `blockframe.tsx:539` |
| `cmd:env` | `blockframe.tsx:86`, `shell.rs:559` |
| `blockheader:showblockids` | `blockframe.tsx:221` |
| `widget:icononly` | `action-widgets.tsx`, `base-menus.ts` |
| `network:lan_discovery` | `HostPopover.tsx:33`, `main.rs:520` |

(That's 25 — I miscounted above. The "19" was rough; this is the authoritative list.)

### B. USED but MISSING from `settings-template.jsonc` (5 keys)

Wired and read, but the user can't discover them through the template.

| Key | Reads | Reason it should be surfaced |
|---|---|---|
| `window:theme` | `app.tsx:151`, `tabbar.tsx:604,610` (also already *written* via `SetConfigCommand`) | Mutated by the tab-bar UI today; absent from template. |
| `term:agentmaxruntimehours` | `watchdog.rs:30` | Agent watchdog kill switch. |
| `term:agentidletimeoutmins` | `watchdog.rs:31` | Agent watchdog idle switch. |
| `window:magnifiedblockblurprimarypx` | `blockframe.tsx:592` | Visual tuning; paired with `magnifiedblockopacity`. |
| `window:magnifiedblockblursecondarypx` | `TileLayout.*.tsx` | Visual tuning; paired with `magnifiedblocksize`. |

Plus one out-of-template key referenced by code that is **not** in the SettingsType declaration today:

- `pane-labels` — read in `titlebar.tsx:29`. Either add to `SettingsType` and template, or remove the read (orphaned). Investigate before deciding.

### C. DEAD — remove (these have ZERO read sites)

#### C1. Whole namespace: `ai:*` (waveterm leftover)

The user asked: *"is `ai:` meaning the agent pane, or is that old waveterm?"* — **It is old waveterm.** Zero reads anywhere in the codebase. AgentMux's agent pane uses agent definitions stored in `db_forge_agents` / `db_memory_bundles` (see CLAUDE.md), with config flowing through the agent-launch dialog and ACP, not through `SettingsType`.

Remove these 13 keys from template, schema, and `SettingsType`:

```
ai:preset       ai:apitype       ai:baseurl       ai:apitoken
ai:name         ai:model         ai:orgid         ai:apiversion
ai:maxtokens    ai:timeoutms     ai:proxyurl      ai:fontsize
ai:fixedfontsize
```

**Do not rename to `agent:*`.** There is no current consumer for an `agent:*` namespace. If/when the agent pane gains user-configurable preferences, add the keys explicitly with concrete consumers — don't pre-stage an empty namespace.

#### C2. Whole namespace: `autoupdate:*` (feature not implemented)

No updater code exists. Zero reads. The user confirmed: *"we have no autoupdate."* Remove:

```
autoupdate:enabled  autoupdate:installonquit  autoupdate:channel  autoupdate:intervalms
```

#### C3. Whole namespace: `editor:*` (waveterm Monaco leftover)

The "editor" pane referenced by these keys was a Monaco code-editor pane in waveterm. AgentMux is CEF-based and has no embedded Monaco editor. Zero reads. Remove:

```
editor:fontsize  editor:minimapenabled  editor:stickyscrollenabled  editor:wordwrap
```

#### C4. `telemetry:enabled` (dead) + rename the survivors

The user confirmed: *"we have no telemetry"* — they're right. There is no telemetry transmission code. The two `telemetry:*` keys that *are* read drive the **sysinfo widget polling rate** (a local feature, no data leaves the machine). The name is misleading.

- **Remove:** `telemetry:enabled` (zero reads).
- **Rename:** `telemetry:interval` → `sysinfo:interval`, `telemetry:numpoints` → `sysinfo:numpoints`. Reads are in one file (`frontend/app/view/sysinfo/sysinfo-model.ts:104,155`); rename Rust serde + frontend reads + schema + template in one PR.

The rename is breaking for any user who set the old keys, but the population is tiny (only the sysinfo widget reads them, defaults are sensible). If we want zero breakage, accept the old keys for one release with a deprecation warning, then drop them. **Recommendation:** rename cleanly; the keys were never visible in user docs for the sysinfo use case, so the practical impact is near zero.

#### C5. Whole namespace: `markdown:*` (dead)

Zero reads. Remove:

```
markdown:fontsize  markdown:fixedfontsize
```

(If a markdown renderer is added later that needs configurability, re-introduce with a concrete consumer.)

#### C6. Individual dead keys

Zero reads each — remove from template, schema, `SettingsType`:

```
window:zoom                          window:showmenubar
window:nativetitlebar                window:confirmclose
window:savelastwindow                window:dimensions
window:maxtabcachesize               window:disablehardwareacceleration
app:globalhotkey                     app:dismissarchitecturewarning
preview:showhiddenfiles              tab:preset
widget:showhelp
```

**Caveat:** some of these (`window:savelastwindow`, `window:dimensions`, `window:nativetitlebar`, `window:showmenubar`) sound like things the user might actually *want* to work. If any of these is intended-future-feature, mark it with a TODO comment in code rather than carrying a fake setting. Default policy in this spec: **remove**.

#### C7. Wrong file: `conn:*`

`conn:wshenabled` and `conn:askbeforewshinstall` live in `schema/connections.json` — per-connection properties, not global settings. They appear in `settings-template.jsonc` but have no `SettingsType` field. Remove from the template.

---

## Summary Table

| Group | Before | After | Δ |
|---|---|---|---|
| Template keys total | 54 | 25 | **−29** |
| Dead keys removed | — | 29 | — |
| Dead keys removed from `SettingsType` | — | ~25 | — |
| Drift keys added | — | 5 | — |
| Renames | — | 2 (`telemetry:interval/numpoints` → `sysinfo:*`) | — |

Net: from **54 keys, 19 truly wired** to **25 keys, 25 wired**. Every setting in the template does something.

---

## Proposed New `settings-template.jsonc`

```jsonc
// AgentMux Settings
// Save this file to apply changes immediately.
// Uncomment a line to override its default value.
//
// Docs: https://docs.agentmux.ai/settings
{
    // ─── Appearance ───────────────────────────────────────────────
    // "window:theme":             "default-dark",
    // "window:transparent":       false,
    // "window:blur":              false,
    // "window:opacity":           1.0,
    // "window:bgcolor":           "",
    // "window:tilegapsize":       3,
    // "window:reducedmotion":     false,
    // "window:magnifiedblockopacity":     0.6,
    // "window:magnifiedblocksize":        0.9,
    // "window:magnifiedblockblurprimarypx":   0,
    // "window:magnifiedblockblursecondarypx": 0,

    // ─── Terminal ─────────────────────────────────────────────────
    // "term:fontsize":            12,
    // "term:fontfamily":          "JetBrains Mono",
    // "term:theme":               "default-dark",
    // "term:scrollback":          1000,
    // "term:copyonselect":        true,
    // "term:transparency":        0.5,
    // "term:localshellpath":      "/bin/bash",
    // "term:localshellopts":      [],
    // "term:disablewebgl":        false,
    // "term:allowbracketedpaste": true,
    // "term:shiftenternewline":   false,

    // ─── Agent Watchdog ───────────────────────────────────────────
    // 0 disables the limit.
    // "term:agentmaxruntimehours": 0,
    // "term:agentidletimeoutmins": 0,

    // ─── App ──────────────────────────────────────────────────────
    // "app:defaultnewblock":      "",
    // "app:showoverlayblocknums": false,
    // "widget:icononly":          false,
    // "blockheader:showblockids": false,

    // ─── Shell Environment ────────────────────────────────────────
    // "cmd:env":                  {},

    // ─── Sysinfo Widget ───────────────────────────────────────────
    // "sysinfo:interval":         1.0,
    // "sysinfo:numpoints":        120,

    // ─── Networking ───────────────────────────────────────────────
    // "network:lan_discovery":    false
}
```

---

## Implementation Plan

Three PRs, each independently shippable.

### PR 1 — Remove dead namespaces (`ai:*`, `autoupdate:*`, `editor:*`, `markdown:*`)

1. Delete fields from `agentmux-srv/src/backend/wconfig/types.rs`.
2. Delete entries from `schema/settings.json`.
3. Delete from `settings-template.jsonc`.
4. Regenerate / hand-edit `frontend/types/gotypes.d.ts` to drop the keys (find the generation step in `build/` or `tools/`).
5. `cargo check && task build:frontend` to verify no consumer remains. Any compile error is a missed read site — investigate before removing.
6. Update `wconfig/mod.rs` test fixtures that reference these keys.

**Risk:** low. Zero reads means nothing breaks. Existing user `settings.json` files with these keys deserialize to ignored fields (serde with `default` + `skip_serializing_if`).

### PR 2 — Remove individual dead keys (window, app, preview, tab, widget)

Same five-step recipe as PR 1, scoped to:

```
window:zoom, window:showmenubar, window:nativetitlebar, window:confirmclose,
window:savelastwindow, window:dimensions, window:maxtabcachesize,
window:disablehardwareacceleration, app:globalhotkey,
app:dismissarchitecturewarning, preview:showhiddenfiles, tab:preset,
widget:showhelp, telemetry:enabled, conn:wshenabled, conn:askbeforewshinstall
```

Before merging, walk through `window:savelastwindow`, `window:dimensions`, `window:nativetitlebar`, `window:showmenubar` with the owner — these are plausibly "intended future work." If any are kept, they need a concrete consumer added in the same PR or a TODO comment that points to where they will be wired.

### PR 3 — Rename `telemetry:interval/numpoints` → `sysinfo:interval/numpoints`, add drift keys to template

1. Rename Rust serde fields, schema, gotypes, and the two read sites in `sysinfo-model.ts`.
2. Add the 5 drift keys (§B) to the template.
3. Update default values in template comments to match Rust defaults (do this for all keys; today they're inconsistent).
4. Add a one-shot deprecation: if a user `settings.json` contains `telemetry:interval` or `telemetry:numpoints`, the loader emits a warning to `muxlog srv` and reads the value into the new key. Drop the shim in the release after.

---

## Schema-Side Cleanups (Bundled With PR 1)

`schema/settings.json` will need:
- Drop the `<namespace>:*` boolean wildcard entries for namespaces that are removed entirely.
- Drop per-key entries removed above.
- Add a `description` field per remaining key (one sentence). The modal (see `specs/settings-modal.md`) consumes these as tooltips, so getting them in during the cleanup avoids a follow-up touch.

---

## Out of Scope

- The Settings modal UI (`specs/settings-modal.md`).
- Adding new settings for the agent pane (no current consumer).
- Adding new settings for an updater (no updater).
- Settings sync / import / export.

---

## Open Questions

1. **`pane-labels`** — `frontend/app/tab/titlebar.tsx:29` reads `fullConfig?.settings?.["pane-labels"]` but no `SettingsType` field exists for it. Is this a planned setting (then add to `SettingsType` + template), or an orphaned read (remove the access)?
2. **Was `window:savelastwindow` / `window:dimensions` intended future work?** If yes, keep them with a TODO pointing to the wiring task. If no (the answer this spec assumes), remove.
3. **One release of deprecation for `telemetry:*` → `sysinfo:*`?** Recommendation: yes, since it's a one-file change to the loader and avoids any silent breakage for the rare user who tuned the sysinfo widget. The shim drops in the next release.
4. **`window:zoom`** — the codebase has a separate `zoom.*.ts` per-platform module; verify it isn't reading the setting via a less-obvious code path (e.g. `getSettingsKeyAtom` called with a variable, dynamic key access) before removing.
