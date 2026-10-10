# Hidden tips in the Help pane: the gestures nobody would guess

**Status:** active: P1 in #NNNN; owner decisions in §9.
**Date:** 2026-10-10.
**Requested by:** repo owner: "lets add more hidden tips to the help. there are a lot of unobvious key shortcut helpers, like using ctrl when border resize, or anything else u find, write that to a separate spec".
**Author:** AgentA@Area54, with Masty@starpower (macOS) and Maricon@charlie (Linux) to verify per platform.
**Related:**
- [PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md](PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md) (the shortcut table and its verification);
- [PLAN_SHORTCUT_KINKS_2026_10_10.md](PLAN_SHORTCUT_KINKS_2026_10_10.md) (K7, Ctrl+F in a terminal, moves here);
- [SPEC_HELP_PANE_FILTER_2026_10_08.md](SPEC_HELP_PANE_FILTER_2026_10_08.md) (the Help pane's filter, which tips join).

---

## 1. What's there today

The Help pane's keyboard list is generated from the shortcut table (`frontend/app/keybindings/defaults.ts` via `help.ts`), so it can't drift from the keys. Everything that isn't a table key is hand-written in `frontend/app/element/quicktips.tsx`:
- **One "Mouse" line:** "Resize a single border: Shift + drag" (`MOUSE_ENTRIES`). It is correct.
- **Three "More Tips":**
  - "Right click any tab to change backgrounds or rename". Right-click opens a colour picker with Rename (`tab/tab.tsx`), so "backgrounds" is out of date.
  - Two "click the gear" tips for the web view and the terminal.

A sweep of the frontend and the host found about 45 gestures a user wouldn't guess: modifier + mouse, double- and middle-click, drag and drop, and keys a pane handles itself. Help mentions two of them.

On the owner's example: the border modifier is **Shift**, not Ctrl. Shift+drag on a pane border moves only the border between the two panes beside it (`layout/lib/TileLayout.*.tsx`). On Windows, Shift while dragging the app window's edge gives all the size change to the panes along that edge (`crates/cef/src/client/wndproc.rs`, `layout/lib/windowEdgeResize.ts`).

## 2. Goals

1. **Help shows the hidden gestures**, grouped by where they apply, with this platform's modifier names (⌘ on macOS where the code accepts Cmd).
2. **Tips can't drift from the code**, any more than the shortcut list can: each tip is a row in a table, and a test fails when the code it describes changes.
3. **The Help filter finds them** ("resize", "middle", "rename").
4. **No tip says something false on one platform.** Where a gesture differs by platform, or only works with Ctrl even on macOS, either fix the code first (§5) or say so in the tip.

## 3. The tips table

A new `frontend/app/keybindings/tips.ts`, next to the shortcut table:

```ts
interface TipRow {
    id: string;                 // "pane:border:shiftDrag"
    area: TipArea;              // "Panes" | "Window" | "Tabs" | "Terminal" | "Editor" | "Files" | "Media" | "Agent" | "Find" | "Browser"
    gesture: string;            // "shift+drag", "mod+wheel", "dblclick", "middleclick", "drop", "key:Space", ...
    label: string;              // what it does, in the Help pane's voice
    where?: string;             // "on a pane border", "over an image"
    platforms?: KeyPlatform[];  // when not all ("win32" for the window-edge Shift)
    source: { file: string; anchor: string }; // the code it describes (§4)
}
```

- Gestures use the shortcut table's key syntax for modifiers (`mod` = ⌘ on macOS, Ctrl elsewhere; `ctrl` = Ctrl everywhere), plus a small set of pointer words: drag, wheel, dblclick, middleclick, rightclick, drop, hold. `formatKey` renders them per platform ("⌘ + wheel", "Ctrl + wheel").
- The Help pane renders the table as a **"Mouse and gestures"** card grouped by area. It replaces `MOUSE_ENTRIES` and the stale "More Tips", and `QuickTips`' filter matches on area, label, `where` and the rendered gesture.
- The docs site's shortcut page (`keybindings/doc.ts`) gets the same table as a second section.
- ListShortcuts keeps returning table keys only. Tips aren't runnable, but a separate `kind: "tip"` list is a later option if agents want it.

## 4. Keeping tips true

Each row's `source` names the file and an **anchor**: a short string that appears in the code the tip describes (`onResizeMove(… !event.shiftKey)`, `e.button === 1`, `onDblClick={() => props.nodeModel.toggleMagnify()}`).
- A vitest (`tips.test.ts`) checks that every anchor still appears in its file. Renaming or removing the handler fails the test, so whoever changes the gesture updates the tip in the same PR.
- That's drift detection, not behaviour testing, which is deliberate: it's cheap and catches the common case.
- Behaviour per platform is checked by hand with the partners, once per release that touches a gesture (§7), and recorded in §10.

## 5. Fix first: modifiers that differ on macOS

Several pane handlers read `ctrlKey` only, while the global zoom handler accepts Ctrl or Cmd. On macOS, Cmd+wheel over those panes falls through to the global handler. For the terminal and editor that's probably the same zoom, but Swarm keeps its own zoom setting, so the result may differ (unverified).

Unify these on `mod` (Cmd on macOS) before documenting them, or the tip is wrong on one platform:
- **Ctrl+wheel zoom:** terminal (`term.tsx`), editor (`editor-view.tsx`), Warden, section pane, Swarm, and the agent's shell drawer (`AgentShellSubblock.tsx`).
- **Ctrl + / - / 0 zoom in Swarm** (`swarm-view.tsx`).
- **Ctrl+F search in the agent pane** (`useAgentKeyboard.ts`). On macOS the table's Find is ⌘F, so the agent pane should match.

Each fix is a one-line change with a test. Masty confirms on macOS.

## 6. The tips (first set)

Ordered by how much they help. The list comes from a sweep of the code; §7 checks each tip by hand before it ships. "In Help" is today's state. Platform notes: "all" unless stated; `mod` = Ctrl, or ⌘ on macOS.

| Area | Gesture | What it does | Source | In Help |
|---|---|---|---|---|
| Panes | Shift + drag a border | Moves only the border between the two panes beside it (plain drag resizes the whole group) | `layout/lib/TileLayout.*.tsx` | yes |
| Panes | Double-click a pane header | Maximizes or restores the pane | `block/blockframe.tsx` (`onDblClick` → `toggleMagnify`) | no |
| Panes | Drag a pane header | Moves the pane in the layout (not while maximized) | `layout/lib/TileLayout.core.tsx` | no |
| Panes | Hold Ctrl+Shift | After a moment, numbers every pane, for Ctrl+Shift+1–9 | `store/keymodel-dispatch.ts` (`registerControlShiftTracking`) | partly: only the 1–9 keys |
| Panes | mod + wheel | Zooms the pane under the pointer; over the title bar, status bar or a pane header, zooms the app instead | `app/app.tsx` (`isOverChrome`) | no |
| Panes | mod + Shift + wheel | Zooms every pane in the window together | `app/app.tsx` | no |
| Panes | Drag a pane onto a window tab | Hold to open that tab and place the pane, or drop on the tab to add it there | `tab/droppable-tab.tsx` | no |
| Panes | Drag a pane or pane tab out of the window | Makes it a floating window; drag it back over the main window to dock it | `drag/pane-tab-tearoff.ts` | no |
| Window | Shift while dragging the window's edge | All the size change goes to the panes along that edge (Windows) | `crates/cef/src/client/wndproc.rs` | no |
| Window | Double-click empty title-bar space | Maximizes the window | `hook/useWindowDrag.*` | no |
| Tabs | Double-click a tab's name | Renames it (Enter saves, Esc cancels) | `tab/tab.tsx` | partly: F2 only |
| Tabs | Right-click a tab | Colour and Rename | `tab/tab.tsx` | yes, but says "backgrounds" |
| Tabs | Wheel over the tab strip | Scrolls it sideways | `tab/tab-reorder.ts`, `element/PaneTabStrip.tsx` | no |
| Tabs | Drag a tab below the strip | Opens it in a new window; Esc during the drag cancels | `tab/tab-reorder.ts` | no |
| Tabs | Middle-click a pane, document or editor tab | Closes it | `element/PaneTabStrip.tsx` (`e.button === 1`) | no |
| Tabs | Double-click a preview tab | Keeps it open (pins it) | `doc-tabs/DocTabStrip.tsx`, `editor/editor-tab-strip.tsx` | tooltip only |
| Terminal | Click a URL | Opens it in the system browser | `term/termwrap.ts` | no |
| Terminal | Click a file path | Opens it (with `:line`, in VS Code at that line), or shows it in the file manager | `term/filelinkprovider.ts` | no |
| Terminal | Drop files on a terminal | Copies them into its working folder | `term/term.tsx` | no |
| Terminal | Ctrl+F (Windows/Linux) | Goes to the shell; find in a terminal is Ctrl+Shift+F | table rows `pane:find` | no (K7) |
| Editor | File tree: click / double-click | Click opens a preview tab, double-click a normal tab; F2 renames | `editor/file-tree.tsx` | no |
| Files | Ctrl+click / Shift+click (⌘ on macOS) | Toggle one row / select a range; Ctrl/⌘+Space toggles the focused row | `files/files-view.tsx` | no |
| Files | Space, typing, `/`, Backspace | Preview panel; jump to a name; filter; go back | `files/files-view.tsx` | no |
| Files | Middle-click a folder | Opens it in a new tab | `files/files-view.tsx` | no |
| Files | Double-click the folder path | Type a path | `files/files-view.tsx` | tooltip only |
| Files | Drag rows onto a pane / drop files onto the list | To an agent, editor, media or terminal pane; drop copies here, or into the folder row under the pointer | `files/files-view.tsx`, `drag/file-drop.ts` | no |
| Media | Wheel / drag / double-click on an image | Zoom around the pointer / pan / fit or zoom; + − 0 1 and arrows too | `media/media-view.tsx` | no |
| Agent | Esc in the message box | Clears it (mod+Z brings it back); in an empty box, interrupts the agent | `agent/components/AgentFooter.tsx` | no |
| Agent | Up in an empty box | Takes back the last queued message; Up/Down step through history | `AgentFooter.tsx` | no |
| Agent | Tab or → in an empty box | Accepts the suggested next prompt | `AgentFooter.tsx` | no |
| Agent | `/` and `!` | `/` opens command autocomplete; a message starting with `!` runs as a shell command | `AgentFooter.tsx`, `agent/bang-command.ts` | no |
| Agent | Paste or drop files | Attaches them to the next message | `AgentFooter.tsx`, `hooks/useAgentDropAttach.ts` | no |
| Agent | Permission prompt keys | Enter allows; Shift+Enter denies; O / S / P / G set the scope; Esc minimizes | `agent/components/AgentDecisionPanel.tsx` | no |
| Agent | Middle-click a link | Opens it in a browser pane (a plain click opens the system browser) | `element/link-open.ts` | no |
| Find | Enter / Shift+Enter in a find bar | Next / previous match | `element/search.tsx` | tooltip only |
| Browser | In a browser pane: mod+L, mod+R, Alt+←/→ | Address bar, reload, back/forward | `crates/cef/src/client/handlers.rs` | no |

Left out of the first set, to add once checked in the app: the xterm defaults (Alt+click moves the cursor, Shift+click extends a selection) and the CodeMirror defaults (mod+click adds a cursor, Alt+drag makes a block selection). The repo doesn't configure them, so they aren't verified (§7). Also left out are niche views (the drone canvas, the memory editor's divider, bundle reordering): they can join their own pane's tips later.

## 7. Verification

- **Windows (AgentA):** each first-set tip is tried by hand in a dev build, and the result recorded in §10.
- **macOS (Masty) and Linux (Maricon):** the same, after §5's fixes. That especially covers the mod-key tips, the double-click title bar on each window manager, and the drag-out-to-window gestures.
- **Library defaults** (xterm, CodeMirror): tried on all three platforms before they join the table.
- **CI:** the anchors test (§4), plus a Help-pane test that every tip renders and is found by its filter words.

## 8. Phases

1. **P1.**
   - `tips.ts` with the first set, the anchors test, and the "Mouse and gestures" card in Help (replacing `MOUSE_ENTRIES` and the stale tips).
   - The Ctrl+F-in-a-terminal note.
2. **P2.** §5's Ctrl-to-mod fixes, then the zoom and search tips that depend on them.
3. **P3.** The docs site page from the same table; the library-default tips that passed §7.
4. **P4 (optional).** A per-pane "Tips for this pane" strip in Help's filter results, or a `kind: "tip"` list for agents.

## 9. Decisions for the owner

**Decided 2026-10-10** (the owner: "choose best recommendations"): D1, D2 and D3 are all yes as proposed.

The questions as asked:

- **D1.** Help shows the curated first set (§6, about 35 tips), not all 45 found. Proposed: yes, and niche views get their own pane's tips later.
- **D2.** Unify pane zoom and agent search on ⌘ for macOS (§5) before documenting them, rather than documenting "Ctrl, even on macOS". Proposed: yes.
- **D3.** Library defaults (xterm, CodeMirror) appear only after a per-platform check. Proposed: yes.

## 10. Results

**P1 (#NNNN).**
- `keybindings/tips.ts` holds 34 tips: the first set, minus the zoom tips and the agent pane's Ctrl+F, which wait for §5's ⌘ fixes (P2).
- They render as a "Mouse and gestures" card in Help. It replaces the hand-written "Shift + drag" line and the out-of-date tab tip; the two gear tips stay under "More Tips".
- Every tip's anchor is checked by `tips.test.ts`.
- The hand check per platform (§7) is still owed, for Windows as well; it's recorded here as it's done.
