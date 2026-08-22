# Retro: Reopening a Persistent Agent Silently Resumed a Session 9 Days Stale

**Date:** 2026-08-22
**Severity:** High — no data loss on disk, but the user was talking to an
agent that believed it was 9 days in the past, had no memory of ~9 days of
real intervening work (4+ merged PRs), and generated a status summary
describing work that never happened in its own timeline. No error or
warning was ever surfaced.
**Observed by:** the user, directly — noticed the agent ("Camper")
responded with a summary of unrelated work (Warden Supervisor /
harness-model-vendor-decoupling / a fabricated-sounding "Antigravity"
provider) when the actual most recent conversation had been about the
Agent Picker / Armory rename / CLAUDE.md ownership protection work.
**Related:** `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
(root-caused the underlying mechanism two days before this incident),
PR #2693 / #2699 (partial fix, merged the day before this incident), PR
#1479 (introduced `session_backfill.rs`, whose intended cross-channel
continuity guarantee this bug defeats).

---

## TL;DR

The user closed this agent ("Camper") while it was running in an older
AgentMux build, then reopened the same named agent in a freshly-launched
`0.55.20` portable build — a different channel, per this repo's per-build
data isolation. On reopen, the CLI was `--resume`d against the shared
cross-channel registry's `session_id` for Camper. That value was written
**once**, the very first time `backfill_session_ids` ever ran for this
record, and has never been refreshed since — a known, previously
root-caused defect
(`STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`). In this
incident the stale id happened to point at a **real, still-valid session
file** from 2026-08-13 (right after this agent's own PR #2558 merged) —
9 days and roughly 4 more real merged PRs before "now." Because that
session file genuinely exists and is genuinely readable, the CLI's
`--resume` call **succeeded**, not failed. The user saw a fully coherent,
plausible-looking conversation resume — just one frozen 9 days in the
past — with zero errors, warnings, or any other signal that anything was
wrong.

This is a **new manifestation of an old, only-partially-fixed bug class**:
the fix that landed the day before this incident (#2693/#2699) only
covers the case where the stale `--resume` is **confirmed unreachable**
(the CLI rejects it outright). It does nothing for the case reproduced
here, where the stale id is still a perfectly valid, resumable session —
just not the *latest* one. That's a strictly worse failure mode: a
rejected resume at least produces a WARN in the srv log; a stale-but-valid
resume produces nothing at all.

---

## What happened, in the user's words

> "you picked up on a bad history .. i were not working on antigravity ..
> this is what you last said: [a summary of merged PRs #2715, #2731,
> #2732, #2734, #2747 — muxspect Phase A, Agent Picker cleanup, Armory
> rename, CLAUDE.md ownership protection] ... but this is an opportunity
> to fix this .. first of all, it still takes a long time for an existing
> agent to load ... and while it is loading, it is showing garbage, like
> old long-running processes that are s[t]ock[ed]. Then the actual
> conversation that loads is wrong ... note, i closed the previous agent
> in an older version and opened it here, in 55.20 a portable instance"

The agent's own working context, at the time this was raised, was a long
conversation that ended with PR #2558 ("feat(agent): expose model vendor
/ custom endpoint UI + add Antigravity harness") — real work, genuinely
merged, but from **2026-08-13**. The agent had no memory of anything after
that point and, when asked "are you there?", responded as if #2558 were
the most recent thing that had happened.

## Confirming this wasn't a hallucination — ground-truthing against real git history

Before proposing any fix, the actual merge timestamps of both the agent's
own remembered PRs and the user-described PRs were checked directly
against GitHub (`gh pr view <n> --json mergedAt`), not trusted from either
party's memory:

| PR | Title | Merged |
|---|---|---|
| #2552–2557 | Warden Supervisor build-out (this agent's session) | 2026-08-12 22:14 → 2026-08-13 00:16 |
| #2558 | harness/model-vendor UI + Antigravity (this agent's session) | 2026-08-13 07:04 |
| #2715 | muxspect Phase A | 2026-08-22 03:21 |
| #2731 / #2732 | Agent Picker heading/typo fixes | 2026-08-22 18:57 / 19:08 |
| #2734 | Armory rename (Memories → Global/Personal Memory) | 2026-08-22 19:33 |
| #2747 | CLAUDE.md ownership protection | 2026-08-22 22:43 |

Both sets of PRs are 100% real — the same agent identity/conversation
genuinely did all of them, in that order, over roughly 9 real days. This
was never two conversations colliding; it's one continuous conversation
whose active context got rolled back by ~9 days on reopen. Corroborating
evidence from the agent's own transcript: a `currentDate` system message
early in the resumed context read `2026-08-22` (correct), then partway
through — right around a `task changeset` invocation — a *different*
system message announced "the date has changed, today is now
2026-08-13" (the clock moved **backward**), consistent with the harness
re-deriving "now" from a resumed session whose own embedded state was
frozen at that earlier date.

## Root cause

Traced directly against `STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
(written 2026-08-20, two days before this incident) and confirmed still
accurate against the current `agentmux-srv/src/backend/session_backfill.rs`
on `main` as of this writing:

1. Two independent, non-synchronized places track "this agent's session
   id": the local, per-channel `db_agent_instances.session_id` (kept
   genuinely live by `persist_session_id`, but never leaves its own
   channel's SQLite file), and the shared, cross-channel `Registry`
   record's `session_id` — the one place a *different* channel/instance
   actually reads when deciding what to `--resume`.
2. The shared registry's `session_id` is populated by
   `session_backfill::backfill_session_ids`, whose own doc comment says
   plainly: *"Idempotent (skips records that already carry a non-empty
   id)."* Confirmed still true on `main` (`agentmux-srv/src/backend/session_backfill.rs:125`,
   `continue; // already wired`). Nothing in production code ever
   re-validates or refreshes an already-non-empty registry `session_id`
   against what's actually live.
3. **Net effect (unchanged since 2026-08-20):** the shared registry's
   `session_id` for any long-running named agent is only ever as good as
   the *first* value it happened to get, with no self-healing and no
   user-visible warning, for however long the agent keeps running
   continuously in its original channel.

### Why the existing fix (#2693 / #2699, merged the day before this incident) didn't help here

PR #2693 added `find_recovery_session_id` / `find_largest_session_for_working_dir`:
when a `--resume <sid>` attempt is **confirmed unreachable** (the CLI
rejects it, srv logs a WARN), the retry now scans on-disk sessions for the
largest one instead of unconditionally starting blank. This is a real,
well-tested improvement over the pre-#2693 behavior (unconditional blank
fallback) — but it is gated entirely on a **confirmed failure**.

In this incident, the stale registry `session_id` pointed at a session
file from 2026-08-13 that **still exists and is still perfectly readable**
— it's a real, complete, coherent 9-day-old conversation, not a corrupt or
missing file. The CLI's `--resume` call against it **succeeds**. There is
no WARN, no retry, no fallback path triggered at all — `retry_after_resume_failure`
and its new recovery logic never run, because nothing failed. The pane
simply, silently, successfully resumes into the wrong (stale but valid)
conversation. This is a **strictly harder case to detect** than the one
#2693 fixed: a failed resume at least leaves a log trail; a
stale-but-successful resume leaves none.

This matches recommendation #1 from the original status doc, explicitly
flagged there as *"not implemented here"*:

> "Make the shared registry `session_id` a live pointer, not a
> write-once value... or that record should stop being trusted as
> authoritative and instead be recomputed fresh... whenever a `--resume`
> attempt is about to be made rather than relying on a value that might
> be arbitrarily old."

That recommendation is still open on `main` as of this retro.

## Secondary observation (lower confidence, not independently confirmed this session)

The user also reported: *"it still takes a long time for an existing
agent to load... and while it is loading, it is showing garbage, like old
long-running processes that are stuck."* This wasn't traced with the same
rigor as the session-resume issue above (no live repro / log access this
session), but two prior, related fixes are worth checking against for a
possible regression or an uncovered edge case, since both are exactly
"stale process/task state visible in the UI":

- `retro-stuck-background-dock-timer-2026-08-10.md` (issue #2518, fixed
  by #2519/#2520) — Activity Dock rows stuck showing "running" forever
  for backgrounded calls that actually finished synchronously.
- PR #2726 (`fix(agent-pane): refresh Activity Dock subagent rows on
  subagent:abandoned`) — a related staleness fix for subagent rows
  specifically.

Whether the "stuck processes during load" the user saw is a recurrence of
one of these, a new variant, or an artifact of the SAME stale-session-resume
sequence (e.g. the pane briefly rendering process/tracker state left over
from before the resume-vs-fresh decision resolves) is not established
here and needs its own live reproduction.

## Recommended fix direction

Implement the status doc's recommendation #1, since recommendation #2
(#2693's on-disk fallback) is now confirmed insufficient on its own:

1. Stop treating the shared registry's `session_id` as authoritative once
   written. Either propagate `persist_session_id`'s local, genuinely-live
   writes to the shared cross-channel record on every real capture (not
   just the one-time backfill), or re-derive the resume target fresh
   (largest on-disk session for the agent's working dir) every time a
   `--resume` is about to be attempted from a *different* channel than
   the one that last wrote it, rather than trusting a value that may be
   arbitrarily old.
2. Surface *something* to the user when a persistent agent resumes into a
   session whose captured timestamp is suspiciously old relative to the
   pane's own "last active" metadata — even a passive banner ("Resumed a
   session last active 9 days ago") would have made this immediately
   legible instead of requiring a manual PR-timestamp audit to catch.
3. Separately: this agent's own status-summary behavior compounded the
   problem — asked "are you there?", it answered confidently from stale
   context with no hedge ("no open PRs, everything's merged and clean")
   instead of any signal of uncertainty about how much time had passed.
   Not a code fix, but worth noting: a resumed agent has no reliable way
   to know its own context is stale unless the product tells it so (point
   2 above would help this too).

## What this is NOT

- Not data loss. Every real PR from both halves of the conversation is
  genuinely merged and on `main`; nothing was overwritten or lost on
  disk. The only thing "lost" was this one pane's *active working
  context* for ~9 days of the conversation, until the user caught it.
- Not caused by this session's own work. The `camper/harness-model-vendor-ui`
  branch and PR #2558 were real, correctly scoped, and are unaffected by
  this bug — they're simply the point in time the stale resume rolled
  back to.
