# Retro: shared machine-wide git identity misattributes every agent's commits to AgentY

**Status:** retro

**Date:** 2026-08-22
**Owner:** AgentY
**Area:** local git config (this machine) / cloud GitHub notifications

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

- **Not the earlier merge-commit misattribution** (fixed in the private
  cloud repo on 2026-08-20, exact same *shape* of bug — "Naki's reagent
  jekts go to AgentY's"). That one was about merge commits. Checked commit history on 7 of the misfired PRs
  (#2761–#2766, #2768) via `gh api repos/.../pulls/<n>/commits`: the vast
  majority are single-parent, genuine feature commits — not merges. The #57
  fix doesn't apply here and isn't regressed; this is a different bug.
- **Not a PR-author routing problem.** The PRs carry the right title
  prefix and body tag, and their real authors were notified too (§7). The
  extra notifications follow the git commit author, not the PR author.

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
running on this shared Windows account (`user`), committing from a clone
that doesn't set its own local override, silently inherits **my** identity
as the git commit author, no matter which agent is actually doing the work.

This is a **two-layer identity system that only half-works**:

| Layer | Mechanism | Per-agent? |
|---|---|---|
| GitHub push/PR-open identity | `gh-agent.sh` resolves a per-agent token from the secret store, passed as `GH_TOKEN` scoped to one invocation | **Yes** — correctly isolated |
| Git commit author identity | `git commit` reads `user.name`/`user.email` from config (local → global) | **No** — falls through to one shared global config |

`gh-agent.sh` was built specifically to solve the first layer (its own
header comment: *"Agent2's shell inheriting Agent-Y's login... silently
wrong"*) but nothing did the equivalent for the second. The cloud
side isn't misbehaving — it notifies whoever the commit metadata says wrote
the commit, and `AgentY-asaf` → `agenty` is the right mapping. The metadata
itself is what's wrong.

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
- **Out of scope for this retro:** any change on the cloud side, which is
  a question for the private cloud repo; fixing the input comes first.

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

So the **PR-author notification correctly went to smike**, as the body
tag says. Smike receiving a jekt for this PR is not a counterexample to
§3–§4 above: agenty's copy follows the git commit metadata, which names
AgentY because of the shared `.gitconfig`. Only that *input* is bad.
