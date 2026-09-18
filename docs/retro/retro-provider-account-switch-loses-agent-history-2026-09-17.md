# Retro: switching provider accounts silently orphans an agent's whole history

**Date:** 2026-09-17
**Status:** proposed — root cause confirmed; nothing shipped. Recommendations in §6.
**Related:** `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (per-channel
identity isolation, the mechanism this rides on), `docs/MUXSPECT.md`,
`mcp__agentmux__SearchHistory`

---

## 1. Symptom

Mid-turn, an assistant response was replaced by:

```
You've hit your monthly spend limit · raise it at claude.ai/settings/usage
· your weekly limit resets Sep 21, 7pm (America/Los_Angeles)
```

The operator asked one more question ("should we upgrade vite?"), got the same
message, and switched to a different Anthropic account.

The agent came back up with **an empty context and no access to anything it had
done that day** — and, critically, no indication that anything was missing. It
re-ran research it had completed 3 minutes earlier and re-derived the same
conclusion from scratch, while the operator watched a confident answer that
looked like fresh work.

## 2. Root cause

AgentMux keys per-agent provider state on an **identity UUID**, in two places:

```
~/.agentmux/channels/<channel>/identities/<uuid>/claude/   credentials, CLAUDE.md, plans, sessions
~/.agentmux/shared/identities/<uuid>/claude/projects/      the .jsonl transcripts + memory/
                                                            (the channel dir symlinks to this)
```

A different provider account gets a **new UUID**, therefore a new and empty
`projects/` tree. Nothing links the old identity to the new one — not a
pointer file, not a field in `.claude.json`, nothing.

Observed on this machine:

| identity | created | agent4 transcripts |
|---|---|---|
| `a1990489-6de6-484a-9e20-83688c641524` | 14:54 | `c3fc93de-….jsonl` — 13.7 MB, 4,336 entries |
| `f333cda4-1847-4488-a2b9-008789c31f92` | 22:05 | none |

This is per-account isolation working exactly as designed for *credentials*.
The problem is that conversation history and agent memory are stored inside the
same boundary, so they inherit an isolation guarantee nobody wanted for them.
The agent identity (`AGENTMUX_AGENT_ID=agent4`) is stable across the switch and
is the key a human reasons in — but it is not the key anything is stored under.

## 3. The recovery tool has the same blind spot

`SearchHistory` exists precisely for this ("search YOUR OWN past conversations —
including sessions that have ended or been compacted away"). It resolves history
under the **current** identity, so after a switch it searches an empty tree.

In this incident it didn't even get that far — it failed outright:

```
history search request failed: error sending request for url
(http://127.0.0.1:65289/agentmux/reactive/history/search?agent=agent4&query=vite)
```

That connection failure is worth its own look, but it is **not** the interesting
part. Had it succeeded it would have returned `hits: []` with
`truncated: false` — and the tool's own contract says `truncated: true` is the
"treat this as unknown" signal, which means a clean zero-hit result reads as an
authoritative *"you never did that."* There is no signal for **"your history
moved."** A tool whose stated purpose is protecting the agent from
confidently misreporting its own past will, in this exact scenario, actively
produce that misreport.

## 4. Why it stayed invisible

- The limit notice is written into the **assistant turn**, so the transcript
  records the agent as having said it. There is no structured
  "provider exhausted" event for AgentMux to react to.
- A fresh context is the agent's normal startup state. Nothing distinguishes
  "new session" from "new session that lost 7 hours."
- The re-derived answer happened to be *correct*, which is the worst case: it
  builds confidence in a path that silently discards state.

Cost here was low — ~15 minutes of duplicated research that converged. But the
pre-switch session had **already** survived a context compaction at 04:42 UTC,
so this was the second history loss inside 35 minutes, and the second one had
no summary carried across. Blast radius scales with how much uncommitted
reasoning the agent was holding.

## 5. What actually recovered it

Filesystem archaeology, in this order:

```bash
# 1. sibling identities, newest last — the switch is visible as a fresh dir
ls -la --time-style=+%F_%R ~/.agentmux/channels/<channel>/identities/

# 2. the project dir is the agent cwd with / and . mangled to -
ls -t ~/.agentmux/shared/identities/<old-uuid>/claude/projects/\
C--Users-asafe--agentmux-agents-agent4-0831d/

# 3. biggest/newest .jsonl is the orphaned session; parse, don't cat (13.7 MB)
```

The last-modified time on the old transcript (22:06) matches the new identity's
creation time (22:05) — that adjacency is the reliable tell that a switch, not
a normal session end, is what happened.

## 6. Recommendations

1. **Make `SearchHistory` / `ListConversations` identity-spanning.** Resolve by
   `AGENTMUX_AGENT_ID` across sibling identity dirs, not just the active one.
   If that's too broad a default, at minimum return `identities_scanned` so a
   zero-hit result is distinguishable from a wrong-tree result.
2. **Leave a breadcrumb on switch.** Write `previous_identity: <uuid>` into the
   new identity dir. One line makes §5's archaeology a lookup.
3. **Tell the agent.** A first-turn system note — "provider account changed;
   prior sessions for this agent are under identity `<uuid>`" — converts a
   silent amnesia into a recoverable one.
4. **Treat provider-limit exhaustion as an event, not a message.** While it
   arrives in the assistant slot, AgentMux can't distinguish it from output, and
   the transcript is left attributing it to the agent.

Items 2 and 3 are small and independently useful; item 1 is the real fix.
