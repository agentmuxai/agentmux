# Retro: shared machine-wide git identity misattributes every agent's commits to AgentY

**Date:** 2026-08-22
**Owner:** AgentY
**Area:** local git config (this machine) / `agentmux-cloud`'s github-consumer
"notify the committer" feature (`muxbus/consumers/github/handler.ts`)

---

## 1. Symptom

Over roughly two hours this session, agenty received a continuous stream of
`TIER=coord` jekts from `github-consumer` about Codex/ReAgent review activity
on PRs #2760–#2768 in `agentmuxai/agentmux` — none of which agenty opened,
touched, or has any memory of. The PRs' own titles/branches identify the real
authors plainly: `Korp@claudius: ...` (branch `korp/...`), `Smike@claudius:
...` (branch `smike/...`), and an untitled-prefix one on branch
`agentx/pane-block-stack-mount-flicker`. Per `SPEC_PR_TITLE_AGENT_HOST_PREFIX_
2026_08_22.md`, that `<AgentName>@<host>:` prefix means all three agents are
pushing under the shared `GenericAgentX-<host>` GitHub account and rely on
`gh-agent.sh` + the PR-body tag to route notifications correctly — a
legitimately common, expected setup.

## 2. False leads ruled out

- **Not the well-known merge-commit misattribution** (PR #37/#57 in
  `agentmux-cloud`, merged 2026-08-20, exact same *shape* of bug —
  "Naki's reagent jekts go to AgentY's"). That fix skips attribution when the
  head commit has 2+ parents. Checked commit history on 7 of the misfired PRs
  (#2761–#2766, #2768) via `gh api repos/.../pulls/<n>/commits`: the vast
  majority are single-parent, genuine feature commits — not merges. The #57
  fix doesn't apply here and isn't regressed; this is a different bug.
- **Not a bug in `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`'s username-
  first PR-author resolution.** That path is working correctly — it's *why*
  the PRs correctly show up under korp/smike/agentx's own identity in the
  first place (title prefix + presumably correct body tag). The bug is in
  the separate, secondary "also notify the most recent committer" feature.

## 3. Root cause

Every one of the 7 checked PRs' commits — regardless of which agent's GitHub
account opened the PR — has git commit author identity:

```
author: AgentY-asaf
email:  253608533+AgentY-asaf@users.noreply.github.com
```

100% consistent, ~15 commits checked across 3 different agents' branches.
Traced to this machine's **global** `~/.gitconfig`:

```ini
[user]
    name = AgentY-asaf
    email = 253608533+AgentY-asaf@users.noreply.github.com
```

Neither `amx` nor `agentmux-cloud`'s local clones (checked in agenty's own
working copies) have a `[user]` override in `.git/config` — so any agent
running on this shared Windows account (`asafe`), committing from a clone
that doesn't set its own local override, silently inherits **my** identity
as the git commit author, no matter which agent is actually doing the work.

This is a **two-layer identity system that only half-works**:

| Layer | Mechanism | Per-agent? |
|---|---|---|
| GitHub push/PR-open identity | `gh-agent.sh` resolves `gh-token-<agent>` from Secrets Manager, passed as `GH_TOKEN` scoped to one invocation | **Yes** — correctly isolated |
| Git commit author identity | `git commit` reads `user.name`/`user.email` from config (local → global) | **No** — falls through to one shared global config |

`gh-agent.sh` was built specifically to solve the first layer (its own
header comment: *"Agent2's shell inheriting Agent-Y's login... silently
wrong"*) but nothing did the equivalent for the second. The
github-consumer's "notify the committer" feature isn't misbehaving — it
correctly resolves `AgentY-asaf` → `agenty` (that mapping is right) and
notifies exactly who the commit metadata says wrote it. The metadata itself
is what's wrong.

## 4. Why this reads as "jekt misfires" rather than "git config bug"

From the receiving end, every symptom looks like the notification pipeline
is broken: unrelated PRs, urgent-priority spam, three agents' worth of noise
landing on one channel. The actual defect is upstream and invisible from
inside a jekt — nothing in the `[JEKT:...]` marker or the notification text
hints at "the git commit itself is lying about who wrote it." Only cross-
referencing the PR's own commit history (not just its GitHub author/title)
surfaced it.

## 5. Fix

**Not applied yet — deliberately.** `~/.gitconfig` is shared machine-wide
state; changing it unilaterally would just shift the misattribution onto
whichever identity I pick next, and could affect other agents' in-flight
work I can't see from here. Recommending, not doing, until confirmed:

- **Root fix:** at agent spawn/bootstrap, set `GIT_AUTHOR_NAME` /
  `GIT_AUTHOR_EMAIL` / `GIT_COMMITTER_NAME` / `GIT_COMMITTER_EMAIL` in each
  agent's own process environment, matching `$AGENTMUX_AGENT_ID`'s registered
  identity — the same pattern `gh-agent.sh` already uses for GitHub API
  auth, extended to git's *own* identity fields. Env vars take precedence
  over both local and global `.gitconfig`, need no per-repo setup, and
  can't leak between agents the way a shared global file does.
- **Cheaper stopgap:** each agent sets a local (`--local`, not `--global`)
  `user.name`/`user.email` override in every repo clone it commits from.
  Correct but has to be repeated per clone per agent — the env-var fix
  above is a single spawn-time change that covers every repo automatically.
- **Out of scope for this retro:** whether `handler.ts`'s committer-
  notification feature should also cross-check the resolved committer
  identity against something else before firing. Worth a second look once
  the identity source itself is trustworthy, but fixing correct code to
  compensate for wrong input isn't the right order of operations here.

## 6. Verification once fixed

Re-run the same check this retro used: `gh api repos/agentmuxai/agentmux/
pulls/<n>/commits --jq '.[] | .commit.author.name'` on a fresh PR from
another agent — should show that agent's own identity, not `AgentY-asaf`.

## 7. Confirmed: dual-delivery, not misrouted single delivery

Repo owner asked directly why agenty was getting these when smike was too —
worth stating precisely, since it's the piece that nails the mechanism down
rather than leaving it as a plausible theory. Pulled PR #2766 directly:

- GitHub author (who actually opened it): `GenericAgentX-asaf` — the shared
  fallback account, does not resolve to a standard identity.
- PR body: `<!-- agentmux:agent_id=smike -->`.

Per `SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`, an unresolved username
falls back to the body tag — so the **primary "PR author" notification
correctly resolves to smike**. Smike receiving a jekt for this PR is not a
counterexample to §3–§4 above; it's confirmation that the two notification
paths are firing independently exactly as designed: author-path reads the
tag (correct, → smike), committer-path reads raw git commit metadata
(wrong, → agenty, because of the shared `.gitconfig`). Only the second
path's *input* is bad — neither code path is misbehaving relative to what
it was given.
