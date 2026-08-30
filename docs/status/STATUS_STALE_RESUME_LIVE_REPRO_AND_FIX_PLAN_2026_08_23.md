# Status: Live Repro of Cross-Channel Stale `--resume` — Recovery Works, UX Gap Remains

> **RESOLVED 2026-08-29 (docs-cleanup Phase 3) — §6's fix plan has shipped.**
> - **§6.1** (make the shared registry `session_id` a live pointer — "highest
>   leverage, addresses root cause") → **#2755**, `fix(agent): make
>   cross-channel resume session_id a live pointer, not write-once`.
> - **§6.2** (surface the recovery attempt to the user — "the UX gap this
>   report is actually about") → **#2776**, `fix(agent): surface a
>   Reconnecting… status during stale-resume retry`. **#2833** went further,
>   making the resume-vs-new outcome known at pane open.
> - **§6.3** (reword the misleading retry-decision log line at
>   `persistent.rs:3313`) → the phrase it quotes ("retrying fresh, without
>   `--resume`") no longer appears anywhere in the Rust sources, and the
>   resume path has since been extracted into
>   `agentmux-srv/src/backend/blockcontroller/persistent_resume.rs`. Treat as
>   moot rather than confirmed-fixed: this sweep verified the misleading
>   string is gone, not that it was removed deliberately.
> - **§6.4** was explicitly *not* recommended as urgent and remains
>   uninvestigated — correctly so.
>
> Everything below is preserved as the original investigation record.

**Date:** 2026-08-23
**Status:** Root cause confirmed via live incident on this exact machine/agent. Closes the
outstanding live-repro item from `docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md`
(issue #2368). Recovery mechanism (#2693/#2699) confirmed working correctly. Two
recommendations from `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`
remain unimplemented — this document narrows and re-confirms them with fresh evidence,
proposes concrete fixes, and does not change code.

**Trigger:** operator observed "a brief period where nothing was happening, for like 30s...
thought it was a crash, but it recovered" while working in their own agent pane, about a
minute before asking about it.

---

**CORRECTION (2026-08-23, before implementation started):** §6.1 below ("make the
registry a live pointer") was already shipped the day before this incident —
`db06a2168` / PR #2755, *"fix(agent): make cross-channel resume session_id a live
pointer, not write-once"*, merged 2026-08-22. `registry_mirror.rs`'s
`registry_propagate_continuation_session_id` (added by that PR) is called from
`instance_update_partial` whenever a genuine `session_id` write happens, and correctly
excludes the exact false-positive case `registry_upsert_if_named`'s own filter creates
for continuation rows. This was missed in the original research pass (which read
`core.rs`'s `persist_session_id`/`sync_instance_session_id` but not
`registry_mirror.rs`, which those functions call into) and because the original
`STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md` predates the fix by two
days. **§6.1 is not being (re-)implemented — only §6.2 and §6.3 are, since those remain
genuinely unaddressed.** Left below unedited except for this note, so the original
investigation trail stays intact; do not treat §6.1's body text as an accurate
description of current code.

---

## 1. Summary

This was not a crash. The operator's own persistent agent pane (`AgentA`, block
`d34a5dc1-2273-47a7-84f4-47d728c72586`) hit a **stale `--resume` session id**: srv's
locally-cached "current session" pointer for this pane had gone stale, pointing at an
old, superseded top-level session rather than the actual, much larger, currently-live
one. The CLI rejected it immediately, srv detected the failure, searched on-disk for
the real session, found it, and successfully resumed — but the whole detect-fail →
recover → respawn → reach-healthy cycle took **~39 seconds** with **zero UI feedback**,
which is exactly what read as "did it crash?" to the operator.

This is a live, real-world confirmation of the exact bug class documented in
`docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`. The good news:
the recovery fix that shipped since that report (#2693, #2699) worked correctly here —
this is the first confirmed live case where it prevented what would otherwise have been
a silent, total loss of conversation history (exactly like that report's original
incident). The bad news: two of that report's three recommended follow-ups
(§6.1 "make the registry a live pointer" and §6.3 "surface the failure to the user")
are still not implemented, and this incident is proof they're still needed.

## 2. Confirmed timeline (from `agentmuxsrv-v0.55.21.log.2026-08-23`)

| Time (UTC) | Event |
|---|---|
| 18:41:01.792 | `persistent process spawned` — CLI launched with `--resume c1ff18c0-6a6f-4db9-9512-7c63d251edca` |
| 18:41:03.571 | CLI stderr: `No conversation found with session ID: c1ff18c0-...` |
| 18:41:03.571 | srv: `stale --resume session id unreachable under the current config dir — clearing so the next message starts a fresh conversation` |
| 18:41:04.041 | srv: `stale --resume session id caused this exit — retrying fresh, without --resume` (see §4 — this message text is misleading) |
| 18:41:04.055 | `persistent process spawned` (retry) — CLI launched with `--resume 2d1f1558-c9ba-4ef2-a5f2-d76f1fd83bbf` — **the recovery scan found the real, correct, currently-live session** |
| 18:41:43.432 | `agent health transition: Idle → Healthy`, `turn_active flip: active=true` — the pane is fully responsive again |

**Total user-visible gap: ~39.4 seconds** (18:41:04.055 → 18:41:43.432), with no pane
indicator of any kind during that window.

## 3. Root cause chain

1. **Two independent, non-synchronized places track "this agent's current session id"**
   (unchanged since the 2026-08-20 report):
   - The **local, per-channel** `db_agent_instances.session_id` column, kept correct by
     `persist_session_id` (`agentmux-srv/src/backend/blockcontroller/core.rs:139`) on
     every genuine capture during a live turn.
   - The **shared registry** `session_id`, populated **only once** by
     `backfill_session_ids` (`agentmux-srv/src/backend/session_backfill.rs`), which is
     explicitly idempotent — *"skips records that already carry a non-empty id"*. Confirmed
     directly in current code: `persist_session_id` calls
     `sync_instance_session_id` (`core.rs:194`), which writes only to
     `store.instance_update_partial` (the local, per-channel store) — it never touches
     `crate::registry::Registry`. **The shared registry pointer is still write-once, exactly
     as the 2026-08-20 report found; nothing has fixed this since.**

2. **In this specific incident, the stale value was a genuinely real, but superseded, past
   top-level session of the SAME agent** — not a misattributed subagent id (the 2026-08-20
   report's incident, and its own open question #4). Direct evidence: `c1ff18c0-...` appears
   in this channel's own log as the session id of a large subagent-dispatch batch
   (`"backfilling session subagents on pane (re)open" ... "session_id":"c1ff18c0-..."`,
   269 subagent files scanned, all `"parent":"AgentA"`, `"parent_block_id":"d34a5dc1-..."`).
   That means `c1ff18c0` really was this agent's own live session at some earlier point
   (real enough to have dispatched hundreds of subagents under it) — it was simply an
   **old** session, later superseded by a much larger, more recent one
   (`2d1f1558-c9ba-4ef2-a5f2-d76f1fd83bbf`) once conversation compaction/continuation
   moved the live conversation forward. Nothing ever told the registry the pointer had
   moved on. **This resolves the 2026-08-20 report's open item #4 more simply than its own
   speculation**: the "largest session" backfill heuristic doesn't need to have picked
   the wrong session type at all — an ordinary, valid session simply aged out and nothing
   ever refreshed the pointer once conversation continuity moved to a newer one.

3. **The underlying conversation data was never actually inaccessible "across channels."**
   Claude Code's own session transcripts live in a genuinely shared location keyed only on
   the agent's working directory (`~/.agentmux/shared/providers/claude/projects/<slug>/`),
   not per AgentMux channel — confirmed by the recovery scan finding `2d1f1558-...` (a file
   that predates this exact channel, `local-main-b28b7a-cc9756ad`) without issue. The
   "cross-channel" framing in the 2026-08-20 report is about **AgentMux's own bookkeeping**
   going stale, not about the CLI's data being partitioned per channel. This matters for the
   fix: recommendation §6.1 below doesn't need any new cross-channel data-sharing plumbing —
   the data was always reachable; only the pointer needs to stay current.

4. **The existing recovery path (#2693, #2699) worked exactly as designed and is now
   live-confirmed for the first time**, closing the outstanding item noted in
   `docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md` (issue
   #2368: *"close only with a live stale-resume repro... watch the blockfile for the doomed
   attempt's error frame — `held_error_line` should suppress it"*). Confirmed:
   `retry_after_resume_failure` (`persistent.rs:1610`) called
   `find_recovery_session_id` → `session_backfill::find_largest_session_for_working_dir`,
   found the correct, currently-live session, and re-attempted `--resume` with it — which
   succeeded. No stray error frame from the doomed first attempt leaked into the visible
   transcript (the `held_error_line`/`FireRetry` suppression path,
   `persistent.rs:3296-3321`, worked as documented).

## 4. A minor, separate finding: misleading log text

The WARN logged at the moment the retry decision is made
(`persistent.rs:3313`, `"stale --resume session id caused this exit — retrying fresh,
without --resume"`) is logged **before** `retry_after_resume_failure` runs its own
recovery search, and does not reflect what actually happens next. In this incident the
retry was **not** "without --resume" at all — it resumed a different, recovered, real
session id. This is purely a diagnostics/log-clarity issue (it made this exact
investigation slightly more confusing to trace at first), not a functional bug. Worth
a one-line fix: rephrase to something like `"stale --resume session id caused this exit
— retrying (recovery scan may still find a real session to resume)"`.

## 5. Ruled out

- **`[process-tracker] assign_process failed ... Access is denied (os error 5)`** — fires
  twice in this window (once per spawn attempt) but is explicitly non-fatal,
  "opportunistic enrichment, not a liveness signal" per its own code comment
  (`agentmux-srv/src/backend/process_tracker/registry.rs:40-56`). Unrelated to the delay
  or the recovery outcome — confirmed by reading the call site; it only affects the
  Swarm activity panel's process-tree visualization, nothing about spawn/resume logic.
- **The blank `identity_id` WARN** seen in the same window (`instance ... has empty/blank
  identity_id — falling through to the layer-3 gate`) is a real, separate, pre-existing
  oddity (this agent instance appears to be a "legacy continuation row," per
  `agentmux-srv/src/identity/resolver/inject.rs:334-358`'s own comment) but does **not**
  affect which `CLAUDE_CONFIG_DIR`/account gets injected — `resolve_bindings_for_instance`
  keys off `instance.definition_id`, not `identity_id` (same file, line 348-350) — and it
  fires identically on *every* spawn of this pane regardless of outcome, confirmed by it
  also appearing at 18:41:43.430 right before the eventual successful turn. It is not the
  cause of this incident, though it may be worth its own separate follow-up given the
  code itself flags it as unexpected ("Legacy row or UI regression?").

## 6. Fix plan

Ranked by leverage; none of this has been implemented as part of this investigation.

### 6.1 Make the shared registry `session_id` a live pointer (highest leverage, addresses root cause)
`sync_instance_session_id` (`core.rs:194`) already runs on every genuine session capture
and already knows the correct, current id. Extend it (or `persist_session_id`, which
already calls it) to also write through to the shared cross-channel `Registry` record,
replacing whatever stale value is there — not just skip-if-empty. This is a small,
additive change: one more write in an already-executed path, no new polling or background
job needed. Once this lands, the specific failure mode in both this report and the
2026-08-20 report (registry pointing at a superseded session) stops recurring by
construction, and the recovery-scan path (§3.4) becomes a pure safety net for genuinely
unrecoverable cases instead of the primary save.

### 6.2 Surface the recovery attempt to the user (addresses the UX gap this report is actually about)
Right now a stale-resume recovery is **entirely invisible** — no banner, no system
message, nothing distinguishes it from a hang. Given recovery already reliably completes
in the common case (confirmed here), a lightweight, non-alarming pane indicator during
the `ResumeUnreachable → FireRetry → recovery spawn` window (e.g. "Reconnecting…" similar
to the existing "Compacting… Ns" composer-strip treatment already used for
context-compaction visibility, `docs/specs/SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`)
would turn this exact "is it crashed?" moment into a legible, expected state. This is
the single highest-value, lowest-risk fix given how well the underlying recovery already
works — it changes user *perception*, not behavior.

### 6.3 Fix the misleading retry-decision log line (§4) — trivial, low priority
One-line rewording at `persistent.rs:3313` so future investigations don't have to
independently discover that "retrying fresh, without --resume" doesn't mean what it
says.

### 6.4 Not recommended as urgent: investigate the ~39s latency itself
The 18:41:04.055 → 18:41:43.432 gap is plausibly dominated by ordinary CLI cold-start +
loading/replaying a large resumed transcript (this agent's live session is a very long,
multi-hour conversation) rather than any AgentMux-side inefficiency. No evidence was
found of avoidable overhead in this path beyond the two items above. If §6.1 ships,
this latency becomes rare (only for genuinely fresh channels or unrecoverable ids)
rather than something a live, continuously-running pane hits mid-session — likely
sufficient on its own without separately chasing CLI startup performance.

## 7. What this closes out

- Issue #2368's outstanding live-repro requirement
  (`docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md`,
  and the agent's own memory note tracking it) — **done**. The `poison_resume` /
  `find_recovery_session_id` / `held_error_line` suppression machinery from PR #2500/#2501/
  #2693/#2699 is now confirmed working end-to-end on a real, unplanned production-shaped
  incident, not just synthetic tests.
- `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md`'s open item #4
  (how does a wrong id end up in the registry) — narrowed: at least one concrete mechanism
  is an ordinary superseded top-level session, no subagent-id misattribution required.
  Recommendations §6.1 and §6.3 of that report remain the two live action items; this
  document's §6.2 is a new, complementary recommendation specific to the UX gap the
  operator actually noticed.

## 8. Evidence

- `agentmuxsrv-v0.55.21.log.2026-08-23` (this channel:
  `C:\Users\area54\.agentmux\channels\local-main-b28b7a-cc9756ad\versions\0.55.21\logs\`),
  lines covering 18:41:01.784–18:41:43.432 and the 18:38:46 subagent-backfill lines that
  identify `c1ff18c0-...`'s true origin.
- `agentmux-srv/src/backend/blockcontroller/persistent.rs` — `poison_resume` (:327),
  `try_capture_session_id` (:394), `find_recovery_session_id` (:1583),
  `retry_after_resume_failure` (:1610), the `FireRetry` handling and misleading log line
  (:3296-3321).
- `agentmux-srv/src/backend/blockcontroller/core.rs` — `persist_session_id` (:139),
  `sync_instance_session_id` (:194, confirmed local-store-only, no Registry write).
- `agentmux-srv/src/backend/session_backfill.rs` — module doc comment and the
  idempotent skip-if-non-empty backfill logic.
- `agentmux-srv/src/backend/process_tracker/registry.rs` (:40-56) — ruled-out
  `assign_process` warning.
- `agentmux-srv/src/identity/resolver/inject.rs` (:334-358) — ruled-out blank
  `identity_id` warning.
- `docs/status/STATUS_CROSS_CHANNEL_RESUME_STALE_SESSION_ID_2026_08_20.md` — prior
  incident this report confirms, narrows, and extends.
- `docs/specs/SPEC_PERSISTENT_SPAWN_GENERATION_AND_MESSAGE_IDENTITY_2026_08_09.md` —
  issue #2368's original design spec.
