# Retro: agentmux-corp PR #5 opened under AgentY's GitHub identity instead of AgentX's

**Date:** 2026-08-24
**Owner:** AgentY
**Area:** local `gh` CLI authentication (this machine) — a *different* layer
of the same "shared identity on a shared machine" problem class as
`retro-shared-git-identity-committer-misattribution-2026-08-22.md`, not a
recurrence of that exact bug.

---

## 1. Symptom

Over the course of this session, `agenty` received four `TIER=coord` jekts
from `github-consumer` about ReAgent/Codex review activity on
`agentmuxai/agentmux-corp#5` ("docs(trademark): log Office Action #1 +
counsel response packet") — a PR agenty never opened, touched, or has any
memory of. The branch name (`agentx/office-action-1-response-packet`) and
the PR's own content (USPTO trademark filing response work) both point
unambiguously at AgentX, not agenty.

## 2. Root cause — confirmed, not inferred

Pulled the PR directly:

```
gh pr view 5 --repo agentmuxai/agentmux-corp --json author,headRefName
  author.login:   AgentY-asaf
  headRefName:    agentx/office-action-1-response-packet
```

The PR's real GitHub author is **`AgentY-asaf`** — my own dedicated
identity — not `AgentX-asaf`. Confirmed AgentX has its own correctly-
registered PAT that resolves to the right account:

```
$ secrets get services/infra --path gh-token-agentx --raw   # succeeds
$ curl -H "Authorization: token $TOKEN" https://api.github.com/user
  "login": "AgentX-asaf"
```

Also confirmed the shared, machine-wide `gh` CLI keyring — the one
`gh-agent.sh` exists specifically to bypass — currently holds a valid,
logged-in `AgentY-asaf` session:

```
$ gh auth status
  ✓ Logged in to github.com account a5af (keyring)
  ✓ Logged in to github.com account AgentY-asaf (keyring)
```

**Conclusion:** whatever process opened PR #5 called plain `gh pr create`
directly instead of going through `scripts/gh-agent.sh`. Since AgentX has
a correctly-registered dedicated PAT, going through the wrapper would have
authenticated as `AgentX-asaf` with no fallback involved at all — there is
no code path in `gh-agent.sh` that could produce a DIFFERENT agent's own
dedicated identity as a "fallback" (its only fallback is the shared
`GenericAgentX-<host>` account, never another named agent's PAT). Bypassing
the wrapper left `gh` to fall through to the shared local keyring's
currently-active account, which happened to be mine.

This is the exact failure mode `gh-agent.sh`'s own header comment already
names as the reason it exists: *"Agent2's shell inheriting Agent-Y's
login... silently wrong."* It fired here, for real, on a repo outside the
usual `amx` engineering workflow.

## 3. Why this is a *different* bug from the 2026-08-22 retro, not the same one recurring

`retro-shared-git-identity-committer-misattribution-2026-08-22.md` was
about **`git commit`'s** author metadata (`user.name`/`user.email` from a
shared global `~/.gitconfig`), fixed by injecting `GIT_AUTHOR_NAME` /
`GIT_COMMITTER_NAME` / etc. into each agent's own process environment at
spawn (`agentmux-srv/src/server/agent_handlers/input.rs`,
`git_identity_env_vars()`, PR #2777 — merged and deployed, riding in
v0.55.22+).

This is about **`gh`'s own OAuth session** — a completely different
credential surface. `GIT_AUTHOR_NAME`/`GIT_COMMITTER_NAME` env vars have
zero effect on which account `gh pr create` authenticates as; that's
controlled by `gh`'s own local keyring/config, which the August fix never
touched and was never meant to. **The August fix does not cover this case
and was never supposed to** — confirming that fix is working as designed
doesn't rule this one out, and it didn't.

## 4. Why ReAgent's routing itself is not the bug

Verified this is not a repeat of the `identity_links`-source-failure class
of bug or a `agentmux-cloud` consumer routing defect: `reagent` (the
Python Lambda review bot, `a5af/reagent`) has no agent-routing logic of its
own at all — grepped the full pulled-latest source
(`lambdas/`, `config/`) for `agent_id`/`muxbus`/`jekt` and found matches
only in `status.py`, an unrelated status-check endpoint.
`agentmux-corp` is not even explicitly listed in `reagent`'s
`config/repos.json` (only `a5af/claw`, `a5af/reagent`,
`agentmuxai/agentmux`, `agentmux-landing`, `agentmux-mobile` have entries)
— it's reviewed under the bare `"defaults"` block, no special routing
config. The actual "who gets notified" decision, per
`SPEC_AGENT_DETECTION_PRIORITY_2026_08_07.md`, happens in
`agentmux-cloud`'s `muxbus/consumers/github/` by resolving the PR
**author's own GitHub username** first. That logic did exactly its job:
`AgentY-asaf` is a real, standard, registered identity, so it correctly
resolved to `agenty` and notified me — accurately reflecting what GitHub
itself says about who opened the PR. The routing isn't wrong; the
underlying fact it's routing on (who really opened this PR) is.

## 5. Fix

**Not a code fix — this is a process/discipline gap, not a bug in any of
the four repos checked (`agentmux-corp`, `agentmux-cloud`, `reagent`,
`shared-infrastructure`, all pulled to latest `main` for this
investigation).** `gh-agent.sh` already fully solves this when used.
Recommending, not applying unilaterally (this is entirely about how other
agents — starting with AgentX — invoke `gh`, not something fixable from
here):

- **Immediate:** flag directly to AgentX (and the repo owner) that PR #5
  on `agentmux-corp` is attributed to the wrong GitHub account and should
  probably be corrected/re-attributed if that matters for the trademark
  filing's own record-keeping.
- **Root fix:** confirm why plain `gh` got invoked instead of
  `scripts/gh-agent.sh` for this specific PR — a one-off slip, a different
  habit on non-`amx` repos (this is the first "agenty gets misrouted
  notifications" incident that traces to a *non-engineering* repo), or a
  workflow that doesn't have `amx`'s `CLAUDE.md` (and therefore
  `gh-agent.sh`'s instructions) in context at all when working in
  `agentmux-corp`. `agentmux-corp` is a separate repo with its own
  checkout — worth checking whether it has its own `CLAUDE.md` /
  `scripts/gh-agent.sh` copy, or relies on agents remembering the
  convention from `amx` while working in a different directory entirely.
- **Structural, not yet explored:** could `gh-agent.sh`'s protection be
  made harder to accidentally bypass — e.g. a shell alias/wrapper shadowing
  bare `gh` in each agent's spawned environment, so the safe path is also
  the path of least resistance? Not designed here; flagged as a real
  option given this is the second real incident (a different layer each
  time) from the same underlying "shared machine, shared credentials by
  default" hazard.

## 6. Verification once addressed

Re-run this retro's own check on a fresh `agentmux-corp` PR from AgentX:
`gh pr view <n> --repo agentmuxai/agentmux-corp --json author --jq .author.login`
should read `AgentX-asaf`, not `AgentY-asaf`.
