# SPEC: "invalid configuration" flashes during a normal window close

**Date:** 2026-09-22
**Status:** implemented (PR #3860) — reproduced under instrumentation on 2026-09-26 (§7); trigger traced end to end (§8); fix (§9) implemented and verified live (§12).
**Reported by:** user, closing a `task dev` instance normally — "for a brief second it said something like *invalid configuration…*"
**Severity:** cosmetic, but alarming — a successful, user-initiated action briefly renders an error screen.
**Revised:** 2026-09-26 by Clamk (§7–§11), at the repo owner's request ("when I close agentmux, I get an 'invalid configuration…' in white text on the background, after the panes are torn down"). §1–§6 are Manoz's original analysis; the reproduction confirms it.

---

## 1. The symptom

Closing an AgentMux window normally (title-bar close, clean shutdown, exit code 0)
briefly paints a full-window error message before the window disappears:

> invalid configuration, client or window was not loaded

Nothing is actually wrong. The shutdown completes successfully.

## 2. Where it comes from

`frontend/app/app.tsx:414-422` (was `:406-413` when first written):

```tsx
<Show
    when={client() != null && windowData() != null}
    fallback={
        <div class="flex flex-col w-full h-full">
            <AppBackground />
            <CenteredDiv>invalid configuration, client or window was not loaded</CenteredDiv>
        </div>
    }
>
```

`client` and `windowData` are `atoms.client` / `atoms.muxWindow`. The guard exists
to catch a **startup** failure — the app came up but its client/window data never
loaded, which is a genuine misconfiguration worth surfacing.

The bug is that the same condition is also true during **teardown**. As the window
closes, those atoms go null (connection torn down, window record cleared), the
`<Show>` flips to its fallback, and a startup-diagnostic renders for a frame or
two on the way out.

So the message is not lying about its condition — `client()` really is null. It is
lying about the *meaning*: it reports "never loaded" for a state that is actually
"already unloaded."

## 3. Why it is worth fixing despite being cosmetic

1. **It trains users to ignore a real error.** This guard is the only signal for a
   genuine startup misconfiguration. If it also fires on every clean exit, it stops
   carrying information.
2. **It is indistinguishable from the real failure.** Nothing in the message or the
   UI separates "config is broken" from "you clicked close." A user reporting it
   cannot tell us which they saw, and neither can we.
3. **It undermines confidence in shutdown.** An error screen during close reads as
   "something went wrong while quitting" even when the exit is clean (verified:
   exit code 0).

## 4. Root-cause shape

This is a **state-vs-lifecycle conflation**: one predicate (`client == null`) is
being used to infer a lifecycle phase (never-loaded) that it does not actually
determine. The same predicate is true in three different phases:

| phase | `client() == null` | correct UI |
|---|---|---|
| starting up, not yet loaded | yes | loading state (or nothing) |
| loaded, genuinely misconfigured | yes | the error |
| shutting down | yes | last frame, or nothing |

Only the middle one should show the error. Any fix has to distinguish phases, not
refine the null check.

## 5. Fix directions (not yet chosen)

1. **Explicit shutdown flag (preferred).** Set a `shuttingDown` signal when close is
   initiated and suppress the fallback while it is set. Most honest — it encodes the
   lifecycle phase the predicate cannot express. Needs a shutdown signal reachable
   from `app.tsx`; check what the close path already sets.
2. **Distinguish never-loaded from unloaded.** Latch "we have successfully loaded at
   least once" and render the error only when that latch is false. Cheap, no new
   plumbing, and directly matches the message's own wording. Does not fix a hypothetical
   mid-session drop to null, but that is arguably a different (real) error anyway.
3. **Grace period before showing the error.** Delay the fallback by a few hundred ms so
   a transient null never paints. Weakest option — it hides the symptom by making the
   real error slower to appear, and is timing-dependent.

Option 2 is the smallest change that is also semantically correct; option 1 is the
most correct if a shutdown signal already exists. Check before choosing.

## 6. Status / why this is parked

Found while smoke-testing PRs #3515 and #3521. It is unrelated to both, it is
cosmetic, and the shutdown it decorates is genuinely clean. Recorded now so it is
not re-discovered from scratch later.

**Before implementing, reproduce it under instrumentation** — the root cause above is
read from code, not observed. The message was seen by eye for under a second, and the
`task dev` stdout log does not contain it (checked: the log ends with normal Vite
reloads and `^C`), which is consistent with a renderer-side paint that never reaches
stdout. Confirm by adding a temporary log in the fallback branch and closing a window.

---

## 7. Reproduced (2026-09-26)

A `task dev` build of main @ `3512f3d23` on Windows was driven over CDP. The
instrumentation added nothing to the app:

- a `MutationObserver` injected into the main window's page, logging the moment
  the fallback text enters the DOM;
- the page's own console, captured with timestamps. The object store (`mos.ts`)
  already logs every `MuxObj deleted <oref>`;
- a real (trusted) mouse click on the title-bar close button,
  `button.window-action-btn.close-btn`, which is the normal user path.

The window was fully loaded first: title "Clamk - tab1 - AgentMux", no fallback,
panes rendered. The timeline after the click:

| after click | event (from the page's console) |
|---|---|
| +222 ms | `MuxObj updated client:…` (srv pruned this window from `Client.windowids`) |
| +223 ms | **`MuxObj deleted window:19008ef1-…`**, so `muxWindow()` becomes null |
| **+233 ms** | **`[probe] FALLBACK TEXT IN DOM: "invalid configuration, client or window was not loaded"`** |
| +237 ms | `MuxObj deleted tab:…` |
| +248 ms | `MuxObj deleted workspace:…` |
| after | the window closes |

§2–§4 are confirmed exactly. The window record's delete reaches a still-live page.
10 ms later the `<Show>` in `app.tsx` swaps the entire app for its startup
fallback, and the window only closes after that. That is the owner's description:
the panes vanish, then white text on the plain background.

A CDP screencast ran alongside, but it did not deliver a frame containing the
text. Its frames arrive throttled and late, and it stopped once the window was
hidden. The DOM probe is the evidence that the fallback was mounted in a live
window. On the owner's machine it stays up long enough to read.

The fallback is the only thing that reacts this way. The client object is
*updated* (not deleted), so `client()` stays non-null. `windowData()` alone trips
the guard.

## 8. The exact trigger path (why the record dies before the window)

`muxWindow` is not stored state. It is a live view of the object store:
`frontend/app/store/window-identity.ts:28` returns
`MOS.getObjectValue(makeORef("window", windowId()))`. When a delete update arrives,
`mos.ts:315` sets that value to `null`. So the page goes into "never loaded" as
soon as srv deletes and publishes the window's record.

Who deletes it, and when:

1. **Closing the main window on Windows** (`agentmux-cef/src/ui_tasks/window.rs`,
   `CloseWindowTask`). For `label == "main"` (`:121`) the host calls
   `backend_close_window` **synchronously, before closing the CEF window**
   (`:133`, then `get_window_on_ui(…)` → close at `:147`). This ordering is
   deliberate (see the long comment above it). An asynchronous notify lost the race
   to `lib.rs` killing the srv sidecar during quit, so the close had to reach srv
   first. The side effect: the page is still alive and on screen for the whole
   `CloseWindow` round trip.
2. srv's `CloseWindow` (`agentmux-srv/src/server/service/window_close.rs`)
   dispatches `CloseWindowInternal`. It applies the events (Window row deleted,
   `Client.windowids` pruned) and **publishes them** (`:190`). Then it runs the
   `delete_workspace` saga (`:234`), which deletes the tabs, blocks and workspace.
   The page receives the window delete first (§7, +223 ms), then the tab and
   workspace deletes.
3. The HTTP call returns, and the host closes the window.

**Every other window** closes through `on_before_close`
(`agentmux-cef/src/client/lifecycle.rs`, `spawning backend_close_window thread`,
`:1430`). There, srv is notified *after* the browser is gone, so nothing is left to
paint. That is why the flash comes with quitting via the main window, which is the
common way people close AgentMux.

The same fallback also shows whenever a window's record is deleted out from under
a live page, for example if an agent or a script calls `CloseWindow` over the API
for an open window. The fix in §9 covers that case too.

## 9. Fix (chosen): "never loaded" vs "unloaded"

§5's option 2 is the right one. The trace in §8 settles the choice between options
1 and 2:

- **Option 1 (explicit shutdown flag)** would need new plumbing. The host would
  have to tell the page "closing" before the synchronous `CloseWindow`, and the
  page learns nothing it can't already infer. It would also only cover the host's
  close path, not an API-driven `CloseWindow`.
- **Option 2 (a "loaded once" latch)** is local to `app.tsx`, needs no host or srv
  change, and is correct for every way the record can disappear. The message
  itself says "was not loaded", and after a successful load that is simply false.
- **Option 3 (grace period)** is still rejected, for the reason §5 gives.

**Change**, all in `frontend/app/app.tsx`, around the `<Show>` at `:414`:

1. `const [loaded, setLoaded] = createSignal(false)`, plus an effect that sets it
   once `client() != null && windowData() != null` has been true. It never resets:
   a window's page is never re-initialized in place.
2. The fallback becomes a small decision:
   - **not loaded yet → the existing error**, unchanged. This keeps the genuine
     startup-failure signal §3 wants to protect;
   - **loaded, and `client`/`window` now gone → the window is being torn down:**
     render `<AppBackground />` only, with no text. The window closes a moment
     later.
3. Put the decision in a pure helper (e.g.
   `appShellState(clientPresent, windowPresent, loadedOnce): "app" | "unloading" | "invalid"`)
   so it can be table-tested without mounting `App`.

The user will still see the panes disappear before the window closes, because the
workspace, tabs and blocks really are being deleted. They will no longer see an
error. See §11 for making the teardown itself invisible.

## 10. Tests

- `appShellState` table (unit, vitest):
  - `(true, true, *)` → `app`;
  - `(false|true, false, false)` and `(false, true, false)` → `invalid`, the
    startup failure, unchanged;
  - `(true, false, true)` and `(false, false, true)` → `unloading`.
- A render test for `App` is optional. Its store and platform setup is heavy. If
  added: mount with client and window set, then null the window, and assert
  `invalid configuration` is **not** in the DOM and the background is.
- Live, repeating §7 with the fix: the same click, and the probe must not log
  `FALLBACK TEXT IN DOM` before the window closes. A dev-build startup that
  genuinely fails to load (e.g. srv down) must still show the error.

## 11. Out of scope (follow-up worth considering)

**Make the teardown invisible, not just error-free.** On the main-window path, the
host could hide the window (`ShowWindow(SW_HIDE)` or the CEF Views equivalent)
*before* the synchronous `backend_close_window`, keeping the notify-before-quit
ordering the comment at `ui_tasks/window.rs` requires. The window would vanish on
click, and none of the pane teardown would be visible. Not proposed here, because
the WRR and quit-watchdog logic reacts to window visibility and lifecycle events
(`should_quit_on_last_window`, the `draining` state), and hiding first has to be
checked against it. The §9 fix is safe regardless and should land first.

**Seen while reproducing, unrelated:** on each of three fresh `task dev` launches
on 2026-09-25/26, the main window's first load stayed blank (bare "AgentMux" title,
no panes) until a reload. That is a separate startup problem and deserves its own
issue.

## 12. Implemented and verified (2026-09-26)

As §9 describes: `appShellState` in `frontend/app/app-shell-state.ts` (table-tested
per §10 in `app-shell-state.test.ts`), and a never-resetting `loadedOnce` latch
in `app.tsx`. The fallback keeps `<AppBackground />` and renders the error text
only in the `invalid` state.

Live, §7 was repeated on a `task dev` build of the fix: the same probe, the same
trusted click on `button.window-action-btn.close-btn`, and a check that the
served `app.tsx` contained the change. The page again received
`MuxObj deleted window:…` (+286 ms after the click), then the tab and workspace
deletes. The probe **never** logged `FALLBACK TEXT IN DOM`. Afterwards the page's
text was empty (background only).

One difference from §7: the window did not go away. The page was left
`visibilityState: "hidden"` and the app kept running. The host log explains it:
`[wrr] quit watchdog: 0 visible, background-service mode — standing down
(resting, not quitting)`. With background-service mode on (#2983), closing the
last window parks the main browser instead of quitting. This build ran on a
fresh per-branch data directory, unlike the §7 instance, so the difference is
that instance's configuration, not new code. It makes the fix matter more: in
that mode a parked page stays alive indefinitely in whatever state the close
left it, and before this fix that state was the startup error.

Not re-checked live: a genuine never-loaded startup (e.g. srv down) still
showing the error. It is the `loadedOnce == false` branch, unchanged from
before, and covered by the `invalid` rows of the unit table.
