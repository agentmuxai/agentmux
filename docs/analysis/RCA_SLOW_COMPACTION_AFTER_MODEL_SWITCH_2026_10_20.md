# RCA: a manual compaction took 164 s for 438k tokens, after a model switch and a 5.7-hour idle

**Date:** 2026-10-20
**Author:** Lazo@Area54
**Asked by the operator:** "any idea what happened to Manoz's compaction? I changed the model from sonnet to opus, then immediately clicked compact. Is that why it is stuck?" and then "nm, it came back, why did it take so long? 3x the normal time". "RCA to file."
**Status:** analysis. The mechanism (a cold prompt cache) is identified with high confidence from timing; the split between the two contributing factors cannot be measured from the data (§6). The follow-ups in §7 are not implemented.

## 1. Summary

- **Not stuck, not an error.** The compaction ran from 17:30:55Z to 17:33:39Z and completed normally. No retry, error or restart happened during it. The pane showed "compacting" for 2 min 44 s with no other progress.
- **It was slow because it ran on a cold prompt cache.** The agent's last turn before the compaction ended at 11:51:07Z, 5 h 40 min earlier. Prompt-cache entries from that session last one hour (`ephemeral_1h` in its usage records), so the 438k-token conversation had to be read in full, uncached, by the compaction request. Switching from Sonnet 5.5 to Opus 5.5 right before would have forced the same thing, since a prompt cache belongs to one model.
- **Every other compaction on these models in the last four weeks ran warm** (the previous turn minutes earlier, same model) and took 30 to 81 seconds, even at 0.97 million tokens. This one is the only 5.5-era compaction that had both a model change and a long idle, and it took 164 s for less than half the tokens.
- **Was the model switch the cause?** Part of it, together with the idle time. Either one alone would have made the cache cold. We can't tell which cost more, because they happened together (§6). The switch itself did nothing wrong.

## 2. Timeline (UTC, 2026-10-20; from the agent's session transcript and the bashwrap debug log)

| Time | Event | Source |
|---|---|---|
| 11:51:07 | Last turn on `claude-sonnet-5-5` ends (context about 436k tokens, mostly cached) | session transcript |
| 11:51 to 17:29 | Idle: 5 h 38 min with no model request | transcript has no records |
| 17:29:33 | The agent's CLI session is resumed: eight `SessionStart:resume` hooks run within 0.6 s. This is the restart for the model change | bashwrap log, nine hook lines |
| 17:30:55 | `/compact` is sent (82 s after the restart) | transcript |
| 17:30:56.1 | PreCompact hook fires and publishes `compaction_started` (1.2 s later: no delay in AgentMux's part) | bashwrap log |
| 17:33:39 | `compact_boundary` recorded: `trigger: manual`, `preTokens: 437,833`, `durationMs: 163,974` (the whole 2 min 44 s) | transcript |
| 17:33:40 | The summary is delivered back to the agent's memory parts; `PreCompact ... completed successfully` | bashwrap log |
| 17:33:48 | First turn after the compaction, on `claude-opus-5-5` | transcript |

`durationMs` of 163,974 matches the gap from `/compact` to the boundary to within a second, so the time was spent inside the summarising model request, not before it or after it.

## 3. What "normal" is

Compactions found in the agents' session transcripts since the 5.5 models started (22 of them: the 21 below, and a 51k-token test session that took 9 s), with the idle gap before each:

| | Count | preTokens | Duration | Idle before | Model change |
|---|---|---|---|---|---|
| Auto compactions, Opus 5.5 | 15 | 0.97M | 39 to 81 s (median 68 s) | about 1 min | none |
| Manual compactions, Opus 5.5 | 3 | 0.87 to 0.93M | 54 to 75 s | 3 to 6 min | none |
| Auto compactions, Sonnet 5.5 | 2 | 1.0 to 1.05M | 30 to 34 s | about 1 min | none |
| **This one** | 1 | **0.44M** | **164 s** | **343 min** | **Sonnet 5.5 to Opus 5.5** |

Older models were slower (Sonnet 5 and Opus 5: 60 to 270 s for 0.93 to 0.97M), but this session was on the 5.5 models throughout.

The ratio is the problem: about 0.07 s per thousand tokens warm (68 s for 0.97M), against 0.37 s per thousand here, five times more per token. Against the 54 to 75 s that a manual compaction normally takes on these models, this one took 2.2 to 3 times as long (the operator's "3x" matches), and it had less than half the context to summarise. Part of any compaction is a fixed cost (writing the summary), so the per-token figures overstate the gap; the absolute times are the fair comparison.

## 4. Cause

A compaction asks a model to read the whole conversation and write a summary. The request's input is the full conversation, 437,833 tokens here.

- **Warm:** the conversation prefix was cached by the previous turn, so most of those tokens are served from the cache (a cache read, billed and processed at a fraction of the cost and latency). Only the new instruction and the summary output are real work. This is how all the 30 to 81 s compactions ran.
- **Cold:** if the cache has expired or belongs to another model, the model has to process all 437,833 tokens from scratch (and write them into a new cache). Time to first token is then dominated by that prefill. The summary output itself is the same size either way (19.5k tokens after the compaction), so the extra time is almost entirely the prefill. If a warm compaction of this size would take about 30 to 35 s (an assumption: the shortest warm runs were 30 to 40 s), the extra 130 s or so is consistent with a prefill of about 3,000 to 4,000 uncached tokens per second.

Two independent things made the cache cold here:

1. **The cache had expired.** Entries in this session are written with a one-hour lifetime (`cache_creation.ephemeral_1h_input_tokens` in every usage record). The last request was 5 h 40 min before.
2. **The model changed.** A prompt cache is specific to one model, so Opus 5.5 could not have used a cache written by Sonnet 5.5, even if it had still been alive.

Neither is a bug. AgentMux restarted the session for the model change as designed (the eight resume hooks), the hooks all returned success, and nothing in the compaction path added delay.

## 5. Why it looked stuck

- The pane showed `compacting` and nothing else for 2 min 44 s. The session stream only repeated a `status: compacting` event (six of them during the wait); there is no progress or elapsed-time indicator, and no hint that a cold start makes it long.
- The operator had just changed the model and clicked compact within about 80 s, so the model switch was the natural suspect. The wait was consistent with a hang for anyone who hadn't seen a cold compaction before.

## 6. What this analysis cannot say

- **Which factor cost more:** the model switch and the 5.7-hour idle happened together, and no compaction in the sample had one without the other. A same-model compaction after a long idle, or a model switch with a warm cache, would separate them. (A model switch with a warm cache can't be tested by waiting: the switch itself restarts the session, but the old model's cache would still be warm for the first hour. Compacting right after the switch, within the hour, would show the switch's cost alone.)
- **The request's real token accounting:** the compaction request's own usage (cache read versus uncached input) is not written to the transcript. The cold-cache cause is inferred from timing, from the one-hour cache lifetime and from the model change; it is not read from a usage record. If the claude CLI's debug log or an API usage dashboard shows the request, it would confirm the split directly.
- **The cost:** a cold read of 438k tokens is billed as uncached input and cache writes, not cache reads, so this compaction cost more than a warm one. By how much depends on the model's price list, which I did not look up.
- **Whether Opus itself was slower:** not separable from the above. Warm Opus 5.5 compactions are fast, so there is no sign that Opus is slower per token.

## 7. What would help

For the operator, now:
- **Compact before a long break or a model switch**, while the cache is warm, or accept that the first request after either is slow and costs more. After this compaction the context is about 19.5k tokens, so every later turn on Opus is cheap and fast.
- **A manual compaction right after a long idle is the slow case**, whatever the model. Auto-compactions during active work are warm.

For AgentMux, if you want it (none of this is done):
1. **Show elapsed time in the pane while compacting**, and say when the previous turn was more than an hour ago ("the cache has expired, this may take a few minutes"). That would have turned "stuck?" into "waiting".
2. **Surface the compaction's token usage** (cache read versus uncached) in the transcript or the Memory/usage views, so this kind of question can be answered from data. Today the number is simply not recorded.
3. **Warn before a model switch on a large context** that the next request will not use the cache and will cost the full price of the context once.

## 8. Method

- Read Manoz's session transcript (`1a8006ac-...jsonl`, 1,534 records) and the bashwrap debug log around 17:25 to 17:34Z.
- Scanned all agents' session transcripts under `~/.agentmux/shared/identities` for `compact_boundary` records (53, 22 of them from the 5.5 models) and joined each with the time of the previous model turn and the model before and after.
- No change was made to any agent, session or setting.

Agent: Lazo@Area54
