# SPEC: Rename `data-tauri-drag-region` → `data-drag-region`

Status: draft
Date: 2026-04-18
Owner: AgentA
Motivation: the attribute is a Tauri-era name. AgentMux has been CEF-only
since the backend migration. The name misleads readers into thinking it's
dead code (as it did me earlier today when I reviewed PR #438). The
underlying mechanism is very much alive — `useWindowDrag.win32.ts` reads
the attribute at mousedown/dblclick to decide whether to start a window
drag or fire `maximize_window` — so renaming is a straight find-and-replace
with no functional change.

## 1. What moves

Plain find-and-replace:
- `data-tauri-drag-region` → `data-drag-region`

Only the string literal changes. All semantics (values `"true"` / `"false"` /
missing, parent-walk, which elements opt in/out) stay identical.

## 2. Scope

16 references across 8 files (all in `frontend/`):

**Readers** (the live attribute consumer):
- `frontend/app/hook/useWindowDrag.win32.ts:5,16,72` — comment, `getAttribute` call, `dragProps` emitter.
- `frontend/app/hook/useWindowDrag.darwin.ts:5,6,10` — comment + `dragProps` emitter (macOS handles drag at the WebView layer via this same attribute).
- `frontend/app/hook/useWindowDrag.linux.ts:6` — comment only.

**Producers** (elements opting in/out of drag):
- `frontend/app/tab/tabbar.tsx:160,163,183` — `add-tab-btn` (false), `tab-bar-scroll` (false), `tab-bar-fill` (true).
- `frontend/app/tab/tab.tsx:251` — tab root (false).
- `frontend/app/tab/droppable-tab.tsx:102` — drop wrapper (false).
- `frontend/app/window/action-widgets.tsx:393` — widget bar container (false). *Added in PR #438 to fix the double-click-maximize bug; the current PR uses the old name.*
- `frontend/app/window/system-status.tsx:105,115,125` — three status buttons (false).

No backend code touches this — confirmed via `grep -r tauri-drag-region agentmux-cef/src agentmux-srv/src` returns zero matches.

## 3. Verification

- Rename in a single commit so `git blame` for any adjacent line still points where it used to.
- `grep -r "tauri-drag-region" frontend/` returns zero after the change.
- `grep -r "data-drag-region" frontend/` returns exactly 16 matches.
- `npx tsc --noEmit` clean.
- `task dev`: window-header dblclick still maximizes; widget-button dblclick does NOT maximize (regression check for the #438 fix); tab bar fill still allows window-drag; tabs + widget bar still don't trigger drag on grab.

## 4. Not in scope

- The comment `// Tauri: data-tauri-drag-region is handled at the WebView/OS level`
  in `useWindowDrag.win32.ts:5` references the old Tauri behavior for context.
  Updating the comment to reflect current reality is useful cleanup but
  separate from the attribute rename — it'd touch identical lines in the
  darwin/linux variants too.
- Any other Tauri-era stale names (`data-tauri-*`, `invoke` shape changes,
  `@tauri-apps/*` imports still around). Separate sweep.

## 5. Implementation

One commit, one branch off main. PR description: link to this spec, list
"no functional change, grep-only rename". After merge, follow-up commit
to also update the code comments in the three `useWindowDrag.*` files can
land in the same or a tiny follow-up PR.
