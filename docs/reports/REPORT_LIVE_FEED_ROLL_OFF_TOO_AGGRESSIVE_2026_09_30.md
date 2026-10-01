# REPORT: The agent pane moves too much conversation into History

**Date:** 2026-09-30
**Status:** analysis — root cause found; fixes proposed, nothing changed yet.
**Verified against:** `agentmux` `main` @ `9fb8635f1`. Measurements are from
the local transcript store (`~/.agentmux/shared/agents/transcripts/filestore.db`,
the `agent:<defId>:current` zones, 21 agents), read-only.
**Related:**
- `SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` §6.9 (the
  live feed);
- `SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md`
  (the session-scope clamp);
- `SPEC_AGENT_OPEN_LATENCY_2026_09_27.md` and #3964 (restoring only the
  live feed's turns).

## 0. The ask

From the owner, 2026-09-30:

> "We did work that moves conversation from the agent pane to History. It
> is too aggressive. How does it decide what to cut out? Sometimes it will
> cut so much there will only be a couple messages before I see 'Open
> history'."

## 1. Summary

1. **Three mechanisms remove conversation from the pane** (§2):
   - **roll-off**, which keeps 6 finished turns;
   - **restore**, which loads only the last 7 turns when a pane opens;
   - **the session-scope clamp**, which hides everything before a "fresh
     session" boundary.
2. **Root cause: restore and roll-off disagree on what a turn is.**
   - The pane starts a turn at each **user message**. A **jekt** (a message
     from another agent) renders as its own `jekt_message` row and doesn't
     start one.
   - The backend restore (#3964) counts **every** `user` record with string
     content as a turn start, **jekts included**.
   - So when a pane opens, "the last 7 turns" can be 7 jekts. Measured:
     - **Manoz** reopens with **0** of the user's own messages in the
       restored window;
     - an agent that rarely gets jekts reopens with 7 (§3.1).

   That is exactly "a couple of messages, then Open history".
3. **The session-scope clamp is a second cause.** After any "fresh"
   session, everything before it is hidden; 7 of 53 fresh boundaries
   measured left 2 or fewer typed messages visible (§3.2).
4. **The 2 MB byte cap and the 6-turn default rarely bind on their own.**
   One weak spot: a tool that streams more than 4,000 log chunks is counted
   as a full 2 MB, which on its own drops the pane to a single finished turn
   (§3.3).
5. **Proposed fixes, in order** (§4):
   1. make the restore count turns exactly as the pane does, with a parity
      test;
   2. don't hide earlier conversation at a fresh boundary;
   3. size tool logs by their real bytes;
   4. raise the default and put it in Settings.

## 2. How it decides what to cut

### 2.1 Roll-off: the live feed (frontend)

`frontend/app/view/agent/live-feed.ts`, driven by
`hooks/useLiveFeedRollOff.ts`.

- **A turn** starts at each `user_message` node (`splitTurns`, :61).
  Anything before the first one is a leading turn.
- **Kept** (`planRollOff`, :131):
  - the turn in flight;
  - the newest **6** finished turns (`LIVE_FEED_DEFAULT_TURNS`, :21;
    setting `agent:livefeedturns`, which nobody here has overridden);
  - turns with a node on screen;
  - turns still in progress;
  - turns holding an in-pane shell run (`blocksRollOff`);
  - turns with a pinned node.
- **The byte cap:** finished turns are kept only up to **2 MB** in total
  (`LIVE_FEED_MAX_FINISHED_BYTES`, :23), with at least one always kept
  (:154).
- **When it runs:** while the reader follows the bottom, every finished
  turn outside that set rolls off. While they're scrolled up, only turns
  wholly below what they're reading go.
- **What the user sees:** "Open history" at the top, and gap rows where
  turns rolled off around one that had to stay
  (`inject-history-link.ts`).

### 2.2 Restore: what a pane loads when it opens (backend + frontend)

- **The request:** for Claude panes, `agent-view.tsx:413` asks for
  `tail_turns = 6 + 1`.
- **The trim:** `crates/srv/src/server/app_api/blockfile.rs` keeps the
  lines from the 7th-last turn start (`last_turns_start`, :428;
  `trim_to_last_turns`, :446).
- **A turn start** (`is_claude_turn_start`, :397) is any `user` record whose
  content is a non-empty string, or an image plus text with no
  `tool_result`.
- **Its doc comment claims** "the same rule as the frontend's
  `ClaudeTranslator.handleUserMessage` producing a `user_message`". **That
  isn't true for jekts.** Jekts are `user` records with string content
  (`[JEKT:…]`), and the stream parser turns them into `jekt_message` nodes
  (`stream-parser.ts:446`), not `user_message`. The same goes for Claude
  Code's compaction summary, now a context-delivery card.
- **Then roll-off runs** (`onHistoryReady` → `scheduleRollOff`) on whatever
  the restore returned.

### 2.3 The session-scope clamp (frontend reducer)

- **What it does:** `frontend/app/store/agent-document/reducer.ts`
  `clampToSessionScope` (:44) drops every node before the **last**
  `session_outcome` node with `outcome: "fresh"`.
- **When:** on history load (:363), on restore (:459) and on stream flush
  (:589).
- **What creates a fresh boundary:** any session AgentMux couldn't resume.
  Examples are an account switch, a rejected `--resume`, or a missing
  session file.
- **What the user sees:** the pane shows "Session continued · reconstructed
  context" and "Open history", and nothing before it.

## 3. Measurements

### 3.1 Restore keeps jekts instead of the user's messages

For each agent with at least 10 turn starts, this shows how many of the
user's own messages are in the last 7 backend "turns", i.e. what a reopened
pane shows before "Open history":

| Agent | Backend turn starts | User's messages | User's messages in the restored window |
|---|---|---|---|
| Manoz (`43f2b0c6`) | 1,483 | 660 | **0** |
| AgentA (`d76da857`) | 2,268 | 1,521 | 6 |
| 12 other agents | 10–243 | 10–187 | 7 |

The difference is jekts. Manoz is driven mostly by jekts (ReAgent review
notices, other agents' messages), so its last 7 "turns" are all jekts. Of
the string-content `user` records across all agents, 3,033 are messages a
person typed and **1,707 are jekts**.

### 3.2 Fresh boundaries

- **How common:** 53 fresh boundaries, across 10 of the 21 agents.
- **How much stays visible:** the median gap between two fresh boundaries
  is 20 typed messages, but **7 of the 53 gaps were 2 or fewer**. After one
  of those, the pane shows almost nothing before "Open history", whatever
  roll-off would have kept.

### 3.3 The byte cap and turn count

These are measured on transcript content between typed messages, which
approximates the pane's own `nodeBytes`.

- **Turn size:** median 26 KB, p90 314 KB, p99 1.3 MB, max 5.5 MB. Only
  0.4% of turns exceed 2 MB on their own.
- **Simulated roll-off:** with 6 turns and the 2 MB cap, all 6 finished
  turns are kept **93.8%** of the time; 1 to 5 are kept about 1.2% each.
- **The exception: streaming tool logs.** `nodeBytes`
  (`virtualization/streaming-buffer.ts`) counts a tool whose live log has
  more than `MAX_LOG_CHUNKS_SCANNED` (4,000, :157) chunks as
  `NODE_BYTES_CAP` = **2 MB** (:148, :229) without walking it. One long
  build or test run in the newest finished turn therefore fills the whole
  cap, and the pane keeps that single turn. The transcript measurement
  can't see this (live logs aren't in it), so this is a code finding, not a
  measured rate.

## 4. Proposed fixes

| # | Fix | Effect | Size |
|---|---|---|---|
| F1 | **One turn rule.** `is_claude_turn_start` stops counting jekts (the `[JEKT:` marker that `wrap_jekt_message` produces), Claude Code's compaction summary (`isSynthetic`), and AgentMux's own hidden or continuation turns. Add a parity test: the same fixture lines go through the backend rule and through `ClaudeTranslator` + `ClaudeCodeStreamParser`, and both must agree on every turn start. | Fixes §3.1: a reopened pane gets the user's last 7 turns | small |
| F2 | **Don't hide the past at a fresh boundary.** Keep the "Session continued" divider, but stop dropping what's before it. Roll-off then decides as for any other turn. | Fixes §3.2. The divider already marks the context break, and the model has a continuation packet | small–medium; the clamp's spec needs a decision (§5) |
| F3 | **Size streaming logs by their bytes.** Past 4,000 chunks, estimate from the chunks seen so far (or a running byte total kept as chunks arrive) instead of charging the full 2 MB. | One long build no longer shrinks the pane to one turn | small |
| F4 | **A more generous default, in Settings.** Raise `LIVE_FEED_DEFAULT_TURNS` from 6 to about 12, and show `agent:livefeedturns` in Settings ("Turns kept in the pane"). | Fewer "Open history" surprises in normal use; the memory win stays | trivial |
| F5 | **Optional: a floor by messages, not just turns.** Always keep the last N rows the user can read (for example 30), whatever the turn count. | Guards against any future turn-definition drift | small |

F1 is the bug. F2–F5 are about how aggressive it is by design.

## 5. Decisions for the owner

1. **Fresh boundaries (F2):** keep earlier conversation visible below a
   divider (recommended), or keep hiding it as the session-scoped
   scrollback spec decided?
2. **Should jekts count as turns** in the pane as well? Today a jekt-driven
   agent's "turn" can span hours of work, because only the user's own
   messages split turns. F1 makes the backend match the pane. The other
   option is to make both count jekts and raise K.
3. **The default K (F4):** 12, or another number?
