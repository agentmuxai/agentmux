# SPEC: "invalid configuration" flashes during a normal window close

**Date:** 2026-09-22
**Status:** draft — root cause identified from code, not yet reproduced under instrumentation. Parked deliberately; see §6.
**Reported by:** user, closing a `task dev` instance normally — "for a brief second it said something like *invalid configuration…*"
**Severity:** cosmetic, but alarming — a successful, user-initiated action briefly renders an error screen.

---

## 1. The symptom

Closing an AgentMux window normally (title-bar close, clean shutdown, exit code 0)
briefly paints a full-window error message before the window disappears:

> invalid configuration, client or window was not loaded

Nothing is actually wrong. The shutdown completes successfully.

## 2. Where it comes from

`frontend/app/app.tsx:406-413`:

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
