# AgentMux Status Report — 2026-04-03

## Current State

| Field | Value |
|-------|-------|
| **Local version** | 0.33.26 |
| **Remote version** | 0.33.25 (1 local commit not pushed) |
| **Branch** | `main` at `08ff00b` |
| **Working tree** | Clean (untracked docs/specs only) |
| **Running build** | v0.33.25 portable on Desktop |

---

## Unpushed Commit

```
08ff00b chore: bump version to 0.33.26
```

Remote `main` is at `156bfa6` (PR #277 merge). A `git push origin main` will sync.

---

## Recently Merged PRs (last 48h)

| PR | Description | Merged |
|----|-------------|--------|
| #278 | Echo delay fix — bypass RAF for small PTY writes | Apr 2 16:44 |
| #277 | Clipboard support for CEF host (Win32/macOS/Linux) | Apr 2 15:00 |
| #275 | Opacity applies to all windows + removal fix | Apr 2 08:16 |
| #274 | Single-instance new window on re-launch | Apr 2 06:18 |
| #273 | Secondary windows use CEF Views (resize + no flash) | Apr 2 05:54 |
| #272 | CEF Views deferred show — no flash, resize works | Apr 2 04:34 |
| #271 | Remove WS_THICKFRAME to eliminate white border flash | Apr 2 03:07 |
| #270 | Remove disable-gpu-compositing flag | Apr 2 02:53 |
| #268 | Eliminate white flash on startup | Apr 1 21:29 |
| #266 | Per-version cache dir + DWM only on secondary windows | Apr 1 03:21 |

**10 PRs merged in ~36 hours** — major CEF host stabilization push.

---

## Open PRs (9)

### Needs Splitting
| PR | Owner | Description | Action |
|----|-------|-------------|--------|
| #267 | AgentX | Win11 focus ring (bundled 4 changes) | Split into 3 focused PRs |

### Needs Review
| PR | Owner | Description | Age |
|----|-------|-------------|-----|
| #256 | Agent1 | Drone pane Phase 1 (node-graph canvas) | 4 days |
| #232 | AgentX | Linux opacity + backend isolation | 9 days |
| #228 | AgentX | Linux zoom ghost-pixel fix | 9 days |
| #209 | AgentY | Docker attach delivery | 12 days |
| #196 | AgentX | pwsh in Taskfile | 13 days |

### Likely Stale
| PR | Owner | Description | Why |
|----|-------|-------------|-----|
| #249 | AgentA | CLI exe copy fix | 6 days — check if still relevant |
| #234 | AgentA | RAF guard + scroll flash | 8 days — #278 may supersede |
| #224 | AgentA | Tab drag cursor fix | 9 days — stale |

---

## Priority Actions

1. **Push local bump** — `git push origin main` (syncs 0.33.26)
2. **Split PR #267** — 3 PRs: CSS focus ring, DOM renderer default, dragend safety net (analysis in `docs/analysis/pr-267-276-takeover.md`)
3. **Opacity bug** — `set_window_transparency` in `commands/window.rs` still uses single-window lookup; #275 only fixed removal path
4. **Stale PR triage** — close or rebase #234, #249, #224
5. **Review queue** — #256, #232, #228, #209

---

## Untracked Files (not committed)

Docs/specs/analysis files accumulated across sessions:
- 4 handoff docs, 1 prior report
- 10 analysis/spec docs in `docs/`
- Leftover temp dirs: `46316runtimeDir/`, `4648runtimeDir/`, `cef-testbed/`

Consider: commit the docs, `.gitignore` or delete the temp dirs.
