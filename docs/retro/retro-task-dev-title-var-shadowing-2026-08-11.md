# Retro: `task dev TITLE=...` silently ignored (go-task var-shadowing)

**Date:** 2026-08-11
**Owner:** Agent3
**Area:** `Taskfile.yml` (`dev`, `dev:serve` tasks) / dev window titling

---

## 1. Symptom

Asked to set a `task dev` window's OS title to the agent's own name via
`scripts\dev-agent.cmd TITLE="Agent3"` (per this repo's documented pattern for
telling parallel dev sessions apart). The resulting window instead showed the
generic positional fallback, `Window 1 - tab1 - AgentMux`, indistinguishable
from another agent's simultaneously-running dev instance — which is what
triggered the user's initial "your instance isn't running" read: two windows
with the same generic title look like one window, not two.

## 2. Root cause

`Taskfile.yml`'s `dev` and `dev:serve` tasks each declared:

```yaml
vars:
    TITLE: ''
```

In go-task 3.48.0, a task's own `vars:` block is not a *default* in the usual
sense (only-fill-if-unset) — it **unconditionally overwrites** any value
supplied by the caller for that same variable name, whether the caller is a
CLI invocation (`task dev TITLE=Agent3`) or a parent task's own `vars:` pass-through
(`task: dev:serve` with `vars: TITLE: '{{.TITLE}}'`). Confirmed via a
from-scratch isolated repro (`/tmp/tasktest/Taskfile.yml`, deleted after
confirming) with both quoted and unquoted CLI values — identical result
either way, so quoting was never the variable at play.

This meant `{{.TITLE}}` inside `dev:serve` always rendered `''` regardless of
what was passed at any call site, so `VITE_DEV_TITLE` was always baked in
empty. `frontend/app-init.ts:400-408` reads `import.meta.env.VITE_DEV_TITLE`
at startup and calls `ObjectService.UpdateObjectMeta` with it — with an empty
string, the window's `window:displayname` meta is never set, so
`resolveWindowName()` (`frontend/util/window-title.ts`) falls through to its
third tier, `Window ${index + 1}`.

Confirmed live (not just by code reading) via CDP against the actual running
dev instance: `import.meta.env` showed `DEV: true` but `VITE_DEV_TITLE: ""`,
matching the theory exactly before any fix was applied.

## 3. Fix

Removed the `vars: TITLE: ''` declaration from both `dev` and `dev:serve`.
Go templates render an unset variable as `""` by default, so the "no TITLE
passed" case behaves identically to before; the only change is that a
CLI-supplied or parent-task-supplied value is no longer shadowed. Verified
against an isolated repro Taskfile matching `dev:serve`'s exact `env:`-block
structure for both the empty-TITLE and `TITLE=Agent3` cases before applying
to the real file.

## 4. Why this wasn't caught earlier

The wrapper script `scripts/dev-agent.cmd` already auto-defaults `TITLE=` to
`$AGENTMUX_AGENT_ID` when the caller doesn't pass one explicitly — most
day-to-day usage never exercises an *explicit* `TITLE=...` override at all,
since the common case (a single agent, one dev session) never needs to tell
sessions apart. The bug only surfaces when two dev instances are running
side by side and someone deliberately passes a distinguishing title — an
infrequent path, and the failure mode (falls back to a plausible-looking
default, "Window 1") doesn't look like an error, so it wasn't obviously
broken until directly compared against a second instance.

## 5. Fixing the already-running instance

The Taskfile fix only affects *future* `task dev` launches — an
already-running dev instance had already baked in the empty
`VITE_DEV_TITLE` at Vite startup (env values are replaced at build/serve
time, not re-read at runtime). Relaunching was avoidable: the same
`window:displayname` meta the app itself writes is settable directly via the
app's own RPC (`RpcApi.SetMetaCommand(TabRpcClient, { oref: WOS.makeORef("window",
windowId), meta: { "window:displayname": "Agent3" } })`), reached over CDP
(`Runtime.evaluate` against the instance's remote-debugging port — see
[[cdp-live-verify-technique]] in agent memory) since `mcp__agentmux__SetName`
only reaches the invoking agent's own hosting window, not a separately
launched `task dev` process. After the RPC call, the frontend's local
reactive cache for the window object needed an explicit `WOS.reloadWaveObject(oref)`
to pick up the change — `SetMetaCommand` updates the backend but doesn't by
itself push a refreshed value into the already-subscribed local atom.
Confirmed both `document.title` and the native OS title bar
(`Get-Process | Select MainWindowTitle`) updated to `Agent3 - tab1 - AgentMux`
after the reload.

## 6. What went well

- Root-caused with an isolated, from-scratch repro before touching the real
  Taskfile, so the fix was verified in isolation first.
- Live-verified against the actual running instance (CDP) rather than
  trusting code-reading alone, both for diagnosing the empty env var and for
  confirming the eventual live title patch took effect.

## 7. Follow-up

None identified — the fix is a straight deletion of a shadowing declaration
with no behavioral change to the unset-TITLE case, and the live-patch
technique for an already-running instance is documented here for reuse
rather than needing a dedicated tool.
