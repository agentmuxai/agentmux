# Retro: `task dev` launched from a stale branch right after "get up to date" was already done

**Date:** 2026-08-19
**Area:** agent workflow (git branch management / dev-instance launch), not app code

---

## 1. Symptom (as reported)

User asked to sync the workspace to latest `agentmuxai/agentmux` `main`, then
launch `task dev` for smoke testing. After launch, the user reported the dev
instance showed version 0.55.8 while the project was at ~0.55.15/16. Before
that, the user had already said "you are far behind," which I disputed.

## 2. What actually happened

- Fetched `origin`, fast-forwarded local `main` to `origin/main`
  (`bb22a8094` → `e626208af`, 72 commits) — this step was correct and I
  verified it (matching commit hashes both before and on a second re-fetch).
- Immediately after, ran `git checkout agent3/widget-bar-parent-submenus-messengers`
  to return to the feature branch, so as not to disrupt the user's
  in-progress work.
- Launched `task dev` from that checkout. The build therefore compiled the
  **feature branch's** HEAD (`768a6dff`, 2026-08-15, v0.55.8), not the
  freshly-updated `main`.
- When the user first said "you are far behind," I checked `main` vs
  `origin/main` and found them byte-identical — true, but the wrong
  question. I never checked what commit the *running dev build* was
  actually compiled from. I answered "is the git ref up to date" and
  treated that as a rebuttal to a claim about "is the thing you built for
  me up to date," which is a different question.

## 3. Root cause

Two separate axes got conflated:

1. Is the git ref up to date? — yes, verified.
2. Is the code the user is smoke-testing up to date? — no, it was built
   from a different checkout than the one I'd just updated.

I updated (1) but launched the build from a checkout that never touched
(1)'s update, and didn't re-verify after the user's first pushback — I was
confident in the repo-state answer and didn't check the actual build
artifact/version before pushing back.

## 4. Fix

- Killed the stale dev instance (v0.55.8, built from the feature branch).
- Checked out `main` directly (now `e626208af`, v0.55.16).
- Relaunched `task dev` from `main`.

## 5. Why this wasn't caught earlier

When the user first disputed "far behind," I checked the nearest available
signal (branch refs, which I'd just touched and had fresh in mind) instead
of the signal that actually mattered for their complaint — the running dev
instance's own reported version. I should have asked "behind what,
specifically" or checked the live app's version before asserting the
numbers didn't support their claim.

## 6. Follow-up

When a task involves both (a) updating a workspace and (b) launching a
long-running dev process in the same conversation, verify the actual
checkout the dev process is built from (`git log -1` in that same working
directory, immediately before invoking the build) — not just that some
other ref was fast-forwarded earlier. If a user disputes a "you're behind"
claim, check the running artifact/version first, not just the git ref,
before pushing back.
