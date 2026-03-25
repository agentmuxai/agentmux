# AgentMux Dead Code & Legacy Audit

**Date:** 2026-03-09
**Audited by:** Agent2

---

## Executive Summary

The codebase has been mostly cleaned up from the Wave Terminal → AgentMux migration, but several categories of dead code and legacy naming remain. The AI panel (`aipanel/`) is **DEAD CODE** — the sidebar AI chat is not used. The agent pane (`view/agent/`) handles all AI agent interactions. See `specs/archive/remove-aipanel-sidebar.md` for full removal plan.

**Actionable items:** 2,575-line aipanel/ removal (see spec), ~400-line onboarding/ removal, 3 dead SVG assets, 7 unused npm deps, legacy branding across 327 files.

---

## 1. Dead Files (Safe to Delete)

### Unused SVG Assets

| File | Status |
|------|--------|
| `frontend/app/asset/workspace.svg` | Zero imports |
| `frontend/app/asset/magnify-disabled.svg` | Zero imports |
| `frontend/app/asset/thunder.svg` | Zero imports |

### Orphaned Modal Components

Comment in `modalregistry.tsx` says "Onboarding modals removed for lightweight build" but the files remain:

| File | Export | Imported? |
|------|--------|-----------|
| `frontend/app/onboarding/onboarding.tsx` | `NewInstallOnboardingModal` | No |
| `frontend/app/onboarding/onboarding-upgrade.tsx` | `UpgradeOnboardingModal` | No |

---

## 2. Unused npm Dependencies (Safe to Remove)

| Package | Why Unused |
|---------|-----------|
| `prop-types` | No PropTypes validation anywhere; project uses TypeScript |
| `shell-quote` | Zero imports in frontend |
| `class-variance-authority` | Zero imports; project uses tailwind + clsx |
| `color` | Zero imports; tinycolor2 is used instead |
| `env-paths` | Zero imports in frontend |
| `fast-average-color` | Zero imports; feature never shipped |
| `immer` | Zero imports; Jotai handles immutability |

---

## 3. Legacy "Wave" Branding (Still Pervasive)

### High Impact — Core Architecture

These are deeply embedded and require careful refactoring:

| Area | Count | Examples |
|------|-------|---------|
| Wave Object System (WOS) | 169+ types | `WaveObj`, `WaveWindow`, `WaveKeyboardEvent` |
| Core init functions | 5 functions | `initWave()`, `reinitWave()`, `initWaveWrap()` in `frontend/wave.ts` |
| AI model class | 1 class | `WaveAIModel` in `aipanel/agentai-model.tsx` |
| Store module | 31 consumers | `wos.ts` — `makeORef()`, `getWaveObjectAtom()`, etc. |
| Generated types | Full file | `frontend/types/gotypes.d.ts` (generated from backend) |

### Medium Impact — Environment & Config

| Item | File | Current | Should Be |
|------|------|---------|-----------|
| Dev env var | `frontend/util/isdev.ts` | `WAVETERM_DEV` | `AGENTMUX_DEV` |
| Dev Vite var | `frontend/util/isdev.ts` | `WAVETERM_DEV_VITE` | `AGENTMUX_DEV_VITE` |
| Auth key var | `agentmuxsrv-rs/src/backend/wavebase.rs` | `WAVETERM_AUTH_KEY` | `AGENTMUX_AUTH_KEY` |
| Legacy dir | `agentmuxsrv-rs/src/backend/wavebase.rs` | `~/.waveterm` migration | Keep (backward compat) |

### Low Impact — Cosmetic

| Category | Count | Examples |
|----------|-------|---------|
| Debug namespaces | 8 | `debug("wave:app")`, `debug("wave:ws")` |
| CSS class names | 7 files | `.wave-button`, `.wave-iconbutton`, `.waveblock` |
| VDom tag names | 4 | `wave:text`, `wave:null`, `wave:style` |
| Component names | 15+ | `WaveBlock`, `WaveModal`, `WaveStreamdown` |
| Backend comments | 4 | "wave-init event", "Wave application" |

### Copyright Headers

**327 files** still say `Copyright 2025, Command Line Inc.` instead of `Copyright 2026, AgentMux Corp.`

Primarily in:
- `frontend/**/*.ts[x]`
- `frontend/**/*.scss`
- `agentmuxsrv-rs/src/**/*.rs`

---

## 4. Dead Code — AI Panel & Onboarding

### AI Panel (`frontend/app/aipanel/`) — REMOVE

14 files, 2,575 lines. **Not used** — the sidebar AI chat assistant is a legacy Wave Terminal feature. AgentMux uses the agent pane (`view/agent/`) for all AI interactions.

Removal touches 12+ files across workspace layout, focus manager, keyboard shortcuts, and RPC layer. Full removal spec: `specs/archive/remove-aipanel-sidebar.md`

### Onboarding (`frontend/app/onboarding/`) — REMOVE

~400 lines. Already orphaned — not registered in modal registry. References AI panel. Delete entire directory.

### All View Types (Registered & Used)

All 7 views in `frontend/app/view/` are registered in block.tsx:
- `term`, `sysinfo`/`cpuplot`, `vdom`, `help`, `launcher`, `agent`, `chat`

### All Tauri Stub Commands (Called)

All 13 commands in `src-tauri/src/commands/stubs.rs` are referenced from the frontend. None are orphaned.

---

## 5. Recommended Cleanup Priority

### Phase 1 — Quick Wins (< 1 hour)

1. Delete 3 unused SVG assets
2. Delete 2 orphaned onboarding modals
3. Remove 7 unused npm dependencies
4. Update debug namespaces: `wave:*` → `agentmux:*`

### Phase 2 — Branding Pass (2-4 hours)

1. Update 327 copyright headers to `AgentMux Corp`
2. Rename env vars `WAVETERM_*` → `AGENTMUX_*` (with backward compat fallback)
3. Rename CSS classes `.wave-*` → `.agentmux-*` across 7 SCSS files
4. Update backend comments referencing "wave"

### Phase 3 — Deep Refactor (1-2 days)

1. Rename `WaveAIModel` → `AgentMuxAIModel`
2. Rename `initWave()` → `initApp()` in `frontend/wave.ts`
3. Rename WOS types — requires updating `gotypes.d.ts` generator and all 31 consumers
4. Rename VDom tags `wave:*` → `agentmux:*`

**Note:** Phase 3 touches generated types (`gotypes.d.ts`) which come from the Rust backend. The rename must happen in the backend type generator first, then regenerate the frontend types.


