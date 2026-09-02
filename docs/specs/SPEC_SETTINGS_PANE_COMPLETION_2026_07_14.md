# SPEC — Settings pane: fill out the remaining sections (completes SPEC_SETTINGS_PANE_2026_06_25)

**Date:** 2026-07-14
**Author:** Agent2
**Status:** Implemented (PR #2162), revised post-review — see correction note below
**Scope:** `frontend/app/view/settings/` (the pane itself), `schema/settings.json` (audit only, no schema changes proposed), `CLAUDE.md` (one stale line).
**Related (must-read first):**
- `specs/SPEC_SETTINGS_PANE_2026_06_25.md` — the original spec this one completes. Phases 1–2 of that spec are **done and live**; this spec covers Phases 3–5, and **corrects the settings catalog**, which had drifted significantly since 2026-06-25 (see §2).
- `specs/settings-cleanup.md` — a pre-existing (2026-05-11) dead-key audit of the same schema that this spec's first draft failed to cross-reference. It is authoritative on which keys have real read sites; see the correction note.

## Correction note (post-review, 2026-07-15)

The first draft of this spec (and the PR built from it) checked keys for **frontend** consumers only and assumed a `0 frontend consumers` key was legitimately "host-consumed" by Rust/launcher code. That assumption was wrong for most of them. `specs/settings-cleanup.md` had already done the harder audit — checking Rust struct **field reads**, not just struct **field declarations** — and found that most of these keys deserialize into a struct field that is never subsequently read anywhere. A serde field exists only to round-trip the JSON; nothing acts on the value. An automated PR review caught this (confirmed independently against the Rust source: `#[allow(dead_code)]` annotations on the `term:localshellpath`/`term:localshellopts` constants were the give-away that not everything with a struct field is actually wired, and grepping for the snake_case field names beyond `wconfig/types.rs` turned up zero hits for the flagged keys).

**Reclassified from "NEW, needs UI" to DEAD (zero reads beyond struct declaration) — removed from the shipped UI:**
`window:showmenubar`, `window:nativetitlebar`, `window:disablehardwareacceleration`, `window:maxtabcachesize`, `window:confirmclose`, `window:savelastwindow`, `window:zoom`, `app:globalhotkey`, `widget:showhelp`, `telemetry:enabled`, `preview:showhiddenfiles`.

**Reclassified as wrong-file (schema/settings.json declares them but `SettingsType` has no matching Rust struct field; the real, live versions are per-connection fields in `schema/connections.json`) — removed:**
`conn:wshenabled`, `conn:askbeforewshinstall`.

**Reclassified as UX duplicate (real and live, but already has an established home) — removed from Settings, left in place at its existing location:**
`network:lan_discovery` — already toggleable from `frontend/app/statusbar/HostPopover.tsx`. Adding a second, disconnected control for the same key in Settings was flagged by review as confusing rather than helpful; deferred rather than shipped.

Losing `network:lan_discovery` and both `conn:*` keys left the **Network** section with nothing in it, so it was dropped entirely — same reasoning §3 already used to drop the old spec's **Files** section for being down to one key. The pane now has **5** sections, not 6. `telemetry:interval`/`telemetry:numpoints` survived (real consumers — see `specs/settings-cleanup.md` §A — they drive the sysinfo widget's polling rate, not literal telemetry transmission, hence the relabel to "Sysinfo widget" in the UI to stop implying data leaves the machine).

**Second correction (same review cycle):** `term:localshellpath`/`term:localshellopts` were initially kept as "real" because `agentmux-srv/src/backend/blockcontroller/mod.rs` declares matching `META_KEY_TERM_LOCAL_SHELL_PATH`/`META_KEY_TERM_LOCAL_SHELL_OPTS` constants — but a constant existing is not the same as it being read. Both are annotated `#[allow(dead_code)]`, and the actual shell-spawn path (`blockcontroller/shell/lifecycle.rs:252`) calls `detect_local_shell_path_windows()` unconditionally on Windows (or reads `$SHELL` on Unix) — neither setting is ever consulted. Removed both from the UI, which also removed `StringArrayEditor`'s only caller; deleted the component entirely (as dead code) rather than leaving it unused, which incidentally also resolved a real bug review caught in it: keying `<For>` by primitive string value meant duplicate array entries (e.g. two identical shell flags) would collide and misdirect edit/remove.

**Standing lesson for future settings-pane work:** a Rust `pub const META_KEY_*` or a `#[serde(rename = "...")]` struct field is evidence a key **deserializes**, not evidence it's **read**. Always grep for the field/const's *usage* site beyond its own declaration before wiring UI for it, and check for `#[allow(dead_code)]` as a hard signal.

The rest of this document is left as originally written for the historical record of the (partially wrong) research; §2/§3/§5/§7 below carry inline strikethrough-style corrections rather than being silently rewritten.

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
| `window:showmenubar` | **DEAD** (corrected post-review) | Rust struct field exists in `types.rs`, never read anywhere else. Not shipped. |
| `window:nativetitlebar` | **DEAD** (corrected post-review) | same — not shipped |
| `window:disablehardwareacceleration` | **DEAD** (corrected post-review) | same — not shipped |
| `window:maxtabcachesize` | **DEAD** (corrected post-review) | same — not shipped |
| `window:confirmclose` | **DEAD** (corrected post-review) | same — not shipped |
| `window:savelastwindow` | **DEAD** (corrected post-review) | same — not shipped |
| `window:dimensions` | NEW | 0 frontend consumers — host-consumed, likely not user-facing (window geometry string, probably write-only from the app itself) — **excluded from UI**, see §6 |
| `window:zoom` | **DEAD** (corrected post-review) | Rust struct field exists, never read — not shipped |
| `term:disablewebgl` | ✅ old spec (Advanced) | 1 consumer — shipped in Advanced |
| `term:localshellpath` | **DEAD** (corrected post-review, round 2) | const is declared `#[allow(dead_code)]` in `blockcontroller/mod.rs`; the Windows shell-spawn path ignores it entirely (`detect_local_shell_path_windows()`, unconditional). Not shipped. |
| `term:localshellopts` | **DEAD** (corrected post-review, round 2) | same finding as above. Not shipped; `StringArrayEditor` (its only caller) deleted as dead code. |
| `term:predictiveecho` | NEW | confirmed live (`termwrap.ts`, `predictive-echo.ts`) — shipped in Terminal |
| `term:predictiveecho:thresholdms` | NEW | companion to above — shipped |
| `app:globalhotkey` | **DEAD** (corrected post-review) | Rust struct field exists, never read — not shipped |
| `app:dismissarchitecturewarning` | NEW | 0 frontend consumers — one-shot dismiss flag, **excluded from UI**, see §6 |
| `app:defaultnewblock` | NEW | confirmed live (`keymodel.ts:296`) — shipped in Window & Panes |
| `app:showoverlayblocknums` | NEW | confirmed live (`blockframe.tsx:539`) — shipped in Window & Panes |
| `cmd:env` | ✅ old spec (Advanced) | confirmed live (`blockframe.tsx`, `shell.rs`) — shipped in Advanced with a key-value editor |
| `blockheader:showblockids` | NEW | confirmed live (`blockframe.tsx:221`) — shipped in Window & Panes |
| `preview:showhiddenfiles` | **DEAD** (corrected post-review) | Rust struct field exists, never read — not shipped |
| `tab:preset` | **DEAD** | 0 frontend consumers anywhere (Rust type + generated TS type only) — **excluded from UI**, see §6 |
| `tab:skipcloseconfirm` | NEW | confirmed live (`tabbar.tsx:135,1088`) — shipped in Window & Panes |
| `widget:showhelp` | **DEAD** (corrected post-review) | only appears in generated `gotypes.d.ts`, zero reads — not shipped |
| `widget:icononly` | NEW | confirmed live (`action-widgets.tsx`, `base-menus.ts`) — shipped in Advanced |
| `telemetry:enabled` | **DEAD** (corrected post-review) | not shipped; `telemetry:interval`/`telemetry:numpoints` below are real, this toggle isn't |
| `telemetry:interval` | NEW | real consumer — drives the sysinfo widget's polling rate (not literal telemetry transmission; see `specs/settings-cleanup.md` §C4). Shipped, relabeled "Sysinfo widget" in the UI to avoid implying data collection. |
| `telemetry:numpoints` | NEW | same as above, has schema `min/max/default` (30–1024, default 120) — shipped |
| `conn:askbeforewshinstall` | **wrong file** (corrected post-review) | Declared in `schema/settings.json` but `SettingsType` has no matching Rust struct field — the live version is per-connection in `schema/connections.json`. Not shipped in global Settings. |
| `conn:wshenabled` | same as above | not shipped |
| `network:lan_discovery` | ✅ live, but duplicate (corrected post-review) | Already has a working toggle in `frontend/app/statusbar/HostPopover.tsx`. Not duplicated into Settings — see correction note. |
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

**As shipped: 5 sections** (originally planned as 6, including a `Network` section — cut post-review, see correction note), dropping `Agent`, `Files`, and `Network` as standalone rail items, adding `Window & Panes`:

1. **Appearance** (unchanged, already live) — `window:theme`, `window:transparent`, `window:opacity`, `window:blur`, `window:tilegapsize`, `window:reducedmotion`. **Add:** `window:bgcolor`, the 4 `window:magnifiedblock*` keys — all visual, all fit naturally here.
2. **Window & Panes** *(new section)* — as shipped, just 4 keys: `blockheader:showblockids`, `app:defaultnewblock`, `app:showoverlayblocknums`, `tab:skipcloseconfirm`. (The original plan for this section had 11 keys; 7 turned out dead — see correction note. `window:dimensions` also excluded — see §6.)
3. **Terminal** (unchanged, already live) — existing 8 keys. **Add:** `term:predictiveecho`, `term:predictiveecho:thresholdms` (a normal terminal-feel setting, not power-user-only).
4. **Sounds & Notifications** — all 11 `notify:*` keys, including the 2 the old spec missed.
5. **Advanced** — the real power-user/restart-required catch-all, as shipped: `term:disablewebgl`, `widget:icononly`, `telemetry:interval`, `telemetry:numpoints` (relabeled "Sysinfo widget" in the UI), `cmd:env` (key-value editor). (`app:globalhotkey`, `widget:showhelp`, `telemetry:enabled`, `preview:showhiddenfiles`, `term:localshellpath`, `term:localshellopts` all dropped — dead, see correction note.)

~~**Network**~~ — cut. `network:lan_discovery` stays at its existing home (`HostPopover.tsx`) rather than being duplicated; `conn:askbeforewshinstall`/`conn:wshenabled` turned out to have no global-settings backing at all (wrong-file, see correction note).

`app:dismissarchitecturewarning`, `window:dimensions`, `tab:preset` are deliberately **not surfaced anywhere** — see §6 for why each.

**Rail, as shipped:** Appearance · Window & Panes · Terminal · Sounds · Advanced (5 items, was 7 in the original spec — `RAIL` array in `settings-view.tsx` and `SettingsSection` union in `settings-model.ts` both updated accordingly).

## 4. Component work needed

Everything needed for sections 1–5 above (Appearance additions, Window & Panes, Terminal additions, Sounds, Network) is a straight application of the **existing** `SettingRow` / `ToggleControl` / `SliderControl` / `set()` primitives — no new components. This is most of the remaining work and is low-risk, mechanical.

**New primitives needed only for Advanced (Phase 4-equivalent):**
- **Key-value editor** for `cmd:env` (add/remove rows, each a `key` + `value` text pair). No existing pattern to reuse verbatim; build as a new control alongside `SettingRow`/`ToggleControl` in `settings-view.tsx` (or split into `settings-controls.tsx` once the file grows — `settings-view.tsx` is already 407 lines before this work).
- **String-array editor** for `term:localshellopts` (shell launch args) — same shape as the key-value editor, simpler (single column, add/remove rows).
- **Masked/reveal password field** — infrastructure only, nothing in the current schema needs it (see §2). A reusable pattern already exists elsewhere in the app (`OAuthConnectPanel.tsx`, `AgentNewIdentityModal.tsx`, `BrowserAuthModal.tsx`, `identity-view.tsx`) — if this is built, crib the pattern from one of those rather than inventing a new one. Low priority; only build if/when a secret-type key actually lands in the schema.

## 5. Implementation sequence

Numbering continues from the original spec's Phase 1–2 (already shipped):

**Phase 3 — Window & Panes, Sounds sections, as shipped** (all straight `SettingRow` application, no new components)
- Update `SettingsSection` union (`settings-model.ts`) and `RAIL` array (`settings-view.tsx`): drop `agent`/`files`/`network`, add `window`.
- Wire `window:bgcolor` + 4 `magnifiedblock*` keys into `AppearanceSection`.
- New `WindowPanesSection` component: 4 keys, not the originally planned 11 — see correction note.
- New `SoundsSection` component: 11 `notify:*` keys. `notify:sounds:volume`/`notify:tooltones:volume` use `SliderControl` (0–1, matching `term:transparency`'s existing usage); `notify:sounds:enabled` gates the rest of the section visible/disabled the same way `AppearanceSection` gates opacity/blur behind `window:transparent` today (established pattern, reuse it). `notify:tooltones:scope` is the first `enum`-as-`<select>` outside `window:theme`/`term:theme` — same `<select>` pattern.
- Add `term:predictiveecho` + `term:predictiveecho:thresholdms` to the existing `TerminalSection` (indent the threshold row under the toggle, same pattern already used for opacity/blur under `window:transparent`).
- No `NetworkSection` — cut post-review, see correction note.

**Phase 4 — Advanced section, as shipped**
- Key-value editor control for `cmd:env` — keyed by `Object.keys()`, not `Object.entries()`, so SolidJS's `<For>` can match rows by stable string identity across edits instead of remounting every row on every keystroke.
- No string-array editor — `term:localshellopts` (its only planned use) turned out dead; `StringArrayEditor` was deleted as unused code rather than shipped with an unfixed duplicate-key bug review also caught in it.
- Remaining flat `SettingRow` keys, as shipped: `term:disablewebgl`, `widget:icononly`, `telemetry:interval`/`numpoints` (relabeled "Sysinfo widget"). Dropped: `app:globalhotkey`, `widget:showhelp`, `telemetry:enabled`, `preview:showhiddenfiles`, `term:localshellpath`, `term:localshellopts` — all dead, see correction note.
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
| `frontend/app/view/settings/settings-model.ts` | `SettingsSection` union: drop `"agent"`/`"files"`/`"network"`, add `"window"` (Window & Panes) |
| `frontend/app/view/settings/settings-view.tsx` | `RAIL` array update; new `WindowPanesSection`, `SoundsSection`, `AdvancedSection` components (replacing their `StubSection` placeholders — no `NetworkSection`, cut post-review); `AppearanceSection`/`TerminalSection` extended with the new keys from §3; new key-value/string-array editor controls |
| `frontend/app/view/settings/settings-controls.tsx` | **New, optional** — split out of `settings-view.tsx` once it grows past ~600 lines with the new controls, to keep the file navigable (judgment call at implementation time, not a hard requirement) |
| `frontend/app/view/settings/settings.scss` | New styles for key-value editor rows, string-array editor rows |
| `CLAUDE.md` | "Settings" row in the "Not widgets" table still says "Opens `settings.json` in the user's default editor" — stale since Phase 1 shipped; the raw-JSON open is now a footer link inside the pane, not the primary action. One-line fix, unrelated to the rest of this spec's scope but should be corrected whenever this work lands. |

## 8. Out of scope (unchanged from the original spec)

Everything the original spec scoped out is still out of scope for the same reasons: tab-level settings (context menus, not global — and per §2, `tab:preset` turns out not to even be wired, reinforcing this), widget pinning UI (right-click on the widget bar), agent-level overrides (Identity/Bundles system), import/export (raw-JSON escape hatch covers it).
