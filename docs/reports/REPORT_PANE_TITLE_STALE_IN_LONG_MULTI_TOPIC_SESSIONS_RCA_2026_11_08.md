# RCA: an agent pane's title stayed on an old topic for four hours

**Date:** 2026-11-08. **Build:** 0.60.21; code read at origin/main `51458817`.
**Fix:** this change (task #30049).
**Status:** implemented — the fix described below shipped with this report (task #30049).

## What happened

A leader agent's pane showed the session title "Compare AWS Bedrock and OpenRouter pricing", stored at 14:43 UTC. For the next four hours and roughly thirty human messages, the session moved through many unrelated topics. The title did not change until 18:52, when a message that was itself a bug report about the title produced a new one.

The srv log (`ambient outcome` lines, purpose `activity_summary`) shows six title calls in those four hours:

| UTC | Outcome |
|---|---|
| 14:43 | accepted: "Compare AWS Bedrock and OpenRouter pricing" |
| 17:28 | skipped (the model replied SKIP) |
| 17:57 | skipped |
| 18:19 | skipped |
| 18:31 | skipped |
| 18:49 | timeout (30 s) |
| 18:52 | accepted: a new title |

Four answered SKIP, one timed out, and none replaced the old title.

## Why it didn't update: four causes that add up

1. **The title is designed for a session with one goal.** The prompt (`crates/srv/src/ambient/prompt.rs`, `build_session_title_prompt`) asks for a pull-request-like title for "the OVERALL GOAL of the session", and tells the model "If the current title still accurately describes the overall goal, reply SKIP" (spec `SPEC_AMBIENT_PANE_TITLE_OVERALL_GOAL_TRACKING_2026_08_17.md`). That is right for a worker on one task and wrong for a leader or front-door session that takes a stream of unrelated requests: it has no single overall goal, so "keep the current one" wins by default.
2. **The model saw only the newest message, not what the agent was doing.** `session:activity_summary` (`crates/srv/src/server/app_api/session.rs`) read the recent activity only when there was no user message. With a message, the model got the old title plus that one message. Most messages to a leader are short status questions ("whats the status on that?"), the prompt says to SKIP those, and with no activity to read the model had no way to know the work had moved on.
3. **Most messages were never evaluated.** By the schedule chosen in the spec (decision 5, `frontend/app/store/title-schedule.ts`), once a title exists it is re-evaluated only on human turns 2, 5 and 8, then every third. About two messages in three made no title call, including ones that clearly started a new topic.
4. **A title never aged.** Nothing compared the title's age, or the number of turns since it was written, with the work since. A compaction or a long gap did not reset it either: the session id stays the same, so the old title carried on.

A fifth, smaller factor: the title call's 30 s limit was hit several times for this pane (`activity_summary` once, `next_prompt_suggestion` often). A timed-out title call is a lost evaluation, and the old title stands.

The agent's own status line (`Presence status`) was also stale in this case, but the pane never showed it: the working row and a tab's tooltip read only the generated title.

## The fix

The schedule's stability for one-goal sessions is kept; a title is just no longer allowed to outlive its goal.

1. **The model sees the session.** `session:activity_summary` takes the pane's recent conversation (`activity`, built from the document by `ambient-activity.ts`, so it works for every provider) and reads it with the user's message, not only when there is no message. It falls back to the output file's tail, as before.
2. **A title ages.** The backend stamps the title with when it was stored or confirmed (`term:ambient_summary_at`) and the human turn (`term:ambient_summary_turn`). Past 9 human messages or 2 hours since, or with no stamp (an older build's title, or one restored from a finished session), the next message sends `refresh: true`: the model is shown no current title, so it writes the create-form prompt and "still the same goal, SKIP" is not the easy answer. A result that names a different goal replaces the title; a rewording confirms it (the stamp is renewed, the text kept), so it is not asked about again at every message. A SKIP leaves it aged and it is tried at the next message.
3. **The work moving on resets the age.** A compaction (`compaction_started`) and a catch-up turn mark the title stale (stamp `0`), so the next human message judges it afresh.
4. **The agent's own word wins while it is fresh.** `readPaneSummary` (`swarm-line.ts`) shows the agent's own status line in the working row and tab tooltip when it is under an hour old and newer than the title; a task title written at a claim, or an old line, does not count.
5. **Timeouts.** `activity_summary` has the 45 s background limit instead of 30 s.

## Tests that pin it

- A title more than 9 messages or 2 hours old, one with no stamp, and one marked stale are each sent with `refresh` (`useAgentActivitySummary.test.ts`, `title-schedule.test.ts`).
- A status question in the middle of new work is sent with that work's activity (hook test), and the prompt carries both (`session_tests.rs`).
- A refresh shows the model no current title (`session_tests.rs`).
- A stored title is stamped; a rewording on a refresh renews the stamp without replacing the title; a new goal, an abstention and a superseded request do not (`ambient/title.rs`).
- A fresh status line wins over an older title and loses to a newer one; an hour-old line and a task title do not count (`swarm-line.test.ts`).

## Not done

A refresh whose answer is SKIP is retried at the next message, with no back-off. With the activity now in the prompt, SKIP on a refresh needs a session that shows no work at all, so this is expected to be rare. If it is not, count the refresh attempts per title and wait two messages between them.
