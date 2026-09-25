# SPEC: Agent shell drawer — right-click Paste, and a "context-menu region" mechanism to strip pane entries

**Date:** 2026-09-25
**Status:** implemented (single PR — Phase 1 and Phase 2 together; see §3.5 for the decisions made during implementation)
**Related:**
`docs/specs/REPORT_CONTEXT_MENU_GAP_AUDIT_2026_08_07.md` (same root cause — a pane-body handler swallows right-click and never offers Paste; fixed there per-`<input>`, not for terminals nested in a non-terminal pane),
`docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md` (the `getBodyContextMenuItems` extension point this builds on),
`docs/specs/SPEC_AGENT_SHELL_DRAWER_INFO_PANEL_2026_09_19.md` (drawer layout),
`docs/specs/SPEC_AGENT_SHELL_DRAWER_ZOOM_COORDINATE_SPACE_2026_09_20.md` (why the drawer renders outside `.agent-view-zoomed`),
`docs/specs/SPEC_AGENT_INTERACTIVE_PTY_SHELL_API_2026_09_10.md` (agent lock on the shared shell).

---

## 1. Problem

Two asks, one root cause:

1. **There is no way to paste with the mouse in the agent pane's Shell drawer.** Right-clicking the drawer's terminal shows no Paste entry.
2. **The menu that does appear is the wrong menu for a terminal drawer.** It carries entries that make no sense from there — Split Up/Down/Left/Right and Replace With… — and there is currently no way for a region of a pane to drop entries from the shared menu.

### What right-click in the drawer shows today

The drawer (`AgentShellSubblock`, mounted at `agent-view.tsx:2935`) is a DOM descendant of the agent pane's body, so its `contextmenu` event bubbles up through two handlers:

1. `agent-view.tsx:2354` `handleContextMenu` on `.agent-view--presentation`. It only acts when `window.getSelection()` is non-empty; then it shows a Copy-only menu. xterm's selection is not a DOM selection, so this normally returns without acting — **but if the human has any text selected in the transcript, right-clicking in the drawer shows "Copy" for the transcript selection**, not the terminal's. (It calls `preventDefault` and `showContextMenu`, which stops propagation, so nothing below it runs.)
2. `blockframe.tsx:1212` `onBodyContextMenu` on the pane frame. It builds:
   `viewModel.getBodyContextMenuItems()` (for the agent: Agent History, Quick Fork) → separator → `buildPaneContextMenu()`:
   - **Copy** — `getPaneSelection(viewModel)` reads `viewModel.termRef?.current?.terminal`. The agent's ViewModel has no `termRef`, so this falls back to `window.getSelection()` and is **always disabled for text selected in the drawer's xterm**.
   - **Paste** — only when `blockData.meta.view === "term"` (`pane-actions.ts:35`). The agent block is `view: "agent"`, so **never**. Even if it were shown, its click handler reads `viewModel.termRef`, which the agent ViewModel also lacks, so it would silently do nothing.
   - Split Up / Down / Left / Right, Replace With…, Magnify, Close Pane, Inspect Element.

So in the drawer: Copy can't copy the terminal selection, Paste doesn't exist, and half the menu is irrelevant.

### Can a region strip entries today? No.

`buildPaneContextMenu` (`pane-actions.ts:149`) is all-or-nothing. The only knobs are:
- `inspectAt` — omit to drop "Inspect Element" (a call-site option, not something a region can request).
- `ViewModel.getBodyContextMenuItems()` — can only **add** items, and is per-view-model, not per-region. The drawer is one small region inside an agent pane, so it cannot use a view-model-level hook without also changing the menu everywhere else in that pane (transcript, composer, header strip).

Right-click behaviour also cannot be overridden per element without forking the whole menu: the existing per-element pattern (`showTextInputContextMenu`, `contextmenu.ts`) calls `stopPropagation`, which replaces the pane menu entirely — right for a text `<input>`, wrong for the drawer, where Magnify / Close / Inspect are still wanted.

---

## 2. Design

Add a small **context-menu region** registry. A UI region (any element) registers "when a right-click lands inside me, the pane menu should look like this". `blockframe`'s existing `onBodyContextMenu` resolves the region from the event target and adjusts the menu it already builds. Nothing about the header menu or any pane without a region changes.

### 2.1 Sections

Split `buildPaneContextMenu` into named sections, so "strip entries" means naming sections, not filtering by label:

```ts
export type PaneMenuSection =
    | "viewItems"   // viewModel.getBodyContextMenuItems() (agent: Agent History, Quick Fork)
    | "clipboard"   // Copy / Paste
    | "split"       // Split Up / Down / Left / Right
    | "replace"     // Replace With…
    | "magnify"     // Magnify / Un-Magnify Pane
    | "close"       // Close Pane
    | "inspect";    // Inspect Element
```

`buildPaneContextMenu` builds each section as its own `ContextMenuItem[]`, drops omitted and empty ones, and joins the survivors with a separator between groups. This replaces the hand-placed `{ type: "separator" }` entries, so **omitting any combination of sections can never leave a leading, trailing, or doubled separator** (today `buildReplaceSubmenu` bakes its own trailing separator into its return value, which would dangle if only Replace were removed).

Grouping (which sections share a separator group) preserves today's layout exactly:

```
[viewItems] ─ [clipboard] ─ [split] ─ [replace] ─ [magnify + close] ─ [inspect]
```

(`viewItems` is composed by `blockframe.tsx` ahead of the pane sections, as today, with the same single separator between.)

**Invariant (test-locked, §5):** with no region and no `omit`, the output is item-for-item identical to today's, separators included. This is a pure refactor for every existing caller.

### 2.2 Region registry

New module `frontend/app/block/context-menu-region.ts`:

```ts
export interface ContextMenuRegion {
    /** Sections to drop from the pane menu for a right-click inside this region. */
    omit?: PaneMenuSection[];
    /** Items rendered at the TOP of the menu, above the surviving pane sections
     *  (separated from them). Called at right-click time, not registration time,
     *  so it reads live state (selection, agent lock, …). */
    items?: () => ContextMenuItem[];
}

/** Returns an unregister function — call it from onCleanup. */
export function registerContextMenuRegion(el: HTMLElement, region: ContextMenuRegion): () => void;

/** Nearest registered region at or above `target`, stopping at `boundary`. */
export function resolveContextMenuRegion(
    target: EventTarget | null,
    boundary?: Element | null
): ContextMenuRegion | null;
```

Implementation: a module-level `WeakMap<Element, ContextMenuRegion>`; `resolve` walks `parentElement` from the target until it finds an entry or reaches `boundary`. **Nearest ancestor wins, no merging** — a region fully describes its own menu, which keeps behaviour predictable when regions nest.

Why a registry and not a `data-` attribute: a region needs to contribute items with click closures over live state (the xterm instance), which an attribute cannot carry. Why not a `ViewModel` hook: see §1 — per-view-model, not per-region.

### 2.3 Wiring in `blockframe.tsx`

`onBodyContextMenu` (`blockframe.tsx:1212`), after the existing early-return guard:

```ts
const region = resolveContextMenuRegion(e.target, frameEl());
const omit = new Set(region?.omit ?? []);
const menu: ContextMenuItem[] = [];

if (region?.items) {
    const own = region.items();
    if (own.length) menu.push(...own, { type: "separator" });
}
if (!omit.has("viewItems")) {
    const bodyItems = props.viewModel?.getBodyContextMenuItems?.(browserCtx);
    if (bodyItems?.length) menu.push(...bodyItems, { type: "separator" });
}
menu.push(...buildPaneContextMenu(blockData(), { ...existingOpts, omit }, props.viewModel));
```

The synthetic browser-pane path (`browser-pane-context-menu` event) has no DOM target and passes through this same function; `resolveContextMenuRegion` gets `undefined`/no target there and returns `null` — unchanged.

### 2.4 `agent-view.tsx`'s Copy handler must yield to regions

`handleContextMenu` at `agent-view.tsx:2354` fires before `blockframe`'s handler (it is a descendant) and would win with a stale transcript selection (§1, item 1). Add at the top:

```ts
if (resolveContextMenuRegion(e.target, e.currentTarget as Element)) return;
```

so a right-click inside any registered region always reaches `blockframe` and the region's own menu.

---

## 3. The shell drawer's region

Registered from `AgentShellSubblock.tsx` on `containerRef` (`.agent-shell-subblock` — the terminal surface only; the info panel above it and the resize handle keep the default menu), in `onMount`, unregistered in `onCleanup`:

```ts
registerContextMenuRegion(containerRef, {
    omit: ["viewItems", "clipboard", "split", "replace"],
    items: () => buildShellDrawerClipboardItems(),
});
```

### 3.1 Resulting menu

```
Copy
Paste
─────────────────
Magnify Pane / Un-Magnify Pane
Close Pane
─────────────────
Inspect Element
```

- **Split / Replace With… stripped** — the ask. Splitting from the drawer would clone the *agent* pane (with agent fields blanked, `handleSplitPane`), which is not what a click inside a terminal implies.
- **`viewItems` stripped** (Agent History, Quick Fork) — recommended, not asked: they act on the conversation, not the shell, and are one click away everywhere else in the pane. Cheap to flip: remove `"viewItems"` from `omit` (see §6).
- **`clipboard` omitted, and provided by the region instead** — the generic Copy/Paste read the *pane's* viewModel, which is the wrong terminal (§1).
- **Magnify / Close / Inspect kept** — still meaningful from anywhere in the pane.

### 3.2 Copy

Enabled iff `termWrap.terminal.getSelection()` is non-empty; click writes it with `clipboardWriteText` (same helper `pane-actions.ts` uses). Captured at right-click time, as `getPaneSelection` already does, so the selection cannot change between menu open and click. `termWrap` is the local in `AgentShellSubblock`; the closure reads it lazily, so it is correct across the re-attach path (`attachShell` repointing to a new sub-block).

### 3.3 Paste

Click: `clipboardReadText()` → empty check → `termWrap.terminal.paste(text)`. This is the same call `TermViewModel`'s Ctrl+Shift+V handler makes (`termViewModel.ts:549-556`), and it matters that it is `terminal.paste` and not a raw write: xterm applies bracketed-paste wrapping when the shell has enabled it, and the data then reaches the PTY through the drawer's existing `sendDataHandler` → `blockinput` (`AgentShellSubblock.tsx:660`).

**Agent lock.** While the agent is driving the shell (`agentLocked()`, `term:agentlockuntil`), `sendDataHandler` silently drops human input to avoid interleaving. A Paste that vanished with no feedback would look broken, so the menu item is built with `enabled: !agentLocked()`. The "Agent is using this shell" badge already explains why. `items()` runs at right-click time, so the state is current.

**Size limit.** Pastes over 1 MB are refused with an explanatory notification — see §3.5.

**Not present / not started.** If `termWrap` is undefined (still loading, or failed), Paste and Copy are both built disabled.

### 3.4 Large pastes (Phase 2 — done)

`TermViewModel.sendDataToController` splits input over 4 KB into paced `blockinput` frames with a serialized queue and a progress toast, specifically to avoid saturating Windows ConPTY. The drawer's `sendDataHandler` used to be a bare single-frame send (its own comment: "No chunked-paste handling for this spike"). Right-click Paste makes big pastes (stack traces, logs) the *expected* use, so the drawer would inherit that failure mode.

Implemented as specced: the chunked sender moved out of `TermViewModel` **verbatim** into `frontend/app/view/term/block-input-sender.ts` (`BlockInputSender`, one instance per block id), `TermViewModel.sendDataToController` delegates to it, and the drawer's `sendDataHandler` uses it too — so Ctrl+V and menu Paste in the drawer both get pacing, ordering behind an in-flight paste, and the ≥ 8 KB "Pasting…" toast. The extraction had no prior tests; `block-input-sender.test.ts` now pins chunk size, exact reassembly, a multi-byte character straddling a chunk boundary, ordering of a keystroke arriving mid-paste, and toast lifecycle.

**Gate (the "~100 KB manual paste") — outcome.** Whether an *unchunked* large paste actually fails in the drawer was never established, and no longer needs to be: the drawer now takes the same proven path as terminal panes regardless of the answer, so the extraction is justified on parity alone. See §3.5 for what was and wasn't verified in the running app.

### 3.5 Size limit and how the UI says so (added during implementation)

Chunking makes big pastes *work*; it doesn't make megabytes into a shell a good idea (the line editor re-renders the text, scrollback churns, and a slow chunked send blocks typing while it drains). So the menu Paste has an explicit limit, **1 MB** (`SHELL_PASTE_MAX_BYTES`, UTF-8 bytes of the clipboard text), and the UI states it at both moments a user needs it:

- **Before:** the item reads "Paste (up to 1 MB)". While the agent holds the shell it reads "Paste (agent is using this shell)" and is disabled, so a greyed-out item always says why. (Carried in the *label*, not `ContextMenuItem.sublabel`: the JS-rendered menu, `showJsContextMenu` in `cef-api.ts`, never draws sublabels — found by right-clicking in the running app, where the first cut's sublabel was invisible. `bind-to-agent-menu.ts`'s sublabels are invisible for the same reason; not touched here.)
- **After, when exceeded:** nothing is sent, and a warning notification says what happened, the limit, and the alternative: "The clipboard is 3.2 MB; the shell accepts up to 1 MB. Save it to a file and reference it from the shell instead."

Best-practice notes applied: state the limit up front rather than only on failure; never fail silently; refuse cleanly (nothing partially pasted) rather than truncate; name the alternative action; measure in bytes, since that is what the wire and PTY see. The limit applies to the menu Paste only. Keyboard paste goes straight through xterm's own paste handling and is unchanged apart from now being chunked.

The 1 MB figure is a judgment call, not a measured ceiling — it is a constant in one place if it needs tuning.

---

## 4. Files

| File | Change |
|---|---|
| `frontend/app/block/context-menu-region.ts` | **New.** `ContextMenuRegion`, `registerContextMenuRegion`, `resolveContextMenuRegion` |
| `frontend/app/block/pane-actions.ts` | Export `PaneMenuSection`; restructure `buildPaneContextMenu` into sections joined by generated separators; add `omit?: ReadonlySet<PaneMenuSection>` to `PaneContextMenuOpts`; move `buildReplaceSubmenu`'s trailing separator out |
| `frontend/app/block/blockframe.tsx` | `onBodyContextMenu` resolves the region, prepends `region.items()`, honours `omit` (incl. `viewItems`) |
| `frontend/app/view/agent/agent-view.tsx` | `handleContextMenu` early-returns inside a registered region |
| `frontend/app/view/agent/components/AgentShellSubblock.tsx` | Register the drawer region; use `BlockInputSender` for input |
| `frontend/app/view/agent/components/shell-drawer-menu.ts` | **New.** Copy/Paste items, size limit, `formatSize` (kept out of the component so it is unit-testable without xterm) |
| `frontend/app/view/term/block-input-sender.ts` | **New.** Chunked, ordered `blockinput` sender extracted from `TermViewModel` (§3.4) |
| `frontend/app/view/term/termViewModel.ts` | `sendDataToController` delegates to `BlockInputSender` |
| `frontend/app/block/pane-actions.test.ts` (new or extended) | §5 |
| `frontend/app/block/context-menu-region.test.ts` | **New.** §5 |
| `frontend/app/view/agent/components/AgentShellSubblock.test.tsx` | Region registered on mount, unregistered on cleanup; Copy/Paste enablement |

No backend, RPC, or schema change. Header right-click (`handleHeaderContextMenu`) calls the same builder with no `omit` and is unaffected.

---

## 5. Testing

Unit:
- **Baseline lock:** `buildPaneContextMenu` with no `omit` returns the same labels/types/order as before the refactor, for a `term` block (has Paste), an `agent` block (no Paste), with and without `inspectAt`, magnified and not. Capture the pre-refactor output as the expected value *before* changing the builder.
- **Separator hygiene:** for every subset of `PaneMenuSection` omitted (2^7 = 128 — cheap to enumerate), output never begins or ends with a separator and never contains two adjacent separators; omitting everything returns `[]`.
- **Registry:** nearest ancestor wins; `boundary` stops the walk; unregister removes it; a target outside any region resolves `null`; `null` target resolves `null`.
- **Drawer items:** Copy disabled with an empty selection and enabled with one; Paste disabled when `agentLocked()`; both disabled with no `termWrap`; Paste calls `terminal.paste` with the clipboard text and does nothing on empty/failed clipboard read.

Manual (real app — this is UI in a native-menu host, tests can't cover the visual result):
1. Open an agent pane → Shell drawer → right-click the terminal: menu is exactly the §3.1 list.
2. Copy `echo hello`, select terminal text, right-click → Copy enabled; paste into the composer to confirm.
3. Copy multi-line text, right-click → Paste in the drawer at a shell prompt; confirm bracketed-paste behaviour (no premature execution) in a shell that enables it.
4. Select text in the *transcript*, then right-click in the drawer: menu is the drawer menu, **not** the stale transcript Copy (regression for §2.4).
5. Right-click the transcript, composer, drawer info panel, and pane header: unchanged from today.
6. While an agent is driving the shell (badge visible): Paste is disabled.
7. Right-click a regular `term` pane and a browser pane: unchanged.
8. ~100 KB paste in the drawer — record the outcome; it decides §3.4.

---

### 5.1 Verified in the running app

Run as a `task dev` instance of this branch and driven over CDP with real (synthesized) mouse events — the context menu is JS-rendered DOM (`showJsContextMenu`), so it can be right-clicked, read and clicked like any other element.

- Right-click in the drawer's terminal: menu is exactly **Copy (disabled, nothing selected) · Paste (up to 1 MB) · Magnify Pane · Close Pane · Inspect Element** — no Split, no Replace With…, no Agent History / Quick Fork. (This is also how the invisible-sublabel problem in §3.5 was caught.)
- **The ~100 KB gate (§3.4), done:** a 100,081-byte PowerShell here-string (2,000 lines) placed on the OS clipboard and pasted via the menu arrived intact — the shell reported `count=2000 bytes=99999`, exactly the expected line count and byte count. Chunked send, no truncation, no reordering.
- **Over the limit:** a 1.5 MB clipboard → nothing typed into the shell; the warning notification "Paste too large for the shell — The clipboard is 1.4 MB; the shell accepts up to 1 MB. Save it to a file and reference it from the shell instead." appeared.

Not exercised in the running app: Copy of a real terminal selection (covered by unit tests only), the agent-locked disabled state (unit tests only — no agent was driving the shell), and the stale-transcript-selection regression from §2.4 (unit-level only; the handler yielding is a one-line guard).

## 6. Decisions on the open questions

Resolved by judgment during implementation rather than left open:

1. **Agent History / Quick Fork in the drawer menu** — omitted (`viewItems`), as recommended. They act on the conversation, not the shell.
2. **Magnify / Close in the drawer menu** — kept, as recommended. Removing Close later is one entry in `omit`.
3. **"Clear terminal"** — not added; not asked for.

## 7. Out of scope

- Keyboard paste/copy in the drawer. `AgentShellSubblock` constructs `TermWrap` with no `keydownHandler`, so `TermViewModel`'s Ctrl+Shift+V/C handling does not apply to it. Whether native Ctrl+V reaches xterm's paste event correctly there was not verified here; worth checking alongside Phase 1 but a separate concern.
- Adopting regions elsewhere. Other places that could use it (composer strip, drone/swarm rows) are not touched; the mechanism is deliberately generic so they can opt in later.
- Redesigning the pane menu's contents for any pane other than the shell drawer.
