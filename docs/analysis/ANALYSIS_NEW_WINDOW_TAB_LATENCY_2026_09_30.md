# Opening a new window tab: where the ~650 ms goes

**Date:** 2026-09-30
**Status:** analysis. Findings are in §2 and recommendations in §3. Recommendation 1 (the install-check storm) is implemented in #4106; 2 and 3 are not yet.
**Author:** korp
**Trigger:** Repo owner, 2026-09-30: *"opening a new tab is still quite slow ... any ideas regarding how we can make that faster?"*
**Related:** `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` (switching between existing window tabs, same method), `SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md` (why `createTab` builds the tab inactive and activates it afterwards), `SPEC_TAB_CONTENT_REVEAL_GATE.md`.

Code citations are against `main` @ `1ed3044e3`.

---

## 1. Summary

A new window tab (Alt+T, the menu, or the title-bar menu, all of which call `createTab()` in `frontend/app/store/tab-actions.ts`) takes about **650 ms** before it is fully on screen. That is 400 ms of building it hidden, then an 80–150 ms settle gate, then a 120 ms fade.

Most of the 400 ms is a bug. **The default preset's agent picker sends about 700 `install.check` RPCs every time it mounts.** Its "check every agent's CLI" effect re-subscribes to its own results, so each returning check re-issues the check for every agent still pending. About 700 round trips, each with its own microtasks and store update, keep the page's main thread busy for the whole build. Every other reply, such as the next pane's `CreateBlock` or a host IPC, waits behind them in the queue.

## 2. Measured

**Setup:**
- a fresh dev instance on `main`, with the default preset (agent picker, sysinfo, swarm);
- the owner's own window-tab creations from the host log, plus 3 more traced through DevTools (Alt+T via `Input.dispatchKeyEvent`);
- a trace with screenshots, and `Network.webSocketFrame*` capture for 1.5 s idle and 1.5 s from the keypress.

**Timeline of one new window tab** (host log, from the `CreateTab` call; 5 creations, ranges):

| Step | Ends at | Cost |
|---|---|---|
| `workspace.CreateTab` | +6–17 ms | 6–17 ms |
| `waitForLayoutModel` + agent-picker `CreateBlock` | +14–38 ms | 3–10 ms |
| **sysinfo `CreateBlock`** | **+151–309 ms** | **128–256 ms** |
| swarm `CreateBlock` | +236–373 ms | 39–127 ms |
| `SetActiveTab` | +267–414 ms | 9–41 ms |
| reveal gate lifts (`source=settle`) | +349–554 ms | 81–150 ms |
| View Transition fade | about +470–670 ms | 120 ms |

**Why `CreateBlock` takes 130–250 ms when the server answers in about 40 ms:**
- In the trace, the sysinfo `CreateBlock` response arrives about 38 ms after the request, yet the frontend logs it as taking 164 ms.
- Host IPCs sent in the same window take 160–250 ms. `main_window_focus` is logged at 167–179 ms, although its handler only posts a task and returns.
- The host's UI thread is idle through that gap.
- The page's main thread (`CrRendererMain`) has no idle gap longer than 25 ms in the first 400 ms. Replies are queued behind page work.

**The page work:** 340 of the first 420 ms are busy on the main thread. Of that time:
- `RunMicrotasks`: 243 ms across 858 runs;
- `Receive mojo message`: 206 ms across 1,004 messages;
- style, layout and pre-paint together: about 105 ms, all in slices under 30 ms, so none of it is a long task and the gate can't see it.

**WebSocket traffic:**

| 1.5 s window | Sent | Received |
|---|---|---|
| idle | 3 | 4 |
| from Alt+T | 799 | 818 |

The 775 RPCs sent after Alt+T, by command:

| Count | Command |
|---|---|
| **696** | `install.check {providerId: "claude"}` |
| 4–9 each | `install.check` for codex, antigravity, gemini, openclaw, copilot and pi |
| about 25 | everything else: `eventsub`, `listagents`, `listrecentsessions`, sysinfo history and so on |

**The bug**, `frontend/app/view/agent/components/AgentPicker.tsx`, "Refresh install state whenever the agent list changes":

```ts
createEffect(() => {
    for (const agent of agents()) {
        if (!(agent.id in installState())) void checkInstalled(agent);
    }
});
```

The effect tracks `installState()`. `checkInstalled` only writes an agent's entry once its RPC returns, so nothing marks an agent as in flight. Every result re-runs the effect, which re-issues the check for every agent whose result hasn't arrived yet. With *n* agents that is about n²/2 calls; n ≈ 37 gives about 700, and 696 were measured. It is also per agent, not per provider. Almost all agents share the `claude` CLI, so every one of those calls asks the same question.

**Scope:** this runs whenever an agent picker mounts:
- every new window tab (the default preset);
- every window tab with a picker at launch;
- a new agent pane.

It grows with the number of agents the user has defined.

## 3. Recommendations, in order

1. **Fix the storm (bug; about 200–300 ms per new window tab).**
   - Keep a non-reactive `Set` of agent ids already requested, and read `installState` untracked (`untrack`) inside the effect, so a result can never re-trigger checks.
   - Check **per provider, not per agent**: one `install.check` per distinct `(providerId, cliCommand, npmPackage)`, fanned out to every agent using it. With today's providers that is at most 7 calls instead of about 700.
   - Share the per-provider result across pickers with a short TTL (a module-level `Map<providerKey, Promise<InstallCheckResult>>`, invalidated by the install flow's existing "re-run install.check after install" path). Then a new window tab's picker issues **0** checks when another picker checked recently.
   - Add a regression test: mount the picker with 40 agents on one provider, resolve the checks one by one, and assert that exactly 1 `install.check` is issued.
2. **Show a freshly built window tab without the gate and fade.** `createTab` already waits until the preset's panes exist before activating (`SPEC_TAB_CREATION_REVEAL_ARCHITECTURE_2026_09_16.md`). After fix 1 the build no longer leaves catch-up work behind, so the 80–150 ms settle and the 120 ms cross-fade only delay a tab that is ready. Mark the new tab shown as soon as it is built, and activate it through the same instant path warm switches use. Measure first that nothing pops in after the reveal (the method in `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` §6).
3. **Build the preset in one round trip.** Today it is a poll (`waitForLayoutModel`, every 30 ms) followed by three sequential `CreateBlock` calls. Each is cheap once the main thread is free, but they are serial. Either create the three blocks concurrently and apply the splits once all ids are known, or add a server-side "create tab with preset" that returns the tab with its blocks and layout in one reply.
4. **Show the new window tab's pill immediately** (optional, Chrome-style). The pill appears once `CreateTab` returns, which is 6–17 ms today and fine. Revisit only if 1–3 leave the pill visibly ahead of the content.

**Expected after 1 + 2:**
- about 30–60 ms of `CreateTab` + `CreateBlock` round trips;
- one frame to show the tab.

That is roughly **100 ms instead of about 650 ms**, to be confirmed by re-running §2.

## 4. How this was measured

The DevTools tooling is the same as `ANALYSIS_WINDOW_TAB_SWITCH_PAINT_2026_09_30.md` §6, plus:
- `Network.enable` with `webSocketFrameSent/Received`, grouped by `message.command` and the first 70 characters of its data;
- per-thread idle gaps (`CrBrowserMain`, `Chrome_IOThread`, `CrRendererMain`) from the trace, to tell a blocked host from a busy page.
