# Status: Cross-Channel `--resume` Silently Starts a Blank Conversation on a Stale Registry `session_id` (2026-08-20)

> **RESOLVED 2026-08-29 (docs-cleanup Phase 3).** §3's actual bug — the
> shared registry's `session_id` being effectively write-once, so it never
> self-corrected — was fixed by **#2755** (`fix(agent): make cross-channel
> resume session_id a live pointer, not write-once`). The blank-conversation
> symptom §1 describes was separately mitigated first by **#2693**
> (`fix(agent): recover the largest on-disk session before falling back to a
> blank conversation on a stale --resume`), and the silent-failure UX gap by
> **#2776** (`fix(agent): surface a Reconnecting… status during stale-resume
> retry`). **#2833** later added knowing at pane open whether a conversation
> will resume or start new.
>
> §6's "recommended fix directions (not implemented here)" are therefore now
> implemented — see `STATUS_STALE_RESUME_LIVE_REPRO_AND_FIX_PLAN_2026_08_23.md`,
> which re-confirmed them with fresh evidence and whose §6.1/§6.2 map onto
> #2755/#2776 respectively.
>
> Everything below is preserved as the original investigation record.

**Status: root cause confirmed, precise code evidence, not yet fixed.**
*(Accurate when written — fixed since; see the banner above.)*
Live-reproduced today: opening agent "AgentX" from a fresh channel
(`0.55.18`) after this same agent had been running continuously in another
channel for days silently dropped all conversation history and started a
brand-new, empty session — with no error surfaced to the user beyond a log
line.

## 1. User-visible symptom

User opened the persistent agent "AgentX" in a newly-launched AgentMux
instance (`0.55.18`) while the same named agent was already running,
uninterrupted, in a different, older instance/channel. The pane initially
displayed the full prior conversation history (loaded for **display** via
the on-disk transcript), but the first message sent in the new instance
got no continuation of that context at all — the underlying process had
actually started a **completely fresh Claude Code session**, with zero
prior turns and no awareness of the conversation the user had just been
looking at. User's own description: *"it compacted and started a new
conversation with no memory."*

This directly undermines the intended behavior of `session_backfill.rs`
(added by #1479 specifically so *"a cross-channel open `--resume`s the
original conversation instead of starting a fresh session"*) and of the
`0.55.18` build's own displayed history, which implicitly promises
continuity it did not deliver.

## 2. Root cause — traced end-to-end via `agentmuxsrv-v0.55.18.log.2026-08-21`

1. On pane (re)open, srv read the registry's `session_id` for AgentX:
   `d019e2e4-7223-4eeb-a2a6-b16b688b9893`.
2. It spawned `claude.cmd --resume d019e2e4-7223-4eeb-a2a6-b16b688b9893`
   (`blockcontroller::persistent`, `"persistent process spawned"` log line,
   `02:02:08.132769Z`).
3. **1.5 seconds later**, srv logged:
   > `WARN: stale --resume session id unreachable under the current config
   > dir — clearing so the next message starts a fresh conversation`
   (`02:02:09.640410Z`)
4. Then:
   > `WARN: stale --resume session id caused this exit — retrying fresh,
   > without --resume` (`02:02:09.688750Z`)
5. srv respawned `claude.cmd` with **no `--resume` flag at all**
   (`02:02:09.750186Z`), which produced a brand-new session id
   (`777d47f6-d4c2-4f67-8d5d-57ab26063b59`, captured at `02:02:10.537867Z`
   — this is the empty 8-line, 14KB transcript the user ended up in).

**Why `d019e2e4-...` was wrong:** it is NOT a stale-but-once-valid id for
AgentX's own top-level conversation. Grepping the same log shows `d019e2e4`
is the session id of a **subagent dispatch batch** (slug
`noble-percolating-ritchie`, 155 subagents reconciled `active -> abandoned`
in the same log, all tagged `"parent":"AgentX"` — i.e. subagents Claude Code
spawned as part of *some* AgentX turn, which share their parent's session id
by Claude Code's own design). Confirmed via
`grep -rl "d019e2e4-..." agentmux-srv/*.log*` that **this exact id has been
sitting in the registry since at least 2026-08-14** — 6 days before this
incident. It was never the live session id at the time of this incident;
AgentX's actual live conversation (in the other, continuously-running
instance) was on a completely different, much larger session
(2.8MB vs. this dead one, unrelated size) that the registry never learned
about.

## 3. Why the registry never self-corrected — the actual bug

Two independent, non-synchronized places track "AgentX's session id":

- **`db_agent_instances.session_id`** (local, per-channel SQLite column) —
  kept live and correct by `persist_session_id`
  (`agentmux-srv/src/backend/blockcontroller/core.rs:139-188`), called on
  every genuine session-id capture during a live turn. This is why the
  *original*, continuously-running instance always has the right answer for
  itself — but this write never reaches any *other* channel's copy of this
  table (each channel has its own local `objects.db`).
- **The shared, cross-channel `Registry` record's `session_id`** — the ONE
  place a genuinely different channel/instance actually reads to decide
  what to `--resume` (confirmed: `grep` for production writes to
  `Registry`'s `session_id` field turns up exactly one call site outside
  test code — `backfill_session_ids` in
  `agentmux-srv/src/backend/session_backfill.rs:69-80`). That function's own
  doc comment says it plainly:

  > *"Populate `session_id` for registry records that lack one, from the
  > agent's largest provider session. **Idempotent (skips records that
  > already carry a non-empty id).**"*

  ```rust
  if rec.data.session_id.as_deref().map_or(false, |s| !s.is_empty()) {
      continue; // already wired
  }
  ```

**This is the bug.** `backfill_session_ids` runs on every srv startup
(wired into `bootstrap.rs:574`) and via a DB migration
(`migrations/m0010_session_ids.rs:22`), but it is explicitly a **one-time
gap-filler**, not a continuous refresher: the very first time it ever ran
for this record, it wrote *some* value (apparently a subagent-batch id,
not the main conversation's own id — how exactly the "largest session"
heuristic picked a subagent's session rather than AgentX's own top-level
one at that point in time is not pinned down by this investigation and
would need its own repro) and has skipped that record on every subsequent
run since, forever, **regardless of whether the live session has since
moved on** (through later respawns, compacts, or simply new top-level
sessions over 6+ days of continued use in the original channel). Nothing
in production code ever re-validates or refreshes an already-non-empty
registry `session_id` against what's actually live.

**Net effect:** the shared registry's `session_id` for any long-running
named agent is only ever as good as the FIRST value it happened to get,
and can go stale (or, per this incident, may never have been fully correct
to begin with) with no self-healing mechanism and no user-visible warning
until the next genuinely fresh channel tries to use it — which, for an
agent someone keeps running continuously in one place, may not happen for
days or weeks, by which point the failure is confusing and detached from
any recent action.

## 4. What this is NOT

- **Not caused by having two AgentMux instances open at once.** The stale
  value predates this incident by 6 days (confirmed present in
  `agentmuxsrv-v0.55.8.log.2026-08-14`); nothing about concurrent access is
  implicated by the log evidence.
- **Not a total memory loss.** File-based Claude Code memory (a separate
  mechanism, keyed on the agent's fixed working directory, not on
  `session_id`) most likely still loaded normally in the fresh session —
  its first assistant turn shows 368K `cache_creation_input_tokens`,
  consistent with the full system-prompt/CLAUDE.md/memory context being
  present. Only turn-by-turn conversation history was lost.
- **Not silent on the backend** — the two WARN log lines exist and are
  fairly clear — but nothing surfaces this to the user in the UI beyond
  the pane simply "forgetting" everything after one message. A user without
  log access has no way to know what happened or why.

## 5. Impact

Any named, persistent agent kept running continuously in one AgentMux
instance/channel for an extended period is at risk: the next time it's
opened from a genuinely different channel (a fresh portable build, a
different `task dev` branch, etc.), there is no guarantee the resume will
succeed, and no user-facing signal when it fails — just a quietly empty
conversation. This is likely to recur for any long-lived agent, not just
this one instance.

## 6. Recommended fix directions (not implemented here)

1. **Make the shared registry `session_id` a live pointer, not a
   write-once value.** The local `persist_session_id` write path
   (`core.rs:139`) already fires on every genuine capture — it should also
   propagate to the shared cross-channel registry record (or that record
   should stop being trusted as authoritative and instead be recomputed
   fresh, e.g. re-running the "largest session file" scan, whenever a
   `--resume` attempt is about to be made rather than relying on a value
   that might be arbitrarily old).
2. **At minimum, don't silently fall back to a blank conversation on a
   confirmed-stale id.** When `--resume <sid>` is confirmed unreachable
   (the exact WARN already fired in §2), re-run `largest_session_id`
   against the current on-disk transcripts before giving up and starting
   empty — the correct, much larger session was sitting right there on
   disk the whole time; the fallback logic just never looked for it.
3. **Surface the failure to the user.** A pane that silently starts a
   blank conversation after promising to resume one is a confusing, hard
   to diagnose experience. Even a lightweight system-message ("Could not
   resume the previous session — starting fresh") would have made this
   immediately legible instead of requiring a log dig to explain.
4. Separately worth investigating (not chased down here): exactly how
   `d019e2e4-...` — a *subagent's* session id — ended up as the value
   `backfill_session_ids` wrote into the registry in the first place,
   rather than AgentX's own top-level session. If the "largest session
   file" heuristic can pick a subagent transcript over the parent's own,
   that's a second, related bug worth its own repro.

## 7. Reproduction data

- Affected agent: "AgentX", block/pane `81b9155c-70e8-4243-b5b5-00afe1b85fa9`
- Stale registry value: `d019e2e4-7223-4eeb-a2a6-b16b688b9893` (present
  since at least 2026-08-14, a subagent-batch session, slug
  `noble-percolating-ritchie`)
- The genuinely live, correct session at time of incident:
  `972a6a4f-6a7e-439d-aebc-358452a13d78` (2.8MB, actively being written by
  the original, still-running instance) — never consulted
- Resulting blank session: `777d47f6-d4c2-4f67-8d5d-57ab26063b59`
- Log source: `C:\Users\asafe\.agentmux\logs\agentmuxsrv-v0.55.18.log.2026-08-21`,
  `02:02:08.132769Z` through `02:02:10.537867Z`
