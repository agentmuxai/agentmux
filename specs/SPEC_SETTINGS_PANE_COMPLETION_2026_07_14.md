# SPEC — Settings pane: fill out the remaining sections (completes SPEC_SETTINGS_PANE_2026_06_25)

**Date:** 2026-07-14
**Author:** Agent2
**Status:** Draft
**Scope:** `frontend/app/view/settings/` (the pane itself), `schema/settings.json` (audit only, no schema changes proposed), `CLAUDE.md` (one stale line).
**Related (must-read first):**
`specs/SPEC_SETTINGS_PANE_2026_06_25.md` — the original spec this one completes. Phases 1–2 of that spec are **done and live**; this spec covers Phases 3–5, and **corrects the settings catalog**, which has drifted significantly since 2026-06-25 (see §2).

---

## 1. Summary

The Settings pane exists, is registered as a widget, and opens correctly from the hamburger menu — the original spec's Phases 1–2 are fully implemented (`settings-model.ts`, `settings-view.tsx`, `settings.scss`, `settings.tsx` barrel). Two of the planned seven sections are wired end-to-end: **Appearance** and **Terminal**. The other five (`Agent`, `Sounds`, `Network`, `Files`, `Advanced`) are `<StubSection>` placeholders reading "coming soon."

This spec does two things:
1. **Re-audits the full `schema/settings.json` catalog against the current codebase.** The original spec's catalog (written 2026-06-25) is now significantly stale: it references keys that don't exist today (`voice:*`, `dnd:*`, `messaging:discord:*`, `term:agentmaxruntimehours`, `term:agentidletimeoutmins` — none are in the current schema) and is silent on ~25 keys that do exist. §2 is a corrected, current, key-by-key catalog.
2. **Proposes a revised section structure** (6 sections instead of 7 — see §3) and a phased implementation plan to wire the remaining ~46 keys (§4–§5), reusing the exact primitives (`SettingRow`, `ToggleControl`, `SliderControl`, the `set()` helper) already built and proven in the Appearance/Terminal sections.

## 2. Current settings catalog — corrected, as of this repo's `main`

`schema/settings.json` currently defines **60 real keys** (excludes JSON-Schema structural keywords like `properties`/`additionalProperties`). Status column: **✅ live** (wired in the pane today), **NEW** (exists in schema, not in the original spec's catalog at all), **DEAD** (in schema, zero frontend consumers — flagging, not proposing removal from schema here), **GONE** (was in the original spec's catalog, doesn't exist in schema anymore).

| Key | Status | Notes |
|---|---|---|
| `window:theme` | ✅ live | Appearance |
| `window:transparent` | ✅ live | Appearance |
| `window:opacity` | ✅ live | Appearance |
| `window:blur` | ✅ live | Appearance |
| `window:tilegapsize` | ✅ live | Appearance |
| `window:reducedmotion` | ✅ live | Appearance |
| `term:fontsize` | ✅ live | Terminal |
| `term:fontfamily` | ✅ live | Terminal |
| `term:theme` | ✅ live | Terminal |
| `term:scrollback` | ✅ live | Terminal |
| `term:copyonselect` | ✅ live | Terminal |
| `term:shiftenternewline` | ✅ live | Terminal |
| `term:allowbracketedpaste` | ✅ live | Terminal |
| `term:transparency` | ✅ live | Terminal |
| `window:bgcolor` | NEW | 1 frontend consumer (`blockframe.tsx`) — custom background color override |
| `window:magnifiedblockopacity` | NEW | Pane hover-magnify feature, `blockframe.tsx` |
| `window:magnifiedblocksize` | NEW | same feature |
| `window:magnifiedblockblurprimarypx` | NEW | same feature |
| `window:magnifiedblockblursecondarypx` | NEW | same feature |
| `window:showmenubar` | NEW | 0 frontend consumers — launcher/host-consumed |
| `window:nativetitlebar` | NEW | 0 frontend consumers — restart-required, host-consumed |
| `window:disablehardwareacceleration` | NEW | 0 frontend consumers — restart-required, host-consumed; **was** in old spec's Advanced |
| `window:maxtabcachesize` | NEW | 0 frontend consumers — host-consumed |
| `window:confirmclose` | NEW | 0 frontend consumers — host-consumed |
| `window:savelastwindow` | NEW | 0 frontend consumers — host-consumed |
| `window:dimensions` | NEW | 0 frontend consumers — host-consumed, likely not user-facing (window geometry string, probably write-only from the app itself) — **candidate to exclude from UI entirely**, see §6 |
| `window:zoom` | NEW | 0 frontend consumers — host-consumed |
| `term:disablewebgl` | ✅ old spec (Advanced) | 1 consumer |
| `term:localshellpath` | ✅ old spec (Advanced) | 0 frontend consumers — host-consumed |
| `term:localshellopts` | NEW | 0 frontend consumers — host-consumed, array-of-strings (shell args) |
| `term:predictiveecho` | NEW | 1 consumer — local predictive echo for laggy/remote shells |
| `term:predictiveecho:thresholdms` | NEW | companion to above |
| `app:globalhotkey` | ✅ old spec (Advanced) | 0 frontend consumers — host-consumed (global show/hide hotkey) |
| `app:dismissarchitecturewarning` | NEW | 0 frontend consumers — one-shot dismiss flag, **exclude from UI**, see §6 |
| `app:defaultnewblock` | NEW | 1 consumer — which pane type a new tab/block defaults to |
| `app:showoverlayblocknums` | NEW | 1 consumer — numbered-overlay pane-jump feature (like tmux prefix-number) |
| `cmd:env` | ✅ old spec (Advanced) | 8 consumers — global env vars injected into every shell; key-value editor control needed |
| `blockheader:showblockids` | NEW | 1 consumer — debug aid, show block/pane IDs in headers |
| `preview:showhiddenfiles` | ✅ old spec (Files) | file picker |
| `tab:preset` | **DEAD** | 0 frontend consumers anywhere (Rust type + generated TS type only) — **do not build UI for this**, see §6 |
| `tab:skipcloseconfirm` | NEW | 1 consumer |
| `widget:showhelp` | NEW | 0 frontend consumers — host/widget-bar-consumed |
| `widget:icononly` | NEW | 2 consumers — forces icon-only widget-bar labels (CLAUDE.md's widget table references this) |
| `telemetry:enabled` | ✅ old spec (Advanced) | 1 consumer |
| `telemetry:interval` | NEW | 1 consumer |
| `telemetry:numpoints` | NEW | 1 consumer, has schema `min/max/default` (30–1024, default 120) |
| `conn:askbeforewshinstall` | GONE from old spec's catalog, but real | old spec excluded `conn:*` assuming a separate "Connections (WSH)" panel exists. **It doesn't** — confirmed no such surface exists anywhere in the current frontend. These are homeless; bring them into Settings. |
| `conn:wshenabled` | same as above | |
| `network:lan_discovery` | ✅ live in old spec's catalog, not yet wired | Network section, still stubbed |
| `notify:sounds:enabled` | ✅ old spec (Sounds) | still stubbed |
| `notify:sounds:volume` | ✅ old spec (Sounds) | still stubbed |
| `notify:sounds:suppresswhenfocused` | ✅ old spec (Sounds) | still stubbed |
| `notify:sound:agent.turn.complete` | ✅ old spec (Sounds) | still stubbed |
| `notify:sound:agent.turn.error` | ✅ old spec (Sounds) | still stubbed |
| `notify:sound:agent.turn.interrupted` | ✅ old spec (Sounds) | still stubbed |
| `notify:sound:agent.message.accepted` | NEW | added after 2026-06-25 |
| `notify:sound:agent.message.rejected` | NEW | added after 2026-06-25 |
| `notify:tooltones:enabled` | ✅ old spec (Sounds) | still stubbed |
| `notify:tooltones:volume` | ✅ old spec (Sounds) | still stubbed |
| `notify:tooltones:scope` | ✅ old spec (Sounds) | still stubbed, enum `["all","focused"]` |

**GONE entirely (in the old spec's catalog, not in current schema — do not build UI for these; the old spec's "Agent" section and part of "Files" were built on keys that were apparently never shipped, or were removed):**
`voice:enabled`, `voice:engine`, `voice:whisperModel`, `voice:whisperCliPath`, `voice:groqApiKey`, `term:agentmaxruntimehours`, `term:agentidletimeoutmins`, `dnd:enabled`, `dnd:maxfilesizemb`, `dnd:agentinserttoken`, `messaging:discord:enabled`, `messaging:discord:channel`, `messaging:discord:target`, `messaging:discord:token`.

This means the old spec's entire **Section 3 (Agent)** and most of **Section 6 (Files)** and **Section 7's Discord bridge item** describe features that don't exist in this codebase today. Also notable: **there are currently zero secret/token-type keys in the schema** — the old spec's "masked password field" work item (§Component Architecture, "Secret keys") has nothing to apply to right now. Not proposing to remove that from the plan (see §5, Phase 5) since it's cheap infrastructure to have ready, but it's no longer load-bearing for shipping the rest of the pane.

## 3. Revised section structure

The original 7-section plan (`Appearance, Terminal, Agent, Sounds, Network, Files, Advanced`) doesn't fit the corrected catalog well: `Agent` has nothing left to put in it, `Files` would have exactly one key (`preview:showhiddenfiles`), and `Advanced` would end up holding ~25 keys in one flat list — including a dozen `window:*` behavior keys that have nothing to do with "advanced/power-user" as a concept, they're just window behavior that never got its own home.

**Proposed: 6 sections**, dropping `Agent` and `Files` as standalone rail items, adding `Window & Panes`:

1. **Appearance** (unchanged, already live) — `window:theme`, `window:transparent`, `window:opacity`, `window:blur`, `window:tilegapsize`, `window:reducedmotion`. **Add:** `window:bgcolor`, the 4 `window:magnifiedblock*` keys — all visual, all fit naturally here.
2. **Window & Panes** *(new section)* — `window:showmenubar`, `window:nativetitlebar`, `window:disablehardwareacceleration`, `window:maxtabcachesize`, `window:confirmclose`, `window:savelastwindow`, `window:zoom`, `blockheader:showblockids`, `app:defaultnewblock`, `app:showoverlayblocknums`, `tab:skipcloseconfirm`. Window/pane *behavior*, distinct from Appearance's pure visuals. (`window:dimensions` excluded — see §6.)
3. **Terminal** (unchanged, already live) — existing 8 keys. **Add:** `term:predictiveecho`, `term:predictiveecho:thresholdms` (a normal terminal-feel setting, not power-user-only).
4. **Sounds & Notifications** — all 11 `notify:*` keys, including the 2 the old spec missed.
5. **Network** — `network:lan_discovery`, `conn:askbeforewshinstall`, `conn:wshenabled`. (Old spec's reason for excluding `conn:*` — a separate Connections/WSH panel — doesn't exist; confirmed via search.)
6. **Advanced** — the true power-user/restart-required catch-all: `app:globalhotkey`, `term:disablewebgl`, `term:localshellpath`, `term:localshellopts`, `widget:showhelp`, `widget:icononly`, `telemetry:enabled`, `telemetry:interval`, `telemetry:numpoints`, `cmd:env` (key-value editor), `preview:showhiddenfiles` (folded in from the old spec's now-empty "Files" section — a single file-picker toggle doesn't warrant its own rail item).

`app:dismissarchitecturewarning`, `window:dimensions`, `tab:preset` are deliberately **not surfaced anywhere** — see §6 for why each.

**Rail becomes:** Appearance · Window & Panes · Terminal · Sounds · Network · Advanced (6 items, was 7 — `RAIL` array in `settings-view.tsx` and `SettingsSection` union in `settings-model.ts` both need the rename/restructure).

## 4. Component work needed

Everything needed for sections 1–5 above (Appearance additions, Window & Panes, Terminal additions, Sounds, Network) is a straight application of the **existing** `SettingRow` / `ToggleControl` / `SliderControl` / `set()` primitives — no new components. This is most of the remaining work and is low-risk, mechanical.

**New primitives needed only for Advanced (Phase 4-equivalent):**
- **Key-value editor** for `cmd:env` (add/remove rows, each a `key` + `value` text pair). No existing pattern to reuse verbatim; build as a new control alongside `SettingRow`/`ToggleControl` in `settings-view.tsx` (or split into `settings-controls.tsx` once the file grows — `settings-view.tsx` is already 407 lines before this work).
- **String-array editor** for `term:localshellopts` (shell launch args) — same shape as the key-value editor, simpler (single column, add/remove rows).
- **Masked/reveal password field** — infrastructure only, nothing in the current schema needs it (see §2). A reusable pattern already exists elsewhere in the app (`OAuthConnectPanel.tsx`, `AgentNewIdentityModal.tsx`, `BrowserAuthModal.tsx`, `identity-view.tsx`) — if this is built, crib the pattern from one of those rather than inventing a new one. Low priority; only build if/when a secret-type key actually lands in the schema.

## 5. Implementation sequence

Numbering continues from the original spec's Phase 1–2 (already shipped):

**Phase 3 — Window & Panes, Sounds, Network sections** (all straight `SettingRow` application, no new components)
- Update `SettingsSection` union (`settings-model.ts`) and `RAIL` array (`settings-view.tsx`): drop `agent`/`files`, add `window` (or similar id).
- Wire `window:bgcolor` + 4 `magnifiedblock*` keys into `AppearanceSection`.
- New `WindowPanesSection` component: 11 keys from §3.2.
- New `SoundsSection` component: 11 `notify:*` keys. `notify:sounds:volume`/`notify:tooltones:volume` use `SliderControl` (0–1, matching `term:transparency`'s existing usage); `notify:sounds:enabled` gates the rest of the section visible/disabled the same way `AppearanceSection` gates opacity/blur behind `window:transparent` today (established pattern, reuse it). `notify:tooltones:scope` is the first `enum`-as-`<select>` outside `window:theme`/`term:theme` — same `<select>` pattern.
- New `NetworkSection` component: 3 keys.
- Add `term:predictiveecho` + `term:predictiveecho:thresholdms` to the existing `TerminalSection` (indent the threshold row under the toggle, same pattern already used for opacity/blur under `window:transparent`).

**Phase 4 — Advanced section**
- Key-value editor control for `cmd:env`.
- String-array editor for `term:localshellopts`.
- Remaining flat `SettingRow` keys: `app:globalhotkey`, `term:disablewebgl`, `term:localshellpath`, `widget:showhelp`, `widget:icononly`, `telemetry:enabled`/`interval`/`numpoints`, `preview:showhiddenfiles`.
- `telemetry:numpoints` should respect the schema's declared `minimum: 30, maximum: 1024` bounds in its `<input type="number">`, same as how `term:scrollback`/`term:fontsize` already clamp to their schema ranges today.

**Phase 5 — Polish** (from the original spec, still valid, still not started)
- `settings:section` persistence in block/wave meta — `activeSection` is currently a plain `createSignal` (`settings-model.ts:31`), resets to `"appearance"` on every pane remount. Low effort, same pattern as other per-pane persisted UI state elsewhere in the codebase (e.g. `AgentPaneState.detailsOpen`).
- Theme preview swatch next to the `window:theme` selector.
- Search/filter across all settings keys (all ~46 remaining + the 14 already live) — becomes meaningfully useful only once the catalog is this large; sequencing it last (after Phases 3–4 land the rest of the keys) makes sense rather than building search against a mostly-stub pane.
- Masked/reveal secret-field infrastructure — deprioritize per §2/§4 (nothing needs it yet).

## 6. Keys deliberately excluded from the pane

- **`tab:preset`** — zero frontend consumers anywhere; only exists in the Rust type (`agentmux-srv/src/backend/wconfig/types.rs:117`) and generated TS types. Building a UI control for a key nothing reads would be actively misleading (the toggle would appear to do something and do nothing). Flag to the team as a possible dead-schema-entry cleanup; not this spec's job to remove it, just not to build UI pretending it's live.
- **`window:dimensions`** — a window-geometry string, 0 frontend consumers; almost certainly written by the app itself on close/resize (persisting where to reopen), not something a user hand-edits via a settings form. A raw string field for "window dimensions" would invite users to break their own window placement with a typo for no benefit over just resizing the window normally.
- **`app:dismissarchitecturewarning`** — a one-shot internal dismissal flag (analogous to "don't show this again" checkboxes elsewhere), not a preference someone would deliberately go into Settings to toggle back on. If a way to re-arm the warning is ever wanted, it belongs as a specific action/button tied to that warning's own UI, not a generic Settings toggle.

These three are the exceptions; every other key in the schema gets a home somewhere in §3.

## 7. Files affected

| File | Change |
|---|---|
| `frontend/app/view/settings/settings-model.ts` | `SettingsSection` union: drop `"agent"`/`"files"`, add `"window"` (Window & Panes) |
| `frontend/app/view/settings/settings-view.tsx` | `RAIL` array update; new `WindowPanesSection`, `SoundsSection`, `NetworkSection`, `AdvancedSection` components (replacing their `StubSection` placeholders); `AppearanceSection`/`TerminalSection` extended with the new keys from §3; new key-value/string-array editor controls |
| `frontend/app/view/settings/settings-controls.tsx` | **New, optional** — split out of `settings-view.tsx` once it grows past ~600 lines with the new controls, to keep the file navigable (judgment call at implementation time, not a hard requirement) |
| `frontend/app/view/settings/settings.scss` | New styles for key-value editor rows, string-array editor rows |
| `CLAUDE.md` | "Settings" row in the "Not widgets" table still says "Opens `settings.json` in the user's default editor" — stale since Phase 1 shipped; the raw-JSON open is now a footer link inside the pane, not the primary action. One-line fix, unrelated to the rest of this spec's scope but should be corrected whenever this work lands. |

## 8. Out of scope (unchanged from the original spec)

Everything the original spec scoped out is still out of scope for the same reasons: tab-level settings (context menus, not global — and per §2, `tab:preset` turns out not to even be wired, reinforcing this), widget pinning UI (right-click on the widget bar), agent-level overrides (Identity/Bundles system), import/export (raw-JSON escape hatch covers it).
