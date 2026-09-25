# RETRO: Finding another agent's current conversation took ~25 tool calls and multiple false negatives

**Status:** retro
**Date:** 2026-09-20
**Author:** AgentX (this session)
**Trigger:** User asked to review a recent conversation with Lark about a crash/restart and a pane-recovery RCA. Locating it took most of a long session and required reading the live SQLite store directly — there is no supported way to do this today.

## Impact

A basic, common task — "what did agent X just say" — required manual filesystem/DB
archaeology instead of a tool call. Along the way I produced at least one incorrect
conclusion stated with too much confidence ("Camper has zero conversations, ever"),
which had to be retracted once the same method failed on my *own*, definitely-existing,
currently-active session.

## Timeline of what failed, and why

1. **`GetAgentTranscript(agent="Lark")`** → returned `lines: []`. This reads a live,
   in-memory transcript buffer tied to the reactive/registration layer. It does not
   reflect the actual Claude session content/history — it came back empty despite a
   real 11,131-line session being active at the time.
2. **`SearchHistory(query="Lark")`** → `0 hits, 0 sessions_scanned`, then later
   `sessions_scanned: 0` again even after retry. This tool is explicitly scoped to
   **the calling agent's own history only** (per its own description) — it structurally
   cannot find another agent's conversation. Using it for this purpose was a dead end
   by design, not a bug, but that scope isn't obvious from the task at hand.
3. **Filesystem search under `channels/<channel-id>/identities/<identity-id>/claude/projects/`**
   — found exactly one Lark project folder, containing a session from **32 days ago**.
   Concluded (wrongly) that this was "the last conversation with content."
4. **Broadened to all 26 channels** — same single stale result. Concluded (wrongly,
   again) that Camper had *zero* sessions anywhere, ever.
5. **Sanity check: searched for my own live session** (this very conversation) under
   my own channel/identity — **found nothing**. The single most-recently-modified file
   across all 4,500+ `.jsonl` files in `channels/` was still that same 32-day-old file.
   This proved step 3/4's method was invalid, not that the data didn't exist — my own
   currently-running session isn't there either, so absence there means nothing.
6. **Found the real live store**: `channels/<channel-id>/versions/<build>/data/db/objects.db`
   (SQLite, actively being written — `.db-wal`/`.db-shm` files with second-old mtimes).
   Table `db_agents` has a `session_id` column with Lark's *real* current session id.
   Table `db_block` has the live pane metadata (`term:ambient_summary`,
   `term:next_prompt_suggestion`, `session:line_count`, etc.) matching the block_id from
   the `DiscoverAgents`/`FleetList` APIs.
7. **Used that real session id (`738ffcfb-8c91-45d9-8481-eaa5cac672c3`) to find the file
   directly** — and it was at `C:\Users\asafe\.agentmux\shared\identities\<identity-id>\claude\projects\...\738ffcfb-....jsonl`.
   Note the root: **`shared\identities\`, not `channels\<channel-id>\identities\`.**
   Same identity UUID, two different roots on disk, only one of which had current content.
   Nothing in steps 1-5 pointed at `shared/` at all.

## Root cause

- Two parallel identity trees exist on disk for the same identity UUID
  (`channels/<channel-id>/identities/<id>/...` and `shared/identities/<id>/...`), and only
  `shared/` had the live, current content for this identity. There is no indication from
  any API which root holds the authoritative copy.
- The two MCP tools that sound like they'd solve this (`GetAgentTranscript`,
  `SearchHistory`) don't cover this case: one reads a different, apparently-stale buffer;
  the other is deliberately single-agent-scoped.
- The actual source of truth for "what is agent X's current session id and where's its
  live metadata" is the SQLite `db_agents`/`db_block` tables — accessible only by hand-
  rolling a read-only sqlite3 connection, since there's no `sqlite3` CLI on PATH and no
  MCP tool exposes this.

**2026-09-24 update:** the "two parallel identity trees" root cause above is
now out of date. `ensure_history_link`
(`SPEC_AGENT_IDENTITY_HISTORY_PERSISTENCE_PROTOCOL_2026_08_16.md`, PR #2605)
turns the `channels/.../identities/<id>/...` copy into a junction/symlink
into the always-global `shared/identities/<id>/...` tree at `auth.start`/
spawn — `shared/` is authoritative by design, not an unexplained duplicate.
The one real gap this left (a channel retired before that linking logic
ever ran for it, so its directory sat real but unlinked) had its own
one-time migration, merged as PR #3460 (`m0034_identity_history_backfill.rs`,
2026-09-22). The `GetAgentTranscript`/`SearchHistory` gaps below are
unaffected by this and remain open.

## Recommendation

Build one supported lookup: given an agent name, resolve its current `session_id` and
the on-disk path to that session's transcript, across *any* agent on the host — not just
the caller's own history. Ideally exposed as a tool (e.g. an `agent` parameter on
`SearchHistory`, or a new `GetAgentConversation`) that:

1. Resolves the agent's current live session id from the authoritative store (whatever
   that turns out to be — `objects.db`'s `db_agents` today).
2. Locates and reads that session's transcript regardless of which identity root
   (`channels/.../identities/` vs `shared/identities/`) it physically lives under.
3. Works the same for a caller's own history and any other agent's, since the need to
   check another agent's recent work (crash recovery, review, handoff) is not rare.

**2026-09-24 update:** step 2 is simpler than originally scoped — read
`shared/identities/` directly (see the root-cause update above), falling
back to the channel-scoped copy only defensively.
`SPEC_AGENT_CLAUDE_IDENTITY_LOOKUP_2026_09_24.md` §3.2 picks this
recommendation up as its P3.

## What Lark actually said (for the record)

The user's recollection was "Lark told me to restart the instance and I'd recover the
agents." The real transcript (session `738ffcfb-...`, message at 2026-09-20T20:39:14Z)
says something more specific and more hedged:

> "So: just reopen. No pre-cleanup step. What to expect: Your instance ... goes down and
> comes back; all 10 agents respawn and resume by session id, in their original working
> directories. ... One caveat I'll restate plainly since you're about to act on it: my
> prediction that the blank panes come back is **reasoned, not verified**. If they come
> back blank after a cold start, that actually falsifies my timing hypothesis and would
> be genuinely useful information — tell me and I'll revise the RCA rather than defend it."

This is consistent with PR #3457 (see prior analysis): Lark expected the *processes* to
come back (they did — that's normal respawn-by-session-id), explicitly did **not**
promise the blank-pane rendering bug would be fixed, and framed the restart partly as an
experiment to test the RCA's timing hypothesis. The blank panes persisting after restart
is not a surprise or a broken promise — per Lark's own message, it's closer to expected,
and it's information Lark asked to be told about.
