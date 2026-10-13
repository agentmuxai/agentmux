# RCA: "Agent encountered an error" and two "Given to the agent" cards after an account switch

**Date:** 2026-10-11 · **Author:** agent2
**Status:** implemented: all three fixes in §4, in the same change as this report.
**Seen in:** a local 0.59.20 build (`main` @ `0526a5c0f`), Claude Code 2.1.288, Windows. Agent2's pane, right after the operator switched the agent from one Claude account to another.
**Evidence:** the instance's srv log (19:08:45 to 19:09:17 UTC), the pane's own transcript (the `output` file in the shared transcript store), and the new account's Claude project dir.

## 1. What the user saw

1. A red **"Agent encountered an error"** in the pane, before any message was sent.
2. The **"Given to the agent · new session"** card twice.

The conversation itself was not lost: the next message started a fresh session carrying AgentMux's record of the conversation, as designed for an account switch.

## 2. Timeline (UTC)

| Time | Event | Effect in the pane |
|---|---|---|
| 19:08:46 | The pane opens. Spawn 1 resumes the old session, `be9fdda5`, under the **old** account (`--resume`). | none |
| 19:08:49 | The account switch rebinds the agent and force-restarts it. The resume gate refuses `be9fdda5` ("another identity's"), so spawn 2 starts **fresh** under the new account and holds the continuation record. Claude's SessionStart hooks run in session `4973c054`. AgentMux captures that id from the hook frames and records it as the conversation's head. | **Card 1:** "Given to the agent", memory and startup files delivered to `4973c054` |
| 19:09:11 | A runtime change (the model, Sonnet → Opus) force-restarts the agent (`runtime-apply.ts`, `ControllerResync forcerestart`). No message has been sent yet. | none |
| 19:09:12 | Spawn 3 runs `--resume 4973c054`. | none |
| 19:09:13 | Claude Code: `No conversation found with session ID: 4973c054…`, then a `result` frame with `is_error: true` and `errors: ["No conversation found…"]`, and exit 1. With nothing queued, the retry settles as an empty batch (`settle_empty_resume_retry`): it records a `Fresh` outcome and flushes the held error line into the pane. | **Red "Agent encountered an error"** |
| 19:09:15 | The user sends their message. Spawn 4 starts fresh with the continuation record, in session `e935606c`. | **Card 2:** "Given to the agent", delivered again |

`4973c054.jsonl` does not exist in the new account's project dir; `e935606c.jsonl` does.

## 3. Root cause

**AgentMux treats a Claude session as resumable as soon as it learns its id, but Claude Code only saves a session once its first turn runs.**

- In stream-json mode, Claude Code puts `session_id` on every frame, including the SessionStart `hook_started` and `hook_response` frames it emits at startup, before any user message. The stdout reader captures the id from the first such frame, persists it to the block, and records it as the segment's session (`persistent/stdout_reader.rs:392-432`). From then on, it is the chain head.
- Claude Code writes `<session>.jsonl` only when the first user message is processed. Session `4973c054` never received one: it lived 22 seconds before the model change killed it.
- On the restart, the resume gate (`continuity_segments.rs:335`, `resume_gate`) sees a candidate that is the head, finds no file for it under the spawn's config dir, and has nothing to relocate. It then deliberately **allows** the resume (lines 360-367: "As the candidate, it goes on to `--resume` as before the gate: the CLI's rejection and the retry path already handle it").
- The retry path does recover: it clears the id, discloses `Fresh`, and the next message starts fresh with the record. But for an empty batch it also flushes the CLI's error frame into the pane on purpose (`resume_retry.rs:733-737`), so the user sees a failure for what was a routine restart.

**Why the error text is generic.** The frame's reason is only in `errors[]`. The pane's translator reads `result`, then `error.message`, then `error`, then falls back to "Agent encountered an error" ([`frontend/app/view/agent/providers/claude-translator.ts:278-284`](../../frontend/app/view/agent/providers/claude-translator.ts)). It never reads `errors`. Showing the real text would have read "No conversation found with session ID …": accurate, but still alarming for a restart nobody needs to know about.

**Why the card appeared twice.** Each fresh session gets the startup files and memory, and each delivery adds a card (`memory delivery: complete, notice sent` at 19:08:50 and 19:09:17). The first went to session `4973c054`, which never ran a turn and was then thrown away. Both cards are accurate about what was delivered, but the first describes a session the conversation never used.

**What the account switch has to do with it.** Nothing directly. The switch only made spawn 2 a fresh session. The same sequence should occur whenever a fresh session is restarted before its first message, for example a model, effort or permission-mode change in a new agent or right after a `/clear`. That generalisation is inferred from the code and not yet reproduced.

## 4. Fix options

1. **Don't resume a session Claude never saved** (the root cause). In `resume_gate`, refuse to resume when the candidate is the head, no file exists under the spawn's own config dir, and the head was recorded under that same config dir: it was never written, so there's nothing to resume. The refusal path already starts fresh and carries the record, quietly, as spawn 2 did. The CLI-rejects-and-retry path stays for the cases it exists for: a file deleted or moved from under the agent, or an unknown head. Alternatively, record the segment's session only once a turn has run (a `system/init` or `result` frame), not on hook frames. That is more invasive, because the subagent watcher and memory delivery key on the captured id early.
2. **Show the CLI's own reason.** The translator should also read `errors[0]` before falling back to the generic text. Cheap, and right regardless of fix 1.
3. **One card for a session that never ran.** When a new delivery card follows an earlier one with no turn between them, the pane folds the earlier card into the later one (or hides it). Alternatively, the backend skips the notice when the previous delivery's session never ran a turn. Folding in the pane is simpler and keeps the record.

Fix 1 removes the red error in this scenario; fix 2 makes any remaining stale-resume error say what happened; fix 3 removes the duplicate card. They are independent.

**As built:**
1. `resume_gate` refuses a head recorded under the spawn's own config dir whose file isn't there (`continuity_segments.rs`). The spawn starts fresh and carries the record, with no error.
2. The pane's translator, srv's Claude translator and the failure classifier read `errors[0]` before their generic fallback (`claude-translator.ts`, `translator/claude.rs`, `failure.rs` `first_cli_error`).
3. A session-start card takes the place and id of the card before it when the agent did no work in between; the user's own message between them doesn't count (`memory-injected.ts` `supersededCard`, used by the live stream and by history replay alike).

**Verified live** in an isolated dev build: with a test agent's session file missing from its own config dir, a model change logged "resume gate: … starting fresh with the record", spawned without `--resume`, and the pane showed no error.

**Not covered:** when memory reaches a fresh session through AgentMux's fallback (a hidden message, as in that dev build, whose srv logged no hook delivery) rather than the hook, its "session continued" cards still repeat across a restart. They take a different path (`memory-reinjection.ts`); fold 3 covers the hook's "new session" card only.

## 5. Reproduction (to confirm before fixing)

In an isolated `task dev` build:
1. Open a new Claude agent. Don't send a message.
2. Change its model from the model menu.
3. Expected today: a red "Agent encountered an error"; the srv log shows `No conversation found with session ID`.

Then the same, after switching the agent's account instead of in a new agent.
