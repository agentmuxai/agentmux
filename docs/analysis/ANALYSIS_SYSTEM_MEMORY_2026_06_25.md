# System Memory Analysis — 2026-06-25

**Context:** AgentMux reported low system memory. Total process working set: ~9.3 GB.
Known expected consumers: Traktor, AgentMux 0.49.1, VS Code.

---

## Memory Map (aggregated by process name)

| Process | Total MB | Instances | Category | Actionable? |
|---------|----------|-----------|----------|-------------|
| `Code.exe` | 1,239 MB | ~10 | VS Code (known) | No |
| `claude.exe` | 1,124 MB | ~4 | Claude Code CLI agents | Partial |
| `svchost.exe` | 1,082 MB | 79 | Windows system services | No |
| `agentmux-0.49.1.exe` | 1,063 MB | 2 | AgentMux current (known) | No |
| `Traktor.exe` | 876 MB | 1 | Traktor (known) | No |
| **`agentmux-0.48.1.exe`** | **668 MB** | **2** | **Stale old version — zombie** | **YES** |
| `bash.exe` | 484 MB | 67 | Agent shell sessions | Partial |
| `Memory Compression` | 353 MB | 1 | Windows kernel | No |
| `conhost.exe` | 251 MB | ~30 | Console hosts for shells | Partial |
| `explorer.exe` | 247 MB | 1 | Windows shell | No |
| `SearchApp.exe` | 194 MB | 1 | Windows Search UI | YES |
| `RuntimeBroker.exe` | 142 MB | ~4 | Windows broker | No |
| `node.exe` | 128 MB | 4 | VS Code / build tools | No |
| `parsecd.exe` | 108 MB | 2 | Parsec remote desktop | YES (if unused) |
| `agentmux-bashwrap.exe` | 103 MB | ~12 | Agent shell wrappers | Partial |
| `SearchIndexer.exe` | 83 MB | 1 | Windows Search indexer | YES |
| `MsMpEng.exe` | 79 MB | 1 | Windows Defender | No |
| `agentmux-srv-0.48.1` | **63 MB** | 1 | **Stale sidecar — zombie** | **YES** |
| `agentmux-srv-0.49.1` | 57 MB | 1 | AgentMux sidecar (known) | No |

---

## Root Causes

### R1 — agentmux-0.48.1 still running (731 MB wasted)

Two `agentmux-0.48.1.exe` processes + one `agentmux-srv-0.48.1` sidecar are alive.
This is an old version that was never closed when 0.49.1 launched. The locked
`runtime/` stubs on the Desktop (from earlier cleanup) confirm it's still holding
file handles. **This is the single biggest fixable leak: ~731 MB.**

### R2 — 67 bash.exe instances (484 MB)

67 bash shells in flight is high. Some are legitimate (active agent sessions,
agentmux-bashwrap wrappers), but many are likely orphaned from past sessions
whose parent agent panes were closed without the shell being explicitly stopped.
Each shell is ~7 MB — if half are orphaned, that's ~240 MB recoverable.

### R3 — Windows Search (277 MB)

`SearchApp.exe` (194 MB) + `SearchIndexer.exe` (83 MB) = 277 MB for background
Windows Search indexing. If you don't use Windows Search (you have `rg`/`fd` for
code search), this service can be disabled permanently.

### R4 — Parsec (108 MB)

`parsecd.exe` is Parsec remote desktop, running 2 instances in the background.
If you're not actively using remote access right now, stopping it frees ~108 MB.

---

## Fixes — Ranked by Impact

| Fix | Savings | Risk |
|-----|---------|------|
| Kill agentmux-0.48.1 + its sidecar | ~731 MB | None — stale process |
| Kill orphaned bash sessions | ~200 MB est. | Low — need to identify orphans |
| Disable Windows Search service | ~277 MB | Low — lose Win+S search |
| Stop Parsec | ~108 MB | None if not in active use |

**Total recoverable without touching known-good apps: ~1.1–1.3 GB**

---

## What NOT to touch

- `claude.exe` (1,124 MB) — active agent sessions including this one
- `svchost.exe` (1,082 MB) — Windows OS services, not safe to bulk-kill
- `agentmux-0.49.1.exe` (1,063 MB) — current running version
- `Code.exe` (1,239 MB) — VS Code as flagged by user
- `Memory Compression` — Windows kernel page compression, not a process
