# Spec: README.md Rewrite

## Why

The current README has several issues:
1. **Broken logo** — references `./assets/agentmux-logo.svg` which doesn't exist (actual logo is at `frontend/logos/agentmux-logo.svg`)
2. **Stale pane types table** — lists `chat`, `tsunami`, `vdom`, `launcher` which are legacy Wave Terminal views, not AgentMux features
3. **Windows-centric build outputs** — only shows `.exe` and NSIS outputs, ignores macOS/Linux
4. **Architecture diagram is wrong** — shows `AgentMux.exe` (Windows only), doesn't reflect the Tauri v2 multi-platform reality
5. **npm aliases section is misleading** — `npm run package` doesn't exist, `npm run build:backend` doesn't exist
6. **Missing recent features** — no mention of: Forge widget, drag-and-drop (files, panes, tabs, cross-window), tab color picker, widget reorder, per-pane zoom, Linux AppImage support
7. **VERSION_HISTORY.md stale footer** — bottom sections reference Go, old bump scripts, outdated version numbers

## What to Change

### README.md

**Keep:**
- Header with logo (fix path), title, tagline, badges
- "The Problem" section (good framing)
- "What AgentMux Does" section (mostly accurate)
- Apache 2.0 license note

**Fix:**
- Logo path: `./assets/agentmux-logo.svg` → `./frontend/logos/agentmux-logo.svg`
- Architecture diagram: remove `.exe`, show generic binary names
- Pane types table: remove `chat`, `tsunami`, `vdom`, `launcher` — add `forge` (agent orchestration)
- Build outputs: show per-platform (macOS .dmg, Windows NSIS, Linux AppImage)
- Prerequisites: add Go (still needed for tsunami demo, though not for core)
- npm aliases: remove or fix — these are wrong

**Add:**
- Screenshots section (placeholder — no screenshots exist yet)
- Downloads section pointing to releases
- Linux AppImage note (with backspace/Wayland fix context)

**Remove:**
- npm aliases section (misleading, not how the project works)
- Windows-only build output paths

### VERSION_HISTORY.md

**Fix footer sections:**
- "Latest Fork" says `v0.31.4` → should be `0.31.119`
- "Version Bump Instructions" references `./bump-version.sh` → should reference `bump` CLI
- "Notes for Agents" says "Run `task build:backend` after Go changes" → no Go in core anymore
- Stale appended rows at bottom (lines 521-523) — move into proper version history or remove

## Proposed README Structure

```
Logo + Title + Tagline + Badges
The Problem
What AgentMux Does (features list)
Quick Start
  - Prerequisites
  - Development
  - Production Build
Architecture (diagram + stack)
Pane Types (accurate table)
Build Commands (task commands, per-platform outputs)
Version Management (bump-cli)
License
```

## Files to Edit

1. `README.md` — full rewrite per above
2. `VERSION_HISTORY.md` — fix footer sections only (don't touch version entries)
