# Spec: the pane locate flash, one framework for every path that surfaces an existing pane

Task #29426 (spec), build #29427. Camper@narko, 2026-11-08. Written against `origin/main` f2569c2ec.
Builds on `SPEC_REVEAL_BLOCK_ONE_PATH_2026_09_27.md` (§4.1 this window, §4.3 another window) and the locate flash of #6396-era work in `pane-locate.ts`.

Direction approved by the operator (relayed by AgentY, 2026-11-08): the flash fires for anything that surfaces an already-existing pane, "consolidate into a framework, since there are many consumers"; build follows this spec without a second approval round.

## 1. What exists

One request, one player:

- **Ask:** `emitPaneLocate(blockId)` (`app/util/pane-locate.ts:73`).
- **Play:** `usePaneLocateFlash` (`app/element/pane-chrome-hooks.ts:101`), mounted once per pane by `PaneChrome`. Every pane hears every request; the pane whose stack holds `blockId` waits until it is on screen and still (`whenPaneReady` + `samplePane`), then `showLocateFlash` draws the wash and `emitPaneLocatePlay` lights the matching tab pill.
- **Rules built in:** newest request wins (a new request ends the running or waiting flash in every pane); flashes start at least `LOCATE_MIN_GAP_MS` (340 ms) apart, the newest one waiting its turn; opacity-only, one 900 ms fade, wash only under the motion policy's reduced setting.
- **One caller that matters:** `revealBlock()` (`reveal-block.ts:262`). The selection machine's `flash` effect calls `liveDeps.flash` -> `emitPaneLocate` once the reveal is verified. 13 consumers reach it (Swarm, attention, OS notify, palette, singleton panes via `openOrFocusPaneByView`, Tower x2, Kanban, tokens, Audit, jobs, API go-to, media stop, remote window, agent-opened toast, picker, duplicate-agent link).

So the framework is right. What is missing is that a second family of callers surfaces a pane *without* `revealBlock()` and never asks.

## 2. Findings that change the plan

1. **The player refuses an unfocused pane.** `usePaneLocateFlash` samples the pane with `isFocused() && activeBlockId() === blockId` (`pane-chrome-hooks.ts:122`) and ends the flash when the pane loses focus (`:152-159`). `showBlockWithoutFocus` exists to show a pane *without* moving focus (an agent opening a file, a config file, a pending-open drain). Adding `emitPaneLocate(blockId)` to it, as the task's map proposes, would wait 1.5 s for a focus that never comes and drop the flash. This is the main design point of the spec (§3.2).
2. **`showBlockWithoutFocus` is the real bypass, not `editor-open.ts:77`.** Four callers surface a pane through it and none flashes: `editor-open.ts:77` (file pushed into the Editor on screen), `editor-model.ts:339` (the pane draining `editor:pending_open_files`, which an srv-side `reuse_editor_pane` also writes), `agent-open-in-editor.tsx:82` (the "Open in editor" button), `open-config-file.ts:94` (settings/config files). Fixing it once covers all four. `editor-open.ts` and `editor-model.ts` both call it for one open (the first writes the queue and shows, the pane then drains the queue and shows again), so **a double request per open is the normal case** and must not restart the flash (§3.3).
3. **The doc-tab blockId needs no threading.** The task assumed `DocTabsModel.activate()/attachDoc()` must learn their pane's blockId. They do not: `DocTabStrip` already takes `blockId` ("The block whose tabs these are", `DocTabStrip.tsx:38`) and owns the `onActivate` (`:100`); `dropDocTab(sourceBlockId, tabId, targetBlockId, at)` (`doc-tab-hosts.ts:131`) already names the target pane. `DocTabsModel` stays pure and unchanged.
4. **The doc-tab drag path (`attachDoc`'s "already open" branch, `doc-tabs.ts:162`) is unreachable from a drop.** `docTabsHost.refuseTake` (`doc-tab-hosts.ts:170`) refuses a tab whose key is open in the target *before* `take` runs, so the drop shows the "It is already open there." notice and nothing is surfaced. The surfacing belongs on that refusal (§3.4), not in `attachDoc`.
5. **More in-stack switches exist than the four in the map.** Same operation (a different member of a visible pane comes to the front, nothing asks for a flash): `pane-tab-picker.ts:66` (new Editor tab in a pane that has one), `open-remotes.ts:33` (existing Connectors tab), `open-history-tab.ts:115` (existing history tab), `pane-history.ts:81` (back/forward navigation), `agent-pane-chrome-model.ts:83` (agent pane tab click). Under the steer of §3.4 they stay quiet (they run from inside the pane the person is looking at); pane-history alone flashes.

## 3. The design

### 3.1 One entry point per kind of surfacing

| Kind | Entry point | Focuses? | Flash |
|---|---|---|---|
| Reveal a pane that may be hidden (other window tab, minimized, magnified, background member) and focus it | `revealBlock()` (unchanged) | yes | via the selection machine, as today |
| Surface a pane in this window tab without moving focus | `showBlockWithoutFocus()` (now flashes) | no | `emitPaneLocate(blockId, { focus: false })` |
| Bring a different member/doc tab to the front of a pane that is already showing | the code that did it | already | `emitPaneLocate(blockId)` |

`revealBlock()` stays the one way to *reveal*. The other two rows are not reveals (nothing is hidden, nothing switches window tab), so routing them through the selection machine would add an RPC-shaped state machine, a window-tab check and a verify step to a click that is already on screen. They share the single player and the single request, through one helper, so all paths emit identically.

### 3.2 The helper and the focus-free flash

In `pane-locate.ts`:

```ts
export interface LocateOptions {
    /** The reveal moved focus to the pane (default): the flash waits for it and ends when it leaves.
     *  False for a pane shown without taking focus: it flashes where it is, focused or not. */
    focus?: boolean;
}
/** `blockId` was just surfaced: its pane flashes once it is on screen. */
export function emitPaneLocate(blockId: string, opts: LocateOptions = {}): void;
```

`emitPaneLocate` gains the optional second argument and is the one call for code that surfaced a pane without `revealBlock` (no second name is added, so there is no second spelling to drift). The request listener is told `(blockId, opts)`; `pane-tab-pill`'s `onPaneLocate(() => ...)` ignores the extra argument and needs no change.

`usePaneLocateFlash` changes in three places:

- the readiness sample's `holds` is `(opts.focus === false || isFocused()) && activeBlockId() === blockId`;
- `begin` applies the same test before drawing;
- the "leaving the pane ends its flash" effect ends it only when the running flash was a focus flash. A focus-free flash is not a claim that the pane is the selected one, so a focus change does not cancel it (a newer request still does, in every pane, as today).

The flash itself, the pill strike, the 340 ms gap, the motion policy and the reduced variant are untouched.

### 3.3 No double flash

Two sources of repeat, both handled in the player so no caller has to know:

- **Same block, still waiting:** a request for the block whose gate is already waiting (not yet drawn) is ignored. It does not restart the 1.5 s timeout or reset the stable-frame count. The `editor-open.ts` + `editor-model.ts` pair, and a reveal that both switches the member and then reaches `revealBlock`'s flash, land here.
- **Same block, just drawn:** a request for the block whose flash started less than `LOCATE_COALESCE_MS` (**600 ms**) ago is ignored (the wash is still at its strongest; a second one reads as a flicker). Past that, a new request replaces it, as today.

A request for a different block still ends the running flash and starts the new one (newest wins), so two panes never flash together. The 340 ms start gap keeps applying. Order of precedence when one user action both switches a member and reveals: the `revealBlock` flash (focus) is the same block, so it is the coalesced one; nothing else is needed.

### 3.4 The paths (revised 2026-11-08 after AgentY's steer: the flash locates a pane the person cannot already see or has not focused; a plain tab switch in a pane already in front of them stays quiet)

Flash:

- **Show without focus** (`reveal-block.ts:119` `showBlockWithoutFocus`): `emitPaneLocate(blockId, { focus: false })` after `showBlockInPane`, on the success path only. Covers the four callers in finding 2 (editor already open, the pane's own pending-files drain, "Open in editor", config files). For a brand-new pane (srv made a new Editor) the flash plays once the pane mounts: "here it is"; the gate waits for the mount.
- **Doc-tab drag onto a pane that has the document** (`dropDocTab`, `doc-tab-hosts.ts`): when the refusal is `ALREADY_OPEN_THERE`, bring that existing tab to the front of the target (`DocTabHost.reveal(key)`, built by `docTabsHost` from `tabs.activate`) and `emitPaneLocate(targetBlockId, { focus: false })`. The source tab stays where it was and the notice stays: the notice says why the tab did not move, the flash says where the document is. `attachDoc`'s unreachable "already open" branch is left as the guard it is.
- **Back / forward through pane history** (`pane-history.ts` `selectPane`): `emitPaneLocate(ref.blockId)` after the pane is focused. It can land on a pane the person was not looking at.

Quiet by default (the borderline ones, for the operator to judge on the real build, §4):

- A pane-tab pill click or context-menu activate (`pane-chrome-hooks.ts` `handleActivate`), the agent pane's own tab click (`agent-pane-chrome-model.ts`), a document-tab click (`DocTabStrip.tsx`), and the three "bring the tab I just asked for to the front of this pane" calls that run from inside that pane (`pane-tab-picker.ts` new Editor tab, `open-remotes.ts`, `open-history-tab.ts`). The person's pointer is on that pane and it is focused and in front. Each is one line to turn on (`emitPaneLocate(blockId)` after the switch); none is wired in this build.

### 3.5 (dropped) The five other in-stack switches

Finding 5 listed them so they would not be missed. Under the steer they belong to the quiet group above, except pane-history, which flashes.

### 3.6 What is deliberately not flashed

- Layout-internal member changes: restore, sync from another window, `activateMemberInStack` in persistence and merges (`layoutPersistence.ts:268`, `layoutStack.ts:512`), a close that moves the front to the next tab.
- Keyboard cycling of document tabs (`Ctrl+Tab`, `doctab:*`): the person's eyes and caret are already in that pane.
- A pane that is not on screen when the flash would start (a background window tab): the gate times out as now.

## 4. Behaviour to confirm in the look check

The quiet group in §3.4 is the borderline: a 900 ms wash over a whole pane on every tab click, three a second at most, would read as noise on panes already in front of the person. It ships off. The macOS and Linux look checks are asked to judge the flashing paths (they must read as "this is the pane that just changed", once, never twice) and say whether any quiet path should flash after all; enabling one is a single line and touches nothing else in this spec.

## 5. Tests (build)

- `pane-locate.test.ts`: `emitPaneLocate(id, { focus: false })` flashes an unfocused pane; a focus request on an unfocused pane still waits and times out; a focus-free flash survives the pane's focus changing; same-block request while waiting is a no-op; same-block request within `LOCATE_COALESCE_MS` of a start is a no-op, after it replaces; a different block replaces the running flash.
- `reveal-block.test.ts`: `showBlockWithoutFocus` emits `{ focus: false }` after the pane is shown, and nothing when it could not show it.
- `PaneChrome.test.tsx` (the player): a focus-free request flashes an unfocused pane and survives focus changes; a focus request ends when the pane loses focus; a repeat for the block still waiting, or flashed within `LOCATE_COALESCE_MS`, is the same flash; after the window it starts again; a pane that never became visible lets the next request start afresh.
- `doc-tab-hosts.test.tsx`: a drop refused with `ALREADY_OPEN_THERE` activates the existing tab in the target host and emits for the target block (focus false); other refusals and moves emit nothing.

## 6. Visual check

The change adds the flash where there was none, so it is a visual change: macOS (Clare, starpower) and Linux (Maricon, charlie) look checks, plus Windows by me. Per flashing path: the wash appears once, over the right pane, then fades; a focus-free flash on a pane that is not the focused one looks the same (no ring, no focus change); two quick triggers (the editor-open pair) show one flash; light and dark themes; at the motion policy's reduced setting only the wash plays. Recorded in the PR body with the commit checked.

## 7. Not in scope

- A new look for the flash (colour, length) or the pill strike.
- Moving the selection machine's verify/retry into the other two rows of §3.1.
- Flashing other windows' panes from here (`revealInOtherWindow` already hands that to the owner window, whose `revealBlock` flashes).
