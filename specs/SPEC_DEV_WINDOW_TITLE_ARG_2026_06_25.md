# Spec: `task dev TITLE="..."` — per-session window title for dev builds

**Date:** 2026-06-25  
**Status:** Implemented (this doc's own design shipped — confirmed by reading
current `main`: `Taskfile.yml`'s `dev`/`dev:serve` tasks carry `TITLE` →
`VITE_DEV_TITLE` exactly as designed, and `frontend/app-init.ts` applies it via
`UpdateObjectMeta` — status was never updated after landing, corrected here).
See "Addendum" below for a 2026-08-04 follow-up (agent auto-title).  
**Branch:** `agentx/dev-window-title`  
**Scope:** `Taskfile.yml` (dev task env), `frontend/app-init.ts`

---

## Problem

When running multiple dev sessions in parallel — your own, an agent's, a PR under review — the
OS taskbar shows every window as "AgentMux", making it impossible to tell which agent's work
you are looking at without clicking into each window.

## Goal

```bash
task dev TITLE="agentx: PR #1780"
```

The running dev window title bar becomes `agentx: PR #1780 — AgentMux` within seconds of
startup, with no manual clicks required.

---

## Design

### Why `TITLE=` task variable (not `-- --title`)

Task supports `KEY=value` variable overrides: `task dev TITLE="foo"` sets the Task variable
`{{.TITLE}}` without any shell quoting acrobatics. This is idiomatic for Task and reads
naturally. Using `-- --title foo` would require adding a custom arg parser downstream.
The variable form is also tab-completable in shells that support Task completion.

### Data flow

```
task dev TITLE="agentx: PR #1780"
    │
    ▼
Taskfile injects VITE_DEV_TITLE="agentx: PR #1780"
into the Vite dev-server process environment
    │
    ▼
import.meta.env.VITE_DEV_TITLE available to the frontend
    │
    ▼
app-init.ts: after initWaveWrap() resolves,
  ObjectService.UpdateObjectMeta(
    makeORef("window", initOpts.windowId),
    { "window:displayname": VITE_DEV_TITLE.slice(0, 64) }
  )
    │
    ▼
Reactive title effect (app-init.ts ~line 769)
re-runs: resolveWindowName() → TITLE → formatWindowTitle()
→ document.title = "agentx: PR #1780 — AgentMux"
    │
    ▼
OS window title bar updated
```

---

## Implementation

### 1. Taskfile.yml — inject `VITE_DEV_TITLE`

In the `dev` task (or whichever sub-task actually invokes Vite — `dev:serve`), add
a `TITLE` variable defaulting to empty and expose it to the Vite process as
`VITE_DEV_TITLE`:

```yaml
dev:
  vars:
    TITLE: ''          # override: task dev TITLE="agentx: PR #1780"
  env:
    VITE_DEV_TITLE: '{{.TITLE}}'
  deps:
    - ...
```

If the `dev` task delegates to `dev:serve` via `deps` and `dev:serve` is the task that
actually spawns the Vite server process, the env var must be set in the `dev:serve`
task's `env` block (or inherited from the parent process env if Task propagates it).
Verify that `VITE_DEV_TITLE` is in the process environment when `vite dev ...` is
executed — Vite picks up `VITE_*` env vars automatically in dev mode.

`dev:local` should inherit the same behavior since it wraps `dev`.

### 2. `frontend/app-init.ts` — apply the title after wave init

Locate the two `await initWaveWrap(initOpts)` call sites (approximately lines 388 and 469).
After each one, add:

```typescript
// Apply dev window title if provided via task dev TITLE="..."
const devTitle = import.meta.env.VITE_DEV_TITLE;
if (import.meta.env.DEV && devTitle) {
    void ObjectService.UpdateObjectMeta(
        WOS.makeORef("window", initOpts.windowId),
        { [DISPLAY_NAME_META_KEY]: devTitle.slice(0, DISPLAY_NAME_MAX_LEN) },
    );
}
```

`ObjectService.UpdateObjectMeta` is already in scope at this call site (used at ~line 708).
`DISPLAY_NAME_META_KEY` and `DISPLAY_NAME_MAX_LEN` are imported from
`@/util/window-title` (already imported at line 38).
`WOS.makeORef` is available from the existing WOS import.

**Guard:** `import.meta.env.DEV` is `true` only when Vite is running in dev mode. It is
`false` in production and package builds, so this code path is dead in shipped builds.

**Do NOT clear the display name** when `VITE_DEV_TITLE` is absent — users who have
manually named their dev window via the UI should not have it clobbered on next start.

### 3. No Rust changes required

`document.title` is what the OS reads for the window title bar. The reactive effect at
`app-init.ts ~line 769` already sets it from `resolveWindowName()` → `formatWindowTitle()`.
`UpdateObjectMeta` triggers a wave object update which the reactive atoms pick up, causing
the effect to re-run. No CEF/Rust-side changes needed.

---

## Behavior

| Invocation | Window title |
|---|---|
| `task dev` | `Window 1 — AgentMux` (default, unchanged) |
| `task dev TITLE="agentx"` | `agentx — AgentMux` |
| `task dev TITLE="agentx: PR #1780 testing"` | `agentx: PR #1780 testing — AgentMux` |
| Title > 64 chars | Truncated to 64 chars (matches `DISPLAY_NAME_MAX_LEN`) |

The title applies to the first window only (the window created at startup). Additional
windows opened within the session will still use their default name.

The title persists in the wave object store for the duration of the branch's data dir
lifetime (i.e. it survives hot-reload). On the next `task dev` without a `TITLE`, the
window keeps whatever name it had before — either the prior dev-title or "Window 1".

---

## Out of scope

- `task package` — portable builds do not need this; the build label in the folder name
  already identifies them.
- Multi-window title propagation — only the initial window is titled.
- Clearing the title when `TITLE` is not passed — explicitly out of scope to avoid
  clobbering manually-set names.

---

## Files affected

| File | Change |
|---|---|
| `Taskfile.yml` | Add `TITLE: ''` var + `VITE_DEV_TITLE: '{{.TITLE}}'` env to `dev` / `dev:serve` |
| `frontend/app-init.ts` | After each `initWaveWrap` call site: read `VITE_DEV_TITLE`, call `UpdateObjectMeta` |

---

## Addendum (2026-08-04): agent auto-title in `dev-agent.cmd`

**Problem:** the design above requires the caller to pass `TITLE="..."` explicitly.
Agents invoking `task dev` via `scripts/dev-agent.cmd` (the CLAUDE.md-documented
Windows entry point — see "Launching `task dev` from an agent / MCP Shell" in the
repo's root `CLAUDE.md`) almost never did, so every agent-launched dev window
still showed as plain "AgentMux" in the taskbar — indistinguishable from any other
agent's parallel dev session.

**Fix:** `scripts/dev-agent.cmd` now defaults `TITLE` to the caller's own
`$AGENTMUX_AGENT_ID` (injected into every agent's shell at spawn) when the caller
didn't already pass an explicit `TITLE=` argument. An explicit `TITLE=` from the
caller always wins — this only fills the gap when the argument is absent entirely.

No changes to `Taskfile.yml` or `frontend/app-init.ts` — the existing `TITLE=`
data flow (see "Design" above) is reused unmodified; only the *caller* of `task dev`
changed. This keeps the fix scoped to the one place (`dev-agent.cmd`) that agents
are actually documented to invoke, rather than adding a second default mechanism
inside the Taskfile itself.

See `scripts/dev-agent.cmd` (top-of-file usage comment and the `TITLE_ARG`
computation ahead of the final `task dev` invocation) for the implementation.
