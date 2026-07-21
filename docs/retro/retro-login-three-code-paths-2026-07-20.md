# Retro — fixing "no login URL" in two named call sites left a third, unnamed one broken

**Date:** 2026-07-20
**Trigger:** Live verification of `retro-headless-login-browser-open-2026-07-20`'s
fix. A fresh portable build (0.54.1, containing that fix) was launched
specifically to confirm `/login`/"Login Again" no longer dead-end. Instead of
testing those two, the tester clicked the pane's own **"Retry Login"** button
— the one that actually appears when an agent pane fails to authenticate —
and hit the exact same "stuck on Working…" symptom the fix was supposed to
have eliminated.
**Audience:** anyone touching agent-pane login/auth flows. Read this
alongside `retro-headless-login-browser-open-2026-07-20` — that retro fixed
the *mechanism*; this one is about why fixing it in two places wasn't fixing
it everywhere.

---

## 1. What actually happened, with evidence

The "Retry Login" button in `agent-view.tsx`:

```tsx
<button class="agent-retry-btn" onClick={status.startLaunchFlow}>
    Retry Login
</button>
```

(`frontend/app/view/agent/agent-view.tsx:1002-1007`) calls `startLaunchFlow`
— the **gated launch flow** (`runLaunchFlow`,
`frontend/app/view/agent/flows/launch-flow.ts`) — not `relogin`. This is a
third, independent implementation of "log the provider in," and it was never
touched by the previous retro's fix.

The fresh 0.54.1 build's own host log
(`~/.agentmux/channels/local-agenta-release-v0.54.1-*/versions/0.54.1/logs/agentmux-host-v0.54.1.log`)
shows exactly what it did instead:

```
18:20:17.475  run_cli_login: spawned (PTY), waiting for OAuth URL
18:20:32.481  WARN  run_cli_login_pty: no auth URL captured within 15s
18:20:32.484  WARN  [fe] [perf] ipc run_cli_login 15012.1ms
                                  ← the IPC call itself DID return promptly.
                                    Nothing failed here.
   ...(nothing for the next ~5 minutes — no seed_provider_auth_from_global,
       no open_login_terminal, no further auth activity of any kind)...
18:25:33.011  cancel_cli_login: PTY child killed
18:25:33.242  run_cli_login_pty: child exited, exit_code=1
```

Five minutes and one second — `launch-flow.ts`'s own hardcoded
`deadline = Date.now() + 5 * 60 * 1000` — between the 15-second URL-scrape
timeout and the eventual `cancelCliLogin()`. The gap is
`launch-flow.ts`'s Phase 2 (pre-fix, `:198-300`): when `runCliLogin` returned
no URL, it logged two lines (*"attempting to open browser for login..."*,
*"if no browser opened, run the login command manually"*) and fell straight
into an **unconditional 5-minute `CheckCliAuth` poll loop** — waiting for a
credential that nothing was ever going to produce, because nothing else was
attempted. This is the identical "no-url → dead end" bug the previous retro
diagnosed and fixed — just in a call site that fix never reached.

## 2. Why the first fix missed this

`retro-headless-login-browser-open-2026-07-20`'s fix worked by finding every
caller of the shared `forceProviderLogin` helper
(`frontend/app/view/agent/flows/force-login.ts`) and inserting the new
`runProviderLogin` orchestrator in front of it. That covered `/login`
(`commands/global/login.ts`) and "Login Again"
(`useAgentControllerStatus.ts`'s `relogin`) — both of which called
`forceProviderLogin`.

`launch-flow.ts` never called `forceProviderLogin`. It called the
lower-level primitive underneath it — `getApi().runCliLogin(...)` — directly,
with its own hand-rolled URL-open logic and its own hand-rolled poll loop,
predating (or written in parallel with) `force-login.ts`'s extraction. Two
call sites that both wrap the same primitive, maintained independently, is
exactly the shape that lets a fix land in one and not the other — and
because `launch-flow.ts` is what actually runs on **every agent pane launch
while unauthenticated**, not just on an explicit `/login` or a failure-banner
click, it's the highest-traffic of the three, not an edge case.

This is the same class of failure this repo has now named three times on
three different invariants: `retro-provider-auth-isolation-regression-2026-06-05.md`
and `retro-auth-isolation-invariant-silently-orphaned-2026-07-14.md` both
end with a version of "a written fix / invariant is worthless if nothing
re-checks it when the ground moves." Here the "ground" was simply: how many
places in the codebase independently know how to spawn a provider login.
Nobody had counted.

## 3. The fix — one code path, not two-of-three

`launch-flow.ts`'s Phase 2 now calls `runProviderLogin` (the same
orchestrator `/login` and "Login Again" use) instead of `getApi().runCliLogin`
directly, keeping its own poll-after-`"opened"` loop (launch flow explicitly
needs to know the login *completed*, not just that a browser opened, before
declaring `"success"`) but deleting the duplicated URL-open/browser-pane
logic that `forceProviderLogin` already does internally.

After this change, `getApi().runCliLogin` has exactly one caller left in the
entire frontend: `force-login.ts` itself. Every UI surface that can trigger a
provider login — `/login`, the failure-banner "Login Again", and the gated
launch flow (which backs both automatic pane-open login **and** the "Retry
Login" button) — now goes through the same three-tier `runProviderLogin`
(`flows/run-provider-login.ts`): capture a URL, else (Claude) copy a valid
global login, else open a real terminal and poll for the result.

## 4. Reinforcement — making "one code path" durable, not just true today

Writing "there's one path now" in this file doesn't stop a fourth call site
from appearing the same way the third one already existed silently for
months. Per the same lesson the two invariant retros above already drew:

1. **A grep-shaped test, not a sentence.** Add a test (or a lint rule) that
   fails if `getApi().runCliLogin` / `.runCliLogin(` has more than one match
   in `frontend/`. It should stay pinned to `force-login.ts`. This is
   deliberately narrower than testing behavior — it's testing *topology*,
   which is exactly what silently drifted here.
2. **Module-doc the constraint at the source**, not just in this retro:
   `run-provider-login.ts`'s doc comment should say outright that it is the
   only sanctioned entry point for triggering a provider login, so the next
   person adding a login-triggering surface finds the rule before they need
   a retro to find it for them.
3. **When auditing "who calls X," grep for X itself, not for the last
   refactor's name for X.** The previous retro's audit found every caller of
   `forceProviderLogin` — a helper that already had a good name and a test
   file, so it was easy to find. The bug survived in exactly the place that
   *didn't* use the helper. The lesson generalizes: an audit anchored on a
   convenient abstraction will miss whatever never adopted that abstraction.

## 5. What this retro is explicitly not

Not a claim that the three-tier fallback logic itself (URL capture → global
seed → terminal fallback) was wrong — it worked correctly the moment it was
actually invoked, which is exactly what today's live test on the "Marks"
agent went on to confirm once `launch-flow.ts` was fixed. Not a claim the
original retro should have caught this the first time by being more
careful — grepping for a well-named helper's callers is a normal, correct
audit strategy; it just isn't sufficient when a second, independent
implementation of the same behavior exists under a different name. The fix
here is structural (delete the second implementation), not just "look
harder."
