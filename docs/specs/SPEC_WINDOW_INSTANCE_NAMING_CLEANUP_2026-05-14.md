# SPEC: Window/Instance Naming Cleanup

**Date:** 2026-05-14
**Author:** AgentX
**Status:** Draft — needs scope decision
**Related:** PR #841 (window-title format) surfaced this; recent specs `SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md`, `SPEC_WINDOW_RENAME_2026_04_27.md`, `SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md`

---

## 1. Problem

The terms **"window"** and **"instance"** are used inconsistently across types, atoms, RPCs, UI copy, and specs. Most notably "instance" is overloaded between two genuinely different concepts:

- **Window-rank-within-an-instance** — `(1)`, `(2)`, `(3)` shown after the version in the StatusBar (`StatusBar.tsx:92`), populated by `getInstanceNumber()` RPC into `windowInstanceNumAtom`.
- **A separate AgentMux instance** (a whole launcher → srv → host process tree, see §3) — multiple instances run side-by-side (portable + dev + different versions), per `CLAUDE.md "Multiple Instances Run in Parallel"`. This is the LAN-discoverable thing in `LanStatus.tsx:43` ("N other AgentMux instances on LAN").

These are not the same thing. Today the codebase (and copy) blurs them.

A second axis of drift: which token identifies a window in code?

- `windowId` — backend UUID (OID on `WaveWindow`)
- `label` — launcher-assigned symbolic name (`"main"`, `"window-pool-..."`)
- `instanceNumber` — 1-based rank within this AgentMux instance

Three identifiers, three semantics, used together in `WindowEntry = { label, windowId }` (frontend/app/store/global.ts:148) plus a separate `windowInstanceNumAtom`.

---

## 2. Audit (one-line summary per surface)

### 2.1 Type / data-model layer

| Term | Meaning | Where | Verdict |
|------|---------|-------|---------|
| `WaveWindow` | Persisted backend data object | `frontend/types/gotypes.d.ts:1708`, generated from Go | Legacy from Wave Terminal — **keep** (touching this is a multi-repo schema rename) |
| `Workspace` | Workspace assigned to a window | gotypes | Keep |
| `Tab` | Tab inside a workspace | gotypes | Keep |

### 2.2 Frontend atoms (the front line)

| Atom | Type / purpose | File | Issue |
|------|---------------|------|-------|
| `openWindowEntriesAtom` | `WindowEntry[]` — list of open windows in this instance | `global.ts:149` | OK |
| `openWindowLabelsAtom` | `string[]` — labels only, derived | `global.ts:140` | OK |
| `windowCountAtom` | total count | `global.ts:133` | OK |
| `windowInstanceNumAtom` | this window's rank | `global.ts:132` | **Misleading name** — it IS rank-within-instance, but "instance number" reads like "which instance am I" instead of "which window in my instance" |

### 2.3 RPC / IPC

| Method | Signature | File | Issue |
|--------|-----------|------|-------|
| `openNewWindow()` | — | `custom.d.ts:115` | OK |
| `focusWindow(label)` | by label | `custom.d.ts:138` | OK |
| `closeWindow()` / `closeWindowByLabel(label)` | — | `custom.d.ts:116,255` | OK |
| `getWindowLabel()` | this window's label | `custom.d.ts:?` | OK |
| `listWindowInstances()` | returns `[{ label, windowId }]` for THIS instance | `custom.d.ts:130` | **Misnomer — returns windows, not instances** |
| `getInstanceNumber()` | returns this window's rank | `custom.d.ts:139` | **"InstanceNumber" but it's window-rank** |

### 2.4 UI copy (user-visible)

| Surface | Text | File | Issue |
|---------|------|------|-------|
| StatusBar tooltip | "Click for instance panel" | `StatusBar.tsx:85` | **Says "instance" — confusing** |
| StatusBar aria-label | "AgentMux version — open instance panel" | `StatusBar.tsx:86` | **Same** |
| InstancePanel header | "This process — N window(s)" | `InstancePanel.tsx:271` | **Wrong — "this process" is the renderer; the panel actually lists all windows in the instance** (see §3.1) |
| InstancePanel aria-label | "AgentMux instance panel" | `InstancePanel.tsx:231` | **Says "instance"** |
| InstancePanel row tooltip | "This window — double-click to rename (F2)" | `InstancePanel.tsx:322` | OK |
| LanStatus | "N other AgentMux instances on LAN" | `LanStatus.tsx:43` | **Correct usage** — these really are separate instances (separate process trees) |

### 2.5 Component / file names

| Name | File | Notes |
|------|------|-------|
| `InstancePanel` (component) | `frontend/app/statusbar/InstancePanel.tsx` | The thing it shows is a list of WINDOWS, not instances |
| `.instance-panel-*` (CSS) | InstancePanel.tsx + scss | Implementation detail; safe to keep |

### 2.6 Specs

- `SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md` — predates this cleanup; uses "instance" throughout
- `SPEC_WINDOW_RENAME_2026_04_27.md` — drifts toward "window" naming
- `SPEC_WINDOW_TITLE_FORMAT_2026-05-13.md` — uses "window" + "tab"
- `specs/instance-indicator.md` — older

---

## 3. Process hierarchy

An "AgentMux instance" is **not one process** — it's a process **tree**, rooted at one launcher, sharing one data dir and one Job Object (Windows). A baseline single-window dev session has **4 processes**, plus shared Chromium subprocesses, plus more as panes and windows are opened:

```
agentmux-launcher.exe                    [1]  Job Object owner; single-instance pipe; saga coord; spawns srv + host
├── agentmux-srv.exe                     [2]  Rust backend sidecar (DB, RPC, agent runner)
│   └── agentmux-srv --crash-monitor          srv crash watchdog (counted as part of srv)
└── agentmux-cef.exe                     [3]  CEF "browser" process — host; owns OS windows; IPC bridge
    ├── agentmux-cef --type=renderer     [4]  Chromium renderer for this window's HTML/JS context
    ├── agentmux-cef --type=gpu-process       shared (1 per host)
    ├── agentmux-cef --type=utility …         shared per service (network, storage, …)
    └── (more renderers spawn per browser pane / per additional window)
```

| # | Process | Role | Lifetime | Count per instance |
|---|---------|------|----------|--------------------|
| 1 | `agentmux-launcher.exe` | Job-Object root, single-instance enforcement, saga coord, splash, srv lifecycle | Whole instance | 1 |
| 2 | `agentmux-srv*.exe` | Rust backend sidecar (DB, RPC, agent runner) | Whole instance | 1 (+1 crash-monitor child) |
| 3 | `agentmux-cef.exe` (host / "browser process") | CEF browser process; owns OS windows; IPC bridge to renderers | Whole instance | 1 |
| 4 | `agentmux-cef --type=renderer` | Chromium renderer; runs frontend JS for one browser context | One per browser context (window or browser pane) | 1+ (≥1 per OS window, +1 per browser pane) |
| – | `agentmux-cef --type=gpu-process` | Shared GPU compositor | Whole host | 1 |
| – | `agentmux-cef --type=utility …` | Per-service Chromium utilities (network, storage, audio, …) | As CEF demands | 2–N |

### 3.1 What this means for terminology

- **"Process"** is ambiguous from the renderer's POV — there are many processes in one AgentMux instance. The InstancePanel header `"This process — N windows"` (`InstancePanel.tsx:271`) is **misleading**: the panel lists all windows in the **instance** via IPC to the host, not "this process" (which is just the renderer this JS is running in).
- **"Instance"** is the right word for the user-meaningful unit — it's the whole tree. Distinct AgentMux versions, dev + portable, etc. each get their own instance (separate launcher, separate srv, separate host, separate data dir, separate Job Object).

## 4. Canonical terminology

> **`instance`** = an AgentMux process **tree** rooted at one launcher. Owns one Job Object, one data dir, one srv, one host, and all the Chromium subprocesses the host spawns. Multiple instances run in parallel (portable + dev + per-version) per `CLAUDE.md "Multiple Instances Run in Parallel"`.
>
> **`window`** = an OS window owned by an instance's host. One instance can have many windows. Each window has a `windowId` UUID, a launcher `label`, and a rank within the instance.
>
> **`pane`** / **`browser pane`** = a Chromium browser embedded inside a window for browser-widget tabs. Each browser pane spawns its own renderer (and possibly additional utility processes) — implementation detail, not part of the conceptual model.
>
> **`label`** = launcher-assigned symbolic name for a window inside an instance (`"main"`, `"window-pool-..."`). Internal/IPC-only; not user-facing.
>
> **`windowId`** = backend OID for the persisted `WaveWindow`. Internal; not user-facing.
>
> **`window rank`** (or just "N" in `Window N`) = a window's 1-based position in `openWindowEntriesAtom` within its instance. Cosmetic only — used for the title fallback and the StatusBar `(N)` chip.
>
> **`process`** = an OS process. **Avoid in user-facing copy** — too ambiguous (4+ per instance). Reserve for internal docs / process-management code.

### 4.1 Rules

1. **No "instance" in same-instance window contexts.** "Instance" refers to the whole process tree — not to a window within it.
2. **Drop "process" from user-facing copy where the conceptual unit is the instance.** The InstancePanel says "This process — N windows" — change to "This instance — N windows" (or "Open windows", which sidesteps the question).
3. **Drop "instance number" from user-facing copy.** It's a window rank — call it that or just say `Window N`.
4. **Internal tokens (`label`, `windowId`) stay** — they identify windows in IPC and backend storage and aren't part of the conceptual model the user thinks about.
5. **`WaveWindow` stays.** Renaming it would touch generated Go types, the schema, and unrelated repos. The conceptual mismatch (`Wave*`) is a leftover from the Wave Terminal fork — orthogonal to this cleanup.
6. **`pane` is the term for embedded browser surfaces** — don't call them "windows" in code or copy.

---

## 5. Tiered cleanup

Tiered so we can pick a scope. T1 is purely UI copy, T4 touches RPC + Go. Each tier is independently mergeable.

### T1 — User-facing copy only (low risk, ~5 lines)

| File | Change |
|------|--------|
| `StatusBar.tsx:85` | `data-tip="Click for instance panel"` → `data-tip="Click to open window list"` |
| `StatusBar.tsx:86` | `aria-label="AgentMux version — open instance panel"` → `aria-label="AgentMux version — open window list"` |
| `InstancePanel.tsx:231` | `aria-label="AgentMux instance panel"` → `aria-label="Open windows"` |
| `InstancePanel.tsx:271` | `"This process — N window(s)"` → `"This instance — N window(s)"` (or `"Open windows — N"`) — see §3.1: the panel lists windows in the **instance**, not in "this process" (which is just the renderer) |

No code path changes. No spec breakage. **Safe to ship in any PR.**

### T2 — Frontend atom rename (medium risk, ~20 sites)

| Old | New | Rationale |
|-----|-----|-----------|
| `windowInstanceNumAtom` / `setWindowInstanceNumAtom` | `windowRankAtom` / `setWindowRankAtom` | "Rank" matches what it actually is (1-based position within the instance) |

Touches: `global.ts:132`, `app-init.ts:97-98` (callsite around `getInstanceNumber()`), `StatusBar.tsx:4,17`. ~5 files, mechanical rename.

Mention in CHANGELOG / VERSION_HISTORY because consumers in non-frontend repos that import the atom would break (probably none).

### T3 — RPC / IPC rename (higher risk, requires host update + frontend update in lockstep)

| Old | New |
|-----|-----|
| `getInstanceNumber()` (RPC) | `getWindowRank()` |
| `listWindowInstances()` (RPC) | `listOpenWindows()` |

Touches:
- `frontend/types/custom.d.ts:130,139`
- `frontend/util/cef-api.ts:428`
- `agentmux-cef/src/...` (Rust handlers — find via `grep -rn "getInstanceNumber\|listWindowInstances" agentmux-cef/`)
- `agentmux-launcher/src/...` if the launcher exposes these names

Needs a same-PR rename of both Rust and frontend; can't ship them split. Search for any external consumers (e.g. e2e tests, deploy scripts).

### T4 — Component / file rename (cosmetic but invasive)

| Old | New |
|-----|-----|
| `InstancePanel` (component) | `WindowListPanel` |
| `frontend/app/statusbar/InstancePanel.tsx` | `frontend/app/statusbar/WindowListPanel.tsx` |
| `.instance-panel-*` CSS classes | `.window-list-panel-*` (or leave as implementation detail) |
| `SPEC_VERSION_INSTANCE_PANEL_2026_04_25.md` | Add note "renamed to Window List in 2026-05" — don't rename the historic spec file |

Touches the component file, its scss, all importers (`StatusBar.tsx` + tests). CSS rename is optional — if we leave the class names alone (they're implementation detail), this drops to ~3 files.

---

## 6. Recommendation

Ship **T1 + T2 in one PR** alongside or right after PR #841 lands. Both are low-risk, mostly-mechanical, and resolve the most user-visible confusion.

**Defer T3 + T4** to a second PR after T1+T2 settles. T3 needs a Rust-side change and a careful migration; T4 is a big touch for a cosmetic win and can wait.

---

## 7. Out of scope

- **`WaveWindow` / `WaveObj` rename** — these are generated from Go schema, used across the persisted DB layer; renaming is a multi-week project, unrelated to this cleanup.
- **`label` → something else** — `label` is the established launcher-IPC term, used in production. Renaming would churn `agentmux-launcher/`, `agentmux-cef/`, splash, single-instance pipe, etc. Not worth it.
- **`workspace.name` semantics** — orthogonal; workspace naming is its own concept.

---

## 8. Open questions

1. Confirm **T1 + T2 scope** is what we want for the next PR (vs deferring everything to design more).
2. T2 atom rename — `windowRankAtom` is my proposal; alternatives are `windowOrdinalAtom`, `windowPositionAtom`, or just keep `windowInstanceNumAtom` as a legacy name and only fix the user-facing copy. Pick one.
3. T3 — accept that RPC rename requires a frontend+Rust same-PR change, or skip T3 entirely?
4. Should we add a section to `CLAUDE.md` codifying the canonical terminology so it doesn't drift again?
