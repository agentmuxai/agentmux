# OAuth Pre-Launch Smoke-Test Diagnostic — 2026-05-14

**Branches under test:** `agenta/oauth-prelaunch-modal` (#847) stacked on `agenta/oauth-prelaunch-controller` (#850).
**Build:** `task dev` at v0.33.876.
**Symptom (latest session):** Click Connect → spinner / Waiting panel shows briefly but no auth URL ever appears in the panel, and the system browser never auto-opens. Earlier session in the same task-dev process showed the browser auto-opening; that behavior has regressed.

---

## 1. What the logs prove vs disprove

Grepping `tasks/beuvhbes1.output` (most recent run):

| RPC | Fired? | Notes |
|---|---|---|
| `resolvecli` (claude) | ✅ twice | 1162ms first call (npm install), 114ms second |
| `auth.start` | ✅ twice (two clicks) | Logged with `session_id=auth-47fa974a…` and `auth-5009de39…` |
| `spawn_auth_cli` task entry log | ✅ both spawns | `auth.spawn: launching provider CLI` lines present |
| `Command::spawn()` failure log | ❌ **never** | If spawn errored we'd see `finish_failure` triggered + `mgr.detach_process` — neither appears |
| Any stdout-drain output (line capture) | ❌ **never** | No `record_line` / `record_url` / `match_line` logs |
| `auth.poll` | ❌ **never** | The frontend never polls the backend |
| `auth.cancel` | ❌ never | Expected — user didn't cancel |

**Key fact:** the entire frontend-side polling pipeline never runs. Without `auth.poll` traffic, we can't tell from the log whether the CLI is alive, hung, or already failed.

---

## 2. Two-track failure

### Track A — Frontend poll loop is silent

After `auth.start` returns successfully, the controller's `AuthFlowController.connect()` is supposed to:

```ts
// auth-flow-controller.ts:182-186
this.dispatch({ type: "SessionStarted", sessionId, authUrl });
void this.pollOnce(sessionId);   // ← added this turn to fire one poll immediately
this.schedulePoll(sessionId);    // ← setTimeout-based recurring poll
```

Neither the immediate `pollOnce` NOR the scheduled one shows up in the backend log. Possibilities:

1. **`SessionStarted` is being dropped by the reducer's kind-guard.**
   Reagent's P1 fix on #849 added: `if (state.kind !== "waiting") { return drop; }` inside `case "SessionStarted"`. The `ConnectClicked` dispatch is supposed to put state into `waiting` BEFORE the `await this.rpc.start(...)`. But Solid's `createSignal` setter returns synchronously, so by the time `await` resumes, `state.kind` should still be `"waiting"`. Unless something between (e.g. another `Selected` dispatch from the `createEffect` in `PreLaunchAuthPanel`) flipped it back.

2. **The `createEffect` driving `controller.selected(...)` re-fires after `ConnectClicked`.**
   ```ts
   createEffect(() => {
       const id = props.identityId();
       const prov = props.provider;
       if (!prov) return;
       controller.selected(prov.id, id, outcomeFor(id));
   });
   ```
   If anything in the parent re-runs this effect (e.g. `provider()` memo invalidating, identity signal touched), `selected` runs → dispatches `Selected` → reducer resets state to `unauthenticated`, `sessionId=""`, kills the pending poll loop. The Connect-click flow then loses its session before the poll ever fires. **This is the most likely root cause.** Selected always calls `this.stopPolling()` unconditionally, so any spurious re-fire kills the timer.

3. **The `pollOnce` short-circuits because state isn't `waiting` at the moment it runs.**
   Same root cause as #2 — but the symptom is "no poll fires", which is exactly what we see.

4. **`setTimeout` simply not running in this CEF tab.** Very unlikely; everything else (Vite HMR, IPC, font loader) works.

### Track B — Browser auto-open regressed

In the **earlier** smoke session (`tasks/blo2t4ue2.output` at 14:51), the browser opened (CLI's own behavior). In the **latest** session (`beuvhbes1.output` at 15:08), the user reports it doesn't.

This is independent of Track A — the CLI process opening the browser is a backend-side, CLI-internal effect. If our `spawn_auth_cli` actually runs `claude.cmd auth login` to completion, `claude` opens the browser itself.

Possibilities:

1. **`.cmd` files spawning differently this session.** Windows behavior: `tokio::process::Command::new("claude.cmd")` works post Rust 1.69 CVE fix. Should be stable across sessions. Unlikely.
2. **The CLI subprocess died before opening browser** because piped stdin/stdout/stderr without a TTY changes its behavior. Claude Code may detect no-TTY and exit. We have NO stderr drain log to confirm — the stderr drain task is silent unless something is written.
3. **Browser-open is racing the controller's `failConnect`** path. If `controller.failConnect` ran early (e.g., because `Selected` cleared state during await), the dispose path may have killed the child via `kill_on_drop` before claude got to open the browser.

---

## 3. Likely root cause (single)

The `createEffect` in `PreLaunchAuthPanel`:

```ts
createEffect(() => {
    const id = props.identityId();
    const prov = props.provider;
    if (!prov) return;
    controller.selected(prov.id, id, outcomeFor(id));
});
```

tracks two reactive sources:
- `props.identityId()` — Accessor from parent's signal
- `props.provider` — a parent's `createMemo` result, also reactive in Solid

When the user clicks Connect, the parent re-renders (because the `authStateKind` signal updates via the OTHER `createEffect`), and any of these can cause this effect to re-fire:
- Vite HMR swap of the controller module while a session is in flight
- `props.provider` memo dependency invalidating (e.g., if the `agent` prop changes shape, though unlikely)
- A parent re-render that causes a stale closure over a stale `props` object

When `controller.selected(...)` runs, it calls `this.stopPolling()` unconditionally AND dispatches `Selected`, which the reducer treats as a fresh selection — clears `sessionId`, kind back to `unauthenticated`.

Result: the session id we just got from `auth.start` is dropped on the frontend, `pollOnce`'s kind-check fails, and no `auth.poll` ever fires. From the user's POV the panel goes back to the Connect CTA or stays mid-render with no URL.

This is consistent with **both tracks**: poll silence AND the CLI being killed by `kill_on_drop` if the controller's dispose fires when the panel re-mounts mid-flight.

---

## 4. Proposed fixes (ordered, least-invasive first)

### Fix 1 — Guard `controller.selected` against re-firing for the same triple

Make `selected` idempotent: if `(providerId, bundleId, outcome)` matches the last call, no-op. This already happens inside the **reducer** (idempotency on `Selected` returns the same state), but the controller's `stopPolling()` runs BEFORE the reducer dispatch — so the timer is killed regardless. Move `stopPolling` to AFTER a dirty-check:

```ts
selected(providerId, bundleId, outcome) {
    const s = this.state();
    if (s.providerId === providerId && s.bundleId === bundleId && selectionKindFor(outcome) === s.kind) {
        return; // no-op — don't kill the poll loop
    }
    this.stopPolling();
    this.dispatch({ type: "Selected", providerId, bundleId, outcome });
}
```

This is the right shape regardless of root cause — the controller shouldn't kill its own session because the view re-fired the same effect.

### Fix 2 — Move the `createEffect` selection-sync to `onMount` + explicit signal write

Replace the implicit `createEffect` watcher with an explicit subscription that only fires on identityId-or-provider **change**, not on every re-render:

```ts
const selKey = createMemo(() => `${props.provider?.id ?? ""}|${props.identityId()}`);
createEffect((prev) => {
    const key = selKey();
    if (key !== prev && props.provider) {
        controller.selected(props.provider.id, props.identityId(), outcomeFor(props.identityId()));
    }
    return key;
});
```

Memo-keyed effect: re-fires only when the (provider, identity) tuple actually changes.

### Fix 3 — Verify CLI spawn doesn't kill_on_drop a live child

If the `tokio::spawn` handle that owns the child is dropped — e.g., the controller calls `auth.cancel` which calls `mgr.detach_process` — the `kill_on_drop(true)` on the `Command` builder will SIGTERM/TerminateProcess the child. Confirm `auth.cancel` is not being called inadvertently from the frontend (e.g., from `dispose()` running during a re-render).

Add a `tracing::info!` log INSIDE the `kill_on_drop`'s effective drop path so we can see CLI-killed-by-cancel events in the log. Currently silent.

### Fix 4 — Open the browser from the host side as a fallback

The spec's §6.1 happy path relies on the CLI to open the browser. But we already have a captured URL once `auth.poll` returns `url-available` — the frontend can ALSO call `getApi().openExternal(authUrl)` to open the browser independently of whatever the CLI does. Belt-and-suspenders.

Add this to `PreLaunchAuthPanel`:

```ts
createEffect(() => {
    const url = controller.state().authUrl;
    if (url && !openedBrowserFor()) {
        getApi().openExternal(url);
        setOpenedBrowserFor(url);
    }
});
```

Track per-URL so we don't re-open on every re-render.

---

## 5. Diagnostic actions before any code change

1. **Add per-step `console.log` in `connect()`** — log entry, after `await rpc.start`, after `dispatch SessionStarted`, after `pollOnce`, after `schedulePoll`. Push, HMR, click Connect, check `[fe]` logs in the dev output for which step is the last one to run.
2. **Add per-step `tracing::info!` in `spawn_auth_cli`** — log right before `child.spawn()`, log the `Result`, log when the stdout drain enters its loop, log every line received. Will distinguish "child died" from "child running but silent" from "spawn errored silently".
3. **Confirm `kill_on_drop` isn't firing** — log inside the `mgr_for_task.detach_process` path and from a `Drop` impl on the join handle wrapper. Will catch the controller-cancel-killed-CLI scenario.

These three log additions cost ~30 lines total and turn the next smoke session into a definitive trace.

---

## 6. What I'm NOT going to do without your sign-off

- Ship Fix 1 alone — it patches the symptom but if the root cause is the re-firing effect, the panel will still flicker for users.
- Ship Fix 4 (host-side browser open) as a workaround — it makes the symptom invisible but leaves the underlying poll-loop bug, which would silently break PR C's success-handling path.
- Touch the backend `spawn_auth_cli` — the backend logs show it executes the launch call; until we add the diagnostic logging in §5, we don't know if it's the proximate cause or just appears so.

---

## 7. Recommendation

Apply §5 step 1 + 2 (diagnostic logging only, no behavior change), rebuild, take one smoke pass, then choose between Fix 1 / Fix 2 / Fix 3 based on what the logs actually say. Total time: ~10 min to instrument, one smoke click, ~5 min to read logs.

This avoids the trap of patch-shotgunning a symptom and ending up with a flow that works for the happy path but breaks subtle re-render races (which is exactly the kind of bug codex would catch on the next PR review round).
