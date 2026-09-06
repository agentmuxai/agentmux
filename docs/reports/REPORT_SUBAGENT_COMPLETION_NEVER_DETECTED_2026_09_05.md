# Every Agent-tool subagent is reported "interrupted" — completion detection keys on a line type AgentMux never writes

**Status:** active
**Author:** Posa
**Date:** 2026-09-05
**Severity:** display-only, but 100% reproducible and it mislabels successful work as failed
**Resolution:** fixed 2026-09-06 (§7)
**Investigated at:** `1cb5b51c0` (v0.55.36)
**Companion:** `REPORT_SWARM_PANE_VS_ACTUAL_AGENT_CALLS_2026_09_05.md` (the audit that surfaced this)

---

## 1. Symptom

The Swarm pane shows **4 Agent-tool rows for Manoz, all "interrupted."** All four in fact
completed successfully, days apart. This is not specific to Manoz — see §4, it is every
Agent-tool dispatch on this machine.

## 2. Root cause

`SubAgentStatus::Completed` has exactly **one** writer
(`agentmux-srv/src/backend/subagent_watcher/jsonl.rs:243-248`):

```rust
if let Some(last) = new_events.last() {
    if matches!(&last.event_type, SubagentEventType::Result { .. }) {
        completed = true;
        state.info.status = SubAgentStatus::Completed;
    }
}
```

`SubagentEventType::Result` is produced by exactly one branch of the parser
(`parse.rs:311`), matching a JSONL line whose **top-level `type` is `"result"`**.

**AgentMux's agent transcripts never contain a `"type":"result"` line.** Not in subagent
sidechain files, not in parent session files. So the only path to `Completed` can never be
taken, for any Agent-tool dispatch, ever.

What happens instead:

1. Subagent is inserted `Active` (live) — `jsonl.rs:134`.
2. It runs, returns its result to the parent, and its transcript ends on an `assistant`
   message. No `result` line is written, so step 2 above never fires.
3. The parent's turn ends. On the next backfill scan `reconcile_stale_subagents`
   (`scan.rs:146-151`) downgrades any still-`Active` subagent whose parent turn is
   confirmed idle to `Abandoned`, on the documented reasoning that *"any subagent file
   lacking a terminal `Result` line was interrupted (crashed, killed, or the app/srv
   restarted mid-task)"*. That premise is false here: the file lacks a `Result` line because
   **nothing ever writes one**, not because the subagent was cut off.
4. `swarm-view.tsx:187` renders `abandoned` as **"interrupted."**

(A cold-backfill replay reaches the same end state one step earlier: `jsonl.rs:134-144`
inserts directly as `Abandoned` when the parent turn is already idle.)

The code is self-consistent and each piece is individually defensible. The defect is that
`Abandoned` is — as `types.rs:66-72` says itself — **"always an inference, never an
observation,"** and the observation it infers from is unobtainable.

## 3. Evidence

### 3.1 The four dispatches all succeeded

From Manoz's parent transcript, `tool_result` for each `Agent` call:

```
is_error=false  len=1095   "Verify agent isolation from host CLAUDE.md"
is_error=false  len=26349  "Research muxspect architecture for cross-tier spec"
is_error=false  len=1095   "Investigate dual login-button UI states"
is_error=false  len=31351  "Research AgentMux shutdown sequence and splash pattern"
```

Four results, zero errors, two of them 26–31 KB of research that Manoz then acted on.

### 3.2 No subagent transcript on this machine has a `result` line

All 11 subagent transcripts across every project dir under the live identity:

```
no-result  agent-a5cb2c4d2e2c1e79e.jsonl  last=assistant   {"user":64,"attachment":2,"assistant":96}   manoz
no-result  agent-a6418e6524f14ab61.jsonl  last=assistant   {"user":43,"attachment":2,"assistant":63}   manoz
no-result  agent-a829bb416fe845a76.jsonl  last=assistant   {"user":25,"attachment":2,"assistant":39}   manoz
no-result  agent-a8d67573ed85ea142.jsonl  last=assistant   {"user":43,"attachment":3,"assistant":72}   manoz
no-result  agent-a22b6ede7fe421c9b.jsonl  last=assistant   {"user":23,"attachment":3,"assistant":32}   posa
no-result  agent-a98dbcef0c55eef91.jsonl  last=assistant   {"user":46,"attachment":2,"assistant":74}   posa
no-result  agent-ac034d6a6600ca691.jsonl  last=attachment  {"user":1,"attachment":2}                   0.55.31 portable
no-result  agent-a22c5c89aad00e3b3.jsonl  last=attachment  {"user":1,"attachment":2}                   0.55.31 portable
no-result  agent-a1c5fc5e431269a53.jsonl  last=attachment  {"user":1,"attachment":2}                   0.55.31 portable
no-result  agent-a8902c88c3e0dff7b.jsonl  last=attachment  {"user":1,"attachment":2}                   0.55.36 portable
no-result  agent-abb935044040bc964.jsonl  last=assistant   {"user":2,"attachment":2,"assistant":5}     0.55.36 portable

TOTAL subagent transcripts: 11   with a result line: 0
```

Two different agents, two dispatch shapes, portables from 0.55.31 through 0.55.36. Every
completed one ends on `assistant`.

### 3.3 Parent transcripts don't have them either

Manoz's parent session (`468e2051…`, 15k lines) type census:

```json
{"queue-operation":596,"user":2661,"attachment":4405,"atis-latch":665,
 "assistant":4915,"ai-title":665,"last-prompt":663,"mode":563,"system":3}
```

No `result`. The line type the detector waits for is absent from the whole format.

### 3.4 Not a regression — it has never worked

| Transcript | Claude Code version | entrypoint | `result` lines |
|---|---|---|---|
| manoz `b92b1599` (Aug 16) | 2.1.198 | `sdk-cli` | **0** |
| manoz `49f8b975` (Aug 15) | 2.1.198 | `sdk-cli` | **0** |
| manoz `468e2051` (current) | 2.1.247 | `sdk-cli` | **0** |

Both the old and current CLI versions, same answer. AgentMux spawns agents through the SDK
(`entrypoint: sdk-cli`), and that path does not emit `"result"`-typed transcript lines.

### 3.5 The code comment records the moment this broke

The current detector arrived in `5d374006a` ("refactor: implement Tier 2 large-file
modularization items", #2283), with this rationale:

> Keyed off the `Result` discriminant itself (a real `"result"`-typed JSONL line), not
> derived text content — real Claude Code result events populate `result`/`content`, so
> matching against the "Subagent completed" placeholder (only ever produced when both are
> absent) almost never fired.

The previous approach matched placeholder text and "almost never fired." The replacement
keys on a discriminant that **never** fires. A change intended to tighten a flaky heuristic
replaced it with one that is unreachable — and because the failure mode is identical in
appearance (rows that never complete), it read as the pre-existing flakiness rather than a
new absolute.

## 4. Scope

Every Agent-tool dispatch, for every agent, since at least Claude Code 2.1.198. There is no
"sometimes" here — `Completed` is unreachable for this path, so the terminal state is always
`Abandoned`/"interrupted" once the parent turn goes idle.

`DispatchStatus` (`jsonl.rs:709`, "counts-complete + 60s quiet ⇒ Completed") is a **separate**
enum with its own, working, count-based completion rule. Workflow/dispatch-level rows are not
affected by this; only per-subagent `SubAgentStatus` rows are.

## 5. Fix options

Option 2 was implemented — see §7. Recorded as originally weighed, in rough order of directness:

1. **Treat a terminal `assistant` message as completion** when the parent turn has ended.
   This is what actually marks the end of a subagent's transcript in this format. Matches
   observed reality; the risk is calling a genuinely-killed subagent "completed" when it
   happened to die right after an assistant message.
2. **Use the parent's `tool_result` as the completion signal.** Strongest evidence available
   — it is the actual observation that the dispatch returned, including `is_error`, and it
   would let a real failure be distinguished from a real success rather than collapsing both
   into "interrupted." Requires correlating `tool_use_id` → dispatch, which the watcher does
   not currently track.
3. **Invert `reconcile_stale_subagents`' default.** Least code, weakest result: absent
   positive evidence of interruption, do not assert it. Removes the false "interrupted" but
   leaves rows in an indefinite state rather than correctly completed.

Option 2 is the one that makes the status *observed* rather than inferred, which is the
actual root problem — `types.rs` already flags the inference as the weak point in its own
doc comment.

## 6. What I did not verify

- Whether any *other* AgentMux spawn path (non-SDK, interactive `claude`) does produce
  `result` lines. Every transcript on this machine is `sdk-cli`, so the "never" in §2 is
  proven for this deployment but not for every possible configuration.
- Whether the pre-#2283 placeholder matcher ever worked in some earlier CLI version. The
  comment says "almost never fired," which I take at its word rather than having tested it.
- Live Swarm UI state — I reconstructed the display path by reading
  `subagentDisplayStatus` (`swarm-view.tsx:186-192`) and the watcher, not by observing the
  rendered rows. The user's report of "4 interrupted" is the observation this matches.

---

## 7. Fixed (2026-09-06)

Implemented **option 2** from §5 — completion is now an observation, not an inference.

**`agentmux-srv/src/backend/subagent_watcher/completion.rs`** (new): resolves a subagent's
terminal status from the parent's own `tool_result` for its `tool_use_id`. The correlation
key needed no new plumbing — Claude Code already writes it to the `agent-<id>.meta.json`
sidecar beside each transcript:

```json
{"agentType":"Explore","description":"Research AgentMux shutdown sequence and splash pattern",
 "toolUseId":"toolu_01RY4RsXyFZAQJP82G7Zw4YX","spawnDepth":1,"model":"opus"}
```

**`scan.rs::reconcile_stale_subagents`**: instead of downgrading every still-`Active` member
to `Abandoned`, it now consults that resolution. Members the parent recorded a result for
become `Completed` and get the same `subagent:completed` event `jsonl.rs` already emits live,
so open Swarm panes see the correction immediately rather than at the next unrelated reload.

Three properties worth noting, because each was a way to get this wrong:

- **Fails closed.** No sidecar, an unreadable/absent parent transcript, an uncorrelatable
  entry — every one falls back to the historical `Abandoned`. The fix can never
  *optimistically* complete something it could not verify.
- **The lock is not held across the file IO.** A parent transcript is routinely tens of MB,
  and this function is explicitly written to avoid holding the `sessions` mutex across slow
  work. Resolution is two-phase: snapshot candidates under a short lock, read unlocked, then
  apply — and the apply pass re-checks `Active`, so a member whose status moved in between is
  left alone rather than clobbered by a stale decision.
- **One transcript read per pass, not per member.** All of a session's members share a parent.

**An errored result still counts as `Completed`.** The distinction being drawn is *returned*
vs *cut off*, and a dispatch that reported an error returned. Surfacing failure as its own
state needs a status the enum doesn't have (plus a matching frontend union) — deliberately
left as follow-up rather than smuggled into this fix; `is_error` is parsed and available for
whoever picks that up.

**Testing.** 10 unit tests, fixtures taken from the real transcripts in §3 rather than
invented. Mutation-checked: with `terminal_status` reverted to the old always-`Abandoned`
behaviour, exactly the two completion assertions fail while the genuinely-abandoned and
fallback cases keep passing — confirming they test the fix rather than restating it.
Full `agentmux-srv` suite: **3017 passed, 0 failed**.

**Not verified end-to-end in a running app.** The mechanism is proven at the unit level and
the fix targets the exact line that produced the wrong status, but I have not watched Manoz's
four rows flip from "interrupted" to completed in a live Swarm pane. That needs a build and a
pane reopen (reconcile runs on backfill), and is the natural confirmation step.
