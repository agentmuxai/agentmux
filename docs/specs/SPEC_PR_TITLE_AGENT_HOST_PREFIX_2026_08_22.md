# SPEC: PR title `Agent@host` prefix for shared-identity agents

**Date:** 2026-08-22
**Status:** Implemented
**Author:** Korp
**Repos touched:** `agentmux` (this doc + `CLAUDE.md` agent-facing policy)
**Related:** `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` (the existing
`<!-- agentmux:agent_id=... -->` body-tag mechanism this change complements,
does not replace)

## 1. Problem

`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md` already solves *automated*
review-notification routing for an agent pushing under the shared
`GenericAgentX-<host>` fallback account (`CLAUDE.md`'s "Which GitHub account
am I acting as?" — the identity `scripts/gh-agent.sh` falls back to when an
agent has no dedicated PAT registered): the PR-body tag tells the webhook
consumer exactly which agent to notify, even though the GitHub *username* on
the PR is the same shared account regardless of which agent actually opened
it.

That mechanism is invisible to a **human**, though. Someone scanning
`github.com/agentmuxai/agentmux/pulls` sees a flat list of titles and author
avatars — every PR opened by a shared-identity agent shows the identical
generic author (`GenericAgentX-asaf`, or whichever host-suffixed variant),
with no way to tell which agent opened which PR without clicking into each
one individually to read its body tag. Confirmed live: this session's own
PRs (#2738, #2741, #2742) were all opened this way, indistinguishable from
each other in the PR list by author alone.

## 2. Change

Any agent operating under a **shared, non-dedicated** GitHub identity (in
practice: `scripts/gh-agent.sh` resolved the `gh-token-genericagentx`
fallback key, not a `gh-token-<your-id>` dedicated one) must prepend its PR
**title** with:

```
<AgentName>@<host>: <normal title>
```

Example, this session:

```
Korp@claudius: fix(muxlog): srv logs honor AGENTMUX_LOG_DIR, fix channels/ glob depth mismatch
```

- `<AgentName>` — `$AGENTMUX_AGENT_ID` in its natural/display casing (e.g.
  `Korp`), **not** lowercased. This is a deliberate, visible difference from
  the body tag's `${AGENTMUX_AGENT_ID,,}` convention: the tag is a
  machine-matched string (lowercasing keeps it a stable, unambiguous key);
  the title prefix is read by a human, where natural casing is the whole
  point.
- `<host>` — this machine's hostname (`$HOSTNAME` on bash/zsh/fish,
  `$COMPUTERNAME` on Windows — confirmed identical value on a real machine
  during this session: `claudius`).

**Agents with a standard identity are exempt** — a dedicated numbered
PAT/App account (`Agent3-<host>`) or a registered named peer account
(`korp-asaf`), per `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`'s exact
terminology. Their GitHub username already disambiguates them at a glance in
the PR list — the same exception the body tag already carries, extended
here to the title for the same reason.

## 3. Why the title, not just the existing body tag

The two mechanisms solve different problems and both are required together,
not one replacing the other:

| | Body tag | Title prefix |
|---|---|---|
| Reader | Machine (webhook consumer) | Human (PR list, notifications, git log) |
| Purpose | Route review notifications to the right agent | Let a human tell agents apart without opening each PR |
| Format | `${AGENTMUX_AGENT_ID,,}` (lowercase, exact-match key) | `$AGENTMUX_AGENT_ID` (natural casing, for reading) |
| Status | Already implemented, already correct | New as of this spec |

## 4. How to determine your own values

Both are already-available environment values — no new plumbing:
- `$AGENTMUX_AGENT_ID` — injected at agent spawn, same source the body tag
  already reads.
- Host — `$HOSTNAME` (bash/zsh/fish) or `$COMPUTERNAME` (Windows); both
  resolve to the same value on a given machine.

## 5. Non-goals

- Does not change `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`'s routing
  logic or the body-tag mechanism in any way — both still required,
  unchanged.
- Does not apply to standard-identity agents (own dedicated PAT/App account
  or registered named peer account).
- Does not retroactively rename already-*merged* PRs (not worth the
  history churn) — see §6 for what to do with PRs that were still *open* at
  the time this convention landed.

## 6. Migration for PRs open at the time of this change

This session had two open shared-identity PRs at the time this spec was
written (#2741, #2742) — both retitled with the `Korp@claudius:` prefix as
part of landing this change, rather than left inconsistent with the new
convention going forward.
