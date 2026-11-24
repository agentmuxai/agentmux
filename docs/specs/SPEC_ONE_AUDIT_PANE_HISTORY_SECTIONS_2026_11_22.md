# Spec: One Audit pane: History's three views become its sections, and the History pane name goes

**Task:** #48399 (spec); build #48400 follows approval.
**Author:** Agent1@narko, 2026-11-22.
**Status:** active. Approved 2026-11-24; build in progress (#48400). The direction is the operator's, given in conversation on 2026-11-22: the History and Audit panes are not different enough to be two panes, and the name "History" is confusing next to an individual agent's own history; the two become one pane, named Audit.
**Verified against:** `agentmux` `origin/main` @ `31c769e45a` (2026-11-22), by reading the code named below. Nothing was run. Where a claim rests on a file I did not open, it says so.
**Builds on:** [SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md](SPEC_WARDEN_SPLIT_INTO_VAULT_HISTORY_AUDIT_2026_10_28.md) (the split this partly undoes), [SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md](SPEC_AUDIT_LIVE_LOG_OF_EVERY_AGENT_ACTION_2026_11_03.md), [SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md](SPEC_WARDEN_HISTORY_SECTION_2026_10_18.md).

## 0. Summary

One pane, **Audit**, with four sections: **Events**, **Conversations**, **Typed**, **Storage**. Events is today's Audit pane unchanged (its list, filters, live bar and the secrets record). Conversations, Typed and Storage are today's History pane's Agents, Typed and Storage views, moved as they are. The History pane, its widget, its palette entry and its status-bar and menu callers go; everything that names History (saved panes, pins, links, shortcuts, widget calls) opens Audit on the matching section and nothing a person saved is lost. "History" stays only as the name of one agent's own history inside the agent pane.

It is a move, not a redesign: no section changes what it shows, and the reader that opens a conversation is not touched.

## 1. What exists (read, not run)

- **Audit** (`frontend/app/view/audit`): a native pane tab, view `audit`, icon `list-check`, colour amber, `paneZoom`, no sections (`audit.tsx:11-21`). One list from `audit.query`, newest first, 50 a page, with a scope control, an Agent box, a Search box, a range select, eight kind chips, a live bar (pause, "N new", sound) and, below the list, the secrets record (`VaultSecrets`, imported statically in `audit-manager.tsx:13`). Its filters are local signals, not saved. Its meta keys are `audit:agent`, `audit:sound`, `audit:message` (`audit-shared.ts:18-22`); `audit:section` is unused.
- **History** (`frontend/app/view/history`): a native pane tab, view `history`, icon `clock-rotate-left`, colour sky, `paneZoom`, no sections (`history.tsx:11-21`). A three-way control (Agents, Typed, Storage) held in a local signal, not saved, so it always opens on Agents (`history-manager.tsx:28`). All three views are mounted at once and hidden, each gated by `visible`.
- **The reader** is not part of the History pane. "Open history" and an Audit row's "in history" link both open an agent block with `agent:historyTabFor` as a tab in the stack of the pane that asked (`open-history-tab.ts`, `history-agents.tsx:80-86`, `audit-row.tsx:26-40`). So today, from Audit, the reader already opens beside Audit as a tab of the same pane, and the Audit tab is the way back.
- **Both are singletons**: `widget-singleton.ts:22,26` and the mirror in srv `pane_singleton.rs:14-21` (a test keeps the two lists equal).
- **Where History is named**: the widget (`crates/srv-core/src/config/widgets.json:184-196`), `open:history` in the command registry (`command-registry.ts:219-224`), the palette map (`palette-open-commands.ts:21-26`), the default key `h` after Ctrl+Shift+O (`keybindings/defaults.ts:118`), the status bar (`LowDiskNotice.tsx:61`, `HistoryUsagePopover.tsx:114`), the deep-link registry (`crates/common/deeplinks/destinations.json:264`, generated `registry.generated.ts:489-503`, `registry.rs:239`), the Pane tool's `view` list (`crates/mcp/src/tool_schemas.rs:327`), the widget SDK doc (`sdk/widget-sdk/v1.d.ts:38-43`, `README.md:43`), `README.md:167`, `docs/deep-links.md:78-95`, `docs/keybindings.md:22`.
- **Redirects today** cover a (view, section) pair only: `MOVED_SECTIONS` (`pane-sections.ts:73-80`, with the same table in srv `wconfig/types.rs:799-811`), the one-time migration `m0044_warden_split.rs`, and `layout_file.rs:693-708`. A section is refused through an alias (`pane-sections.ts:131-133`). History never had a section key, so a saved History pane is just `view: "history"`: the existing table cannot express it, and **this spec needs a whole-view rule** (§5).
- **Blackout in screen capture**: `capture-mask.ts:45` blacks out `vault`, `audit` and `memory`; History is not blacked out.

## 2. Requirements

1. One pane named Audit with the sections below, in this order. Nothing else is called History except an agent's own history.
2. Events, Conversations, Typed and Storage each show exactly what Audit's list and History's Agents, Typed and Storage views show today, with their own filters and state.
3. Anything that named the History pane opens Audit on Conversations (or on the section it named); a saved pane, pin, layout, widget entry, link, shortcut or `panes.open` call keeps working, and nothing saved is lost or doubled.
4. A section's code loads when it is first shown, as today, and its live reads run only while it is the section on screen.
5. Panels, not modals; the pane's chrome zoom applies to all sections; nothing new is hand-drawn (the kit's tabs/segmented control).

## 3. Choices (each with a recommendation; the first six are the operator's)

**C1. Names and order.** Recommended: **Events, Conversations, Typed, Storage**, in that order. Events first because it is what Audit was and is the live one; Conversations next because it is the other thing people open the pane for. Reasons for the names: "Conversations" says what Agents listed (each agent's kept conversation) without "History"; "Events" matches the list's own rows. Alternatives I considered: "Activity" for Events (friendlier, but the pane is called Audit and "Events" is closer to the rows), "What you typed" for Typed (clearer, longer; the section's own heading can say it, the tab stays "Typed").

**C2. Default section and memory.** Recommended: a new Audit pane opens on **Events**; the chosen section is **kept per pane** in `audit:section` block meta (the same convention as the Vault's `vault:section`, `pane-sections.ts:36-38`), so a saved layout reopens where it was. An opener that names a section wins over the saved one (a link to Conversations, the status bar's disk notice to Storage, an old History pane). This fixes today's "History always opens on Agents".

**C3. How an Events row opens its conversation.** Recommended: **unchanged**: the reader opens as a tab beside Audit in the same pane's stack (that is what happens today), and closing it or clicking the Audit tab is the way back. Opening it inside the Conversations section instead would mean putting an agent pane inside a section, which the reader (an agent block) cannot be today, and gains nothing a tab does not. Conversations' "Open history" does the same, so both paths agree.

**C4. Search.** Recommended: **one box per section**, as now: Events keeps Agent and Search; Conversations keeps "Filter agents"; Typed keeps its agent picker and query; Storage has none. The three searches are over different data (srv's audit log, the agent list, one agent's typed lines), so a single box would have to mean three things. A cross-section agent filter (pick an agent once, every section follows) is a good follow-up and is not part of this move.

**C5. Filters across sections.** Recommended: **no new filters**. Events keeps its time range; Conversations and Typed get none (neither has one today, and a time filter on a list of agents or on typed lines is a feature, not a move).

**C6. Where Storage lives.** Recommended: **here**, as the fourth section, and Settings gets one row that opens it ("Where history is kept and for how long"). The status bar's disk notice and usage popover open Audit on Storage.

**C7. Screen capture blackout.** Today Audit is blacked out and History is not. A merged pane is one view, so one rule. Recommended: **black out the whole pane**: Conversations and Typed are the person's own words and the agents' conversations, which is at least as private as the audit log. The cost: an agent can no longer take a screenshot of Storage (usage numbers only). This is the operator's call: say if Storage should stay visible, in which case the rule has to be per section.

**C8. Deep-link sensitivity.** The registry gives each destination one sensitivity: `audit` is private and own-agent only, `history` is public (`destinations.json:237-264`). Opening an old `history` link now opens Audit. Recommended: **the old `history` link becomes an alias of `audit` with `section=conversations` and takes Audit's sensitivity** (so a link someone shared opens only for the person, which is what Conversations should have meant); `audit` gains a `section` place with the four ids. The registry's owner (#48140) should confirm.

## 3b. Names inside the product

| Place | Today | After |
|---|---|---|
| Pane label, widget, palette | History / Audit (two) | Audit (one); palette "Open Audit"; the History widget leaves the bar (its slot, order 16, is free) |
| Shortcut Ctrl+Shift+O then H | `open:history` | keeps working: opens Audit on Conversations; it is not shown as a separate palette entry (a customised binding to `open:history` keeps working too) |
| Status bar | disk notice and usage popover open History | open Audit on Storage; the last-sound status is unchanged |
| Deep links | `history`, `audit` | `audit` with a `section` place; `history` stays as an alias to Audit on Conversations (C8) |
| Pane tool and `am.panes.open` | `view: "history"` | still accepted, opens Audit on Conversations; the Pane tool's list and the widget SDK doc drop `history` and gain Audit's four sections |
| Pane title | History / Audit | Audit (the section is shown by the pane's own tabs, not repeated in the title) |
| Docs | History and Audit rows | one Audit row naming the four sections; current release only, no note about the old name (alpha) |
| Agent's own history | "Agent History", "<name>'s History", "Open history" | unchanged. Conversations' button says "Open conversation" so the word does not collide |

## 4. Design

**The pane.** `audit-instance.tsx` becomes the single instance: one root with the pane's zoom wrapper, a section switcher (the kit's segmented control, the same as History's today) above the section, and the four sections. Section ids are `events`, `conversations`, `typed`, `storage`, declared as `sections: { default: "events", items: [...] }` on `auditPaneTab`, like the Vault (`vault.tsx:19-22`). The `paneZoom` capability is declared once. The pane's `audit:agent`, `audit:sound` and `audit:message` meta act only on Events and, when present, select Events (a link to a message or sound is a link to an event).

**Loading.** Each section is a lazy import shown the first time it is selected, then kept mounted and hidden (keeps its scroll, filter and a half-typed query, as History's views do now). The shell is the only thing the pane loads up front. Today the secrets record is a static import of Events; it moves behind Events' own load. Live reads (Events' `audit:changed` feed and 30 s poll, Conversations' agent events and 30 s poll, Typed and Storage's read-on-show) run only while `visible() && section is current`: a hidden section does not poll. This is stricter than History today, where a hidden view still follows events.

**Code moves.** `frontend/app/view/history/*` moves under `frontend/app/view/audit/` (`conversations/`, `typed/`, `storage/`), with its tests, its styles and the three i18n groups. Storage's import of `measuredAgo` from the status bar module (`history-storage.tsx:9`) moves to a shared util. `warden-shared` (both panes import its chrome and `formatAge`) is left where it is; renaming it is separate and was planned in the split spec.

**Singleton and registry.** `history` leaves `BUILTIN_SINGLETON_VIEWS` and srv `SINGLETON_VIEWS` together (the test that keeps them equal enforces it), `historyPaneTab` leaves `block-registry.ts`, `defwidget@history` leaves `widgets.json`.

## 5. Migration (the part that must not lose anything)

A saved History pane has no section key, so the existing (view, section) table cannot map it. Add a **whole-view rule**: `{ from: { view: "history" }, to: { view: "audit", section: "conversations" } }`, read by the same readers that read `MOVED_SECTIONS` today, with the same table in srv:

1. **Saved panes and layouts**: a one-time srv migration (`m0045`, written like `m0044_warden_split.rs`) rewrites every block with `view: "history"` to `view: "audit"` with `audit:section: "conversations"`, in blocks, in the session snapshot, and in the layout file (`layout_file.rs` read and write sides). An Audit pane that already exists keeps its meta untouched. A tab that holds both a History pane and an Audit pane keeps both (they become two Audit panes; Audit stops being a singleton only for this migration, then the focus-or-open rule applies to new opens).
2. **Pins**: `widget:pinned` lists History and Audit separately; after the migration a pin list holds Audit once, at the position of whichever came first, and a list with neither is untouched. (Same shape as `PINNED_REPLACEMENT` in `m0044:119`.)
3. **widgets.json entries a person wrote** that name `view: "history"`: rewritten at load by srv's `resolve_section`, so a customised entry keeps working and shows up as Audit.
4. **Runtime**: `movedMeta` (`pane-sections.ts:105`) learns the whole-view rule, so `panes.open("history")`, a deep link, the widget bridge, `open:history` and the Pane tool land on Audit/Conversations. `sectionProblem` says "History is now Audit's Conversations section" instead of refusing silently.
5. **Settings and keys**: any saved keybinding to `open:history` keeps resolving (the command stays as a hidden alias).

**Tests (all must exist before the old pane is removed):** Rust: m0045 inline tests for a block, a snapshot, a layout file, pins (History only, Audit only, both, neither, History twice), and a customised widget entry; the singleton-list equality test. Frontend: `pane-sections.test.ts` (the whole-view rule, `movedMeta` on `view: "history"` with and without other meta), `widget-bridge.test.ts`, command registry (the alias opens Audit on Conversations), deep-link conformance (`history` and `audit` with each `section`), `capture-mask.test.ts` (C7), and a render test that every old History pane test still passes against the Conversations, Typed and Storage sections.

## 6. Build, in small PRs (#48400)

1. **Sections in Audit.** Audit gains the switcher and the three moved sections, History's code moves under `audit/`, `audit:section` is saved and read. The History pane still exists and is a thin shell around the same section components, so nothing is removed yet and every opener still works. Look checks.
2. **Remove the History pane.** The whole-view rule, m0045 and its tests, singleton lists, widget, palette and key alias, status bar and Settings link, deep-link registry (generated files and docs), the Pane tool and SDK doc, the blackout rule (C7). Look checks for the status bar and Settings link.
3. **Docs** for the current release only: README, `docs/deep-links.md`, `docs/keybindings.md`, the SDK README, and the user docs (agentmux-docs, a task for its owner).

## 7. Look checks and zoom

Visual change in PRs 1 and 2: macOS and Linux look checks (the section switcher at narrow and wide widths, 100% and 150% chrome zoom, light and dark; Conversations' table and the reader opening from an Events row; Storage reached from the status bar). The switcher must stay one row at the narrowest width a pane can have, scrolling its own tabs rather than wrapping, as History's does today. No window modal is added: Typed's "forget" confirmations are panels in the pane already and stay.

## 8. What this does not do

No change to what any section shows or to its filters; no new cross-section search or agent filter; no change to the agent's own History (the tab, the menu entry, the reader); no rename of `warden-shared`; no new Settings beyond the one link row.

## 9. Open items for the operator

C1 (names and order), C2 (default Events, section remembered per pane), C3 (reader opens as a tab beside Audit, as now), C4 and C5 (per-section search, no new filters), C6 (Storage stays, Settings links to it), **C7 (the whole pane blacked out in screen capture)** and **C8 (the old History link takes Audit's private sensitivity)**.
