# Retro: `task dev` (and any long-lived foreground GUI process) Gets Killed by `agentmux-bashwrap`'s Idle-Timeout Guard

**Date:** 2026-07-31
**Severity:** Medium — agent-launched dev verification silently loses the app it just built; no data loss, but wastes a full build cycle and is confusing to diagnose without reading the bashwrap-side log line closely.
**Observed by:** AgentA, while verifying PR #2371/#2373's merged fix via a fresh `task dev`.
**Related retros:** `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14.md` (introduced the mechanism that causes this), `retro-task-dev-isolation-multi-agent-2026-06-23.md` (adjacent orphaned-launcher-process history), `retro-task-dev-agent-shell-path-2026-06-27.md` (the PATH/Gap A/B issue `scripts/dev-agent.cmd` already solves — orthogonal to this retro), `retro-persistent-agent-working-status-stuck-2026-07-16.md` (**the actual prior art for the correct fix** — see Correction below).

---

## Correction (same day, after user feedback)

The first version of this retro recommended detaching `task dev` entirely from the Bash tool via PowerShell `Start-Process`. That **does** avoid the idle-kill, but has a real cost the first draft didn't weigh: AgentMux's own UI has **two** separate live-process surfaces —

1. **`ActivityDock`** (`frontend/app/view/agent/components/ActivityDock.tsx`, pinned directly above the composer/input in an Agent pane) — driven entirely by the **conversation transcript**: a Bash `ToolNode` gets promoted into the dock once its `status` has been `"running"` for ≥30s (`tool-adapter.ts`, `TOOL_PROMOTION_MS`). It has no concept of OS PIDs at all.
2. **`AgentProcessRegistry`** (`agentmux-srv/src/backend/process_tracker/registry.rs`) — real Job-Object-based OS process tracking, feeding the "⚙N" badge / Swarm roster. It only sees descendants inside the agent's own per-block Job Object.

A fully detached `Start-Process` child is invisible to **both** — it was never a tool call AgentMux observed (nothing for the dock to promote) and it's not inside the agent's Job Object (nothing for the registry to enumerate). That's exactly what a user reported: the dev instance was alive and healthy, but nowhere to be found in the app's own UI.

**The actual prior art**, found in `retro-task-dev-isolation-multi-agent-2026-06-23.md`'s sibling, `retro-persistent-agent-working-status-stuck-2026-07-16.md`: another agent had already solved this exact idle-kill problem — *without* detaching — by keeping `task dev` as a normal `run_in_background: true` **Bash tool** call (so it gets a real, dock-visible `ToolNode`) and defeating the idle-timeout with a **backgrounded heartbeat loop** instead:

```bash
cd "<repo root>" && (
  while true; do sleep 120; echo "[heartbeat] dev still alive $(date +%H:%M:%S)"; done &
  HEARTBEAT_PID=$!
  trap "kill $HEARTBEAT_PID 2>/dev/null" EXIT
  ./scripts/dev-agent.cmd TITLE="<label>"
)
```

120s < bashwrap's 600s idle-timeout default, so the wrapper never goes quiet long enough to fire — and because this whole thing is still one bashwrap-wrapped Bash tool call, its `ToolNode` stays `"running"` in the transcript the entire time, which is exactly what the dock is designed to surface. This is the **corrected, recommended pattern** — see below. The detach approach is demoted to a fallback for cases where dock visibility genuinely doesn't matter (see "Recommended pattern," revised).

**Known, pre-existing, unrelated-but-adjacent side effect** (documented in the 07-16 retro, not something this fix introduces or is expected to solve): the agent pane's own turn-phase status indicator has no distinct state for "a long-running attached process is alive" — it collapses into the same generic "Working…/Waiting…" affordance used for an active model turn, so the pane may *look* like the agent itself is busy the whole time `task dev` runs. This is a known UX gap (fix direction sketched in that retro's "Fix direction" section) — not a new bug, and out of scope for this retro to fix.

---

## TL;DR (original investigation, still accurate as root-cause analysis)

`scripts/dev-agent.cmd` (the documented, correct way to launch `task dev` from an agent shell — see repo `CLAUDE.md`) still isn't safe to run as a *plain* foregrounded-then-backgrounded Bash tool call with no keepalive. The build succeeds, Vite comes up, the launcher starts the GUI — and then, once the terminal goes quiet (which it always will without a keepalive: a GUI app produces no further stdout once its window is up), `agentmux-bashwrap`'s idle-timeout guard decides the command is "stuck" and kills it, taking the freshly-built dev instance down with it. This is the **exact false-positive risk `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14` explicitly flagged as an open risk** (see its Open Question #6 / the `find /` example in "Corroborating live evidence") — this retro is the concrete case of it actually happening, for a different trigger (a legitimately-silent-forever GUI, not a slow-but-finishing search).

**Fix (corrected — see above):** keep `task dev` as a normal `run_in_background: true` Bash tool call (dock-visible) and wrap it in a backgrounded heartbeat loop to defeat the idle-timeout. A fully-detached `Start-Process` launch also avoids the kill, but sacrifices all dock/registry visibility — use only when that tradeoff is acceptable.

---

## What happened, in order

1. Launched `scripts/dev-agent.cmd` via the Bash tool with `run_in_background: true`, to verify the just-merged PR #2371/#2373 fix in a live dev build.
2. Waited, checked the output file periodically — looked empty/stalled for a while, but `tasklist` confirmed `task.exe` was genuinely alive and building (later corroborated by `mem_attribution` telemetry in the shared srv log showing `rustc.exe` consuming ~1.9GB — a real, in-progress compile).
3. ~17 minutes later, got a `task-notification` reporting the background command **failed with exit code 1**.
4. Reading the captured output: the build had actually **fully succeeded** — `cargo build --release` finished clean (6m13s), the host/launcher/CEF build finished, Vite came up (`VITE v6.4.3 ready`, `Vite ready at http://localhost:5290`), and the launcher was invoked (`AGENTMUX_DEV=1 ./agentmux-launcher.exe --url=...`). The **very last line** in the captured output was:
   ```
   [bashwrap] command produced no output for the idle timeout and was terminated automatically (likely blocked on a pager or other interactive prompt this wrapper can never answer, e.g. `git diff`/`log`/`show` auto-paging output that doesn't fit one screen). Try `git --no-pager <cmd>` or `| cat` on future invocations.
   ```
5. Checked `tasklist` for the newly-launched dev instance's processes: **none found** — no `agentmux-launcher.exe`, no new `agentmux-cef.exe` beyond the pre-existing, unrelated installed instances. The dev instance had been killed as a side effect of the wrapping shell being terminated.

## Root cause

This is not a new bug in `agentmux-bashwrap` — it's the **intended, working-as-designed** idle-kill mechanism from `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14` (default 600s / 10 min of zero PTY output → kill), correctly closing the pager-hang leak it was built for, but firing on a **structurally different case it was never meant to cover**:

- The pager-hang case: a command that **should** exit quickly, is blocked forever on an unanswerable prompt (`less` waiting for a keystroke nobody will send), and **will never produce useful output no matter how long you wait** — killing it is strictly correct.
- The `task dev` case: a command that **succeeds** by transitioning into "a GUI window is now open and running indefinitely" — which, from the terminal's perspective, looks identical to "silent for 10 minutes" because **a running GUI app legitimately produces zero further stdout once its window is up.** Killing it destroys a working, wanted result.

`agentmux-bashwrap`'s idle-timeout has no way to distinguish these two cases from the PTY's byte stream alone — both look like "no bytes for N minutes." `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14` already named this exact ambiguity as an open risk (its `find /`-search example: "a long-silent-but-still-working command... is indistinguishable, from bashwrap's perspective, from a genuinely stuck one") but only evaluated it for commands that eventually **exit** on their own (a slow search that finishes). It didn't consider the GUI/daemon case, where the command is **designed to never exit** on that terminal at all.

Two smaller, previously-known contributing factors from `retro-task-dev-isolation-multi-agent-2026-06-23.md`:
- Bashwrap's background-task output capture showed 0 bytes for a long stretch even while the wrapped process was genuinely alive and progressing — a red herring that made it hard to tell "still building" from "silently stuck" without cross-checking `tasklist`/`muxlog` directly. (That retro's own action item to investigate this was never checked off.)
- The launcher, once up, is a process the wrapping shell has no further reason to interact with — exactly the shape of thing the idle-timeout was never designed to co-exist with.

## Why `scripts/dev-agent.cmd` alone doesn't fix this

`dev-agent.cmd` solves a **different, already-well-documented** problem (`retro-task-dev-agent-shell-path-2026-06-27.md`): MSYS2 bash not resolving `.cmd` files, and cmd.exe not finding `bash.exe` on its registry PATH. It does nothing about *how the agent's tool invokes the wrapper script* — if that invocation still goes through the Bash tool (i.e. through `agentmux-bashwrap`'s PTY-wrapped exec path), the idle-kill guard is still watching it and will still fire once the GUI goes quiet.

## The fix (verified working)

Launch it as a **fully detached OS process**, bypassing `agentmux-bashwrap` and its idle-timeout entirely, using PowerShell's `Start-Process`:

```powershell
Set-Location "<repo root>"
$proc = Start-Process -FilePath "cmd.exe" `
  -ArgumentList '/c', 'scripts\dev-agent.cmd', 'TITLE="<label>"' `
  -WorkingDirectory "<repo root>" `
  -WindowStyle Hidden `
  -PassThru `
  -RedirectStandardOutput "<repo root>\task_dev_detached.log" `
  -RedirectStandardError "<repo root>\task_dev_detached.err.log"
Write-Output "Started detached PID: $($proc.Id)"
```

`Start-Process` returns immediately with the child's PID; the child (and everything it spawns — `task.exe`, the Rust build, Vite, the launcher, the CEF windows) is **not** a descendant of any `agentmux-bashwrap.exe` PTY session, so there is no idle-timeout watching it at all. Redirecting stdout/stderr to a plain log file (rather than a PTY) means there's also no pager-auto-invoke risk from that layer either — a second, incidental benefit.

**Verification:** after the wrapped run had already been killed (confirmed via the "terminated automatically" message and an empty `tasklist` for launcher/cef processes), relaunched via the pattern above. Several minutes later, `tasklist` showed `agentmux-launcher.exe` and eight-plus `agentmux-cef.exe` processes alive and running — the dev instance stayed up well past the point where the previous, bashwrap-wrapped attempt had already been terminated.

## Recommended pattern going forward (corrected)

**Rule of thumb: does the command's *success state* involve a process that keeps running indefinitely with no further stdout (a GUI app, a daemon, a long-lived server)? If yes, it needs a keepalive so the Bash tool's idle-timeout never fires — it does NOT need to be detached from AgentMux's own tracking, and detaching should be the last resort, not the default.**

- **Bash tool / `agentmux-bashwrap`, plain (no keepalive)** — correct for anything expected to **terminate** on its own, however long that takes (builds, test suites, `git` commands, `find`, scripts). The idle-timeout is a safety net *for this class*.
- **Bash tool / `agentmux-bashwrap`, with a backgrounded heartbeat loop, `run_in_background: true`** — the **default correct choice** for a long-lived attached process you want the user to see and manage from the app itself (`task dev`, a manually-started server, a long-lived watch process): wrap the real command in a `while true; do sleep <interval < idle-timeout>; echo heartbeat; done &` keepalive with a `trap ... EXIT` cleanup, exactly as shown above. This keeps a real `ToolNode` alive in the transcript — which the `ActivityDock` promotes to visible after ~30s — so the process shows up in the app's own dock, matching what a user watching the Agent pane expects to see.
- **Fully detached process (PowerShell `Start-Process`)** — fallback **only** when dock/registry visibility genuinely doesn't matter (e.g. a throwaway verification you'll tear down yourself and never need the app to show). Invisible to both the dock and `AgentProcessRegistry` — the user has no way to discover or stop it through the app's own UI, only via `tasklist`/direct PID management.

### A second, related but distinct finding: a rejected/blocked tool call can leave a permanently-stuck dock entry

While investigating the above, a user also noticed a `sleep 45` entry still showing as "active" in the dock with no matching OS process anywhere (confirmed via `tasklist` and `Get-CimInstance Win32_Process` — nothing running). Root cause, confirmed by reading the frontend:

- A Bash tool call creates its `ToolNode` (`status: "running"`) the instant the CLI's own `tool_use` content block starts (`claude-translator.ts`'s `handleContentBlockStart`), and only clears it on a matching `tool_result` (matched by `tool_use_id`).
- The `sleep 45` call in question was **rejected by the Claude Code harness itself** (a built-in guardrail against standalone `sleep` calls, external to this repo — confirmed via a full-repo search: no "standalone sleep"/"Blocked:" validator exists anywhere in `agentmux-srv` or `agentmux-bashwrap`; the one hook agentmux does install, `agentmux-bashwrap/src/hook.rs`, only ever rewrites commands, never denies them).
- The only cleanup path for an orphaned `"running"` `ToolNode` is `scrubOrphanedInProgress` (`frontend/app/store/agent-document/reducer.ts`), which only fires at a **session boundary** (`SessionEnd`, `HistoryRestored`, `HistoryLoaded`) — not immediately when a tool call is rejected mid-turn. This is a known, already-named class of bug (see `SPEC_ORPHAN_THINKING_NODES_2026_05_27.md`), just not one this retro's fix addresses.
- **Practical takeaway:** this specific stuck entry is expected to clear on the pane's next reload/session boundary, not before. There's no lower-cost way to force it from an agent's own tool access. Not fixed as part of this retro — flagged as a known, pre-existing gap.

### Concrete follow-ups (not yet implemented)

- [ ] Update this repo's `CLAUDE.md` (or `scripts/dev-agent.cmd`'s own header comment) to state the heartbeat-loop pattern explicitly, so the next agent session doesn't have to rediscover it via two different retros (this one and `retro-persistent-agent-working-status-stuck-2026-07-16.md`).
- [ ] Add the distinct "long-running attached process" pane status from `retro-persistent-agent-working-status-stuck-2026-07-16.md`'s "Fix direction" — this would resolve the "pane looks perpetually Working" side effect the heartbeat pattern causes, which is the main remaining rough edge of the *correct* fix.
- [ ] Consider whether `scripts/dev-agent.cmd` itself should emit its own internal heartbeat (so callers don't need to remember to wrap it) — would close this gap for any caller, not just agents remembering the pattern.
- [ ] Consider whether `agentmux-bashwrap`'s idle-timeout should special-case a detected GUI/daemon launch directly — lower priority given the heartbeat-loop workaround is simple, precedented (twice now), and doesn't require touching bashwrap's core exec path again (which `RETRO_BASHWRAP_PAGER_HANG_LEAK_2026_07_14`'s five review rounds show is easy to subtly regress).
- [ ] Give `scrubOrphanedInProgress` (or a new mechanism) a way to detect and clear an orphaned `"running"` `ToolNode` sooner than the next session boundary — e.g. a max-age fallback even for `"running"` status, mirroring the retention windows already used for terminal states in `ActivityDock`'s own `types.ts`.
- [ ] File a note (or fold into the existing pager-hang retro's Open Questions) that Open Question #6 there — the false-positive risk of the idle-kill — has now been observed in a second, distinct form (a GUI dev server, not a slow search) and has a verified, precedented mitigation (the heartbeat-loop pattern, corroborated independently by two different agent sessions).

## Timeline

| Order | Event |
|-------|-------|
| 1 | Launched `scripts/dev-agent.cmd` via the Bash tool (`run_in_background: true`), no keepalive, to verify PR #2371/#2373's merged fix. |
| 2 | Polled intermittently; output file appeared stalled, but `tasklist`/shared-log `mem_attribution` telemetry confirmed a genuine, in-progress `rustc.exe` compile. |
| 3 | ~17 minutes in, received a `task-notification`: background command failed, exit code 1. |
| 4 | Read the captured output: full build success (backend + host + CEF + bundling), Vite came up ready, launcher was invoked — then the final line showed `agentmux-bashwrap`'s idle-timeout kill message. |
| 5 | Confirmed via `tasklist` that no launcher/cef process from this run survived — the dev instance was killed along with the wrapping shell. |
| 6 | Relaunched via PowerShell `Start-Process` with `-WindowStyle Hidden` and output redirected to plain log files, fully detached from the Bash tool's bashwrap-wrapped PTY. Verified via `tasklist`: `agentmux-launcher.exe` + multiple `agentmux-cef.exe` alive well past the point the previous attempt had already been killed. First-draft retro written recommending this as the general pattern. |
| 7 | User feedback: the detached instance doesn't show up in the Agent pane's own dock (`ActivityDock`, pinned above the input), and a stale `sleep 45` entry is still showing "active" with no matching process. |
| 8 | Investigated both: (a) the dock is driven entirely by conversation-transcript `ToolNode` status, not OS processes — a detached process is invisible to it and to the separate `AgentProcessRegistry` by design; (b) found the actual prior art (`retro-persistent-agent-working-status-stuck-2026-07-16.md`) — another agent had already solved the idle-kill problem via a backgrounded heartbeat loop, keeping the process as a normal, dock-visible Bash tool call. |
| 9 | Cleaned up the detached instance (`taskkill /PID <launcher-pid> /F`), relaunched via the corrected pattern — `run_in_background: true` Bash tool call wrapping `dev-agent.cmd` in a 120s heartbeat loop. |
| 10 | Investigated the stuck `sleep 45` dock entry: confirmed the sleep-block is a Claude Code harness built-in (no matching validator anywhere in this repo's source), and the stuck-`ToolNode` cleanup (`scrubOrphanedInProgress`) only runs at session boundaries — not an immediately-fixable-from-here issue, documented as a known, separate gap. |
| 11 | This retro corrected in place (see "Correction" section at top) rather than superseded by a new file, since the root-cause analysis of the idle-kill mechanism itself was still accurate — only the recommended fix changed. |
