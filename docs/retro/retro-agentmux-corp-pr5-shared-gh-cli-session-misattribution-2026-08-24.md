# Retro: agentmux-corp PR #5 opened under AgentY's GitHub identity instead of AgentX's

**Date:** 2026-08-24 (confirmation added 2026-08-25 by AgentX — see §7)
**Owner:** AgentY
**Area:** `gh` CLI authentication — leading hypothesis (§2a, added after
Codex's review of the PR shipping this retro correctly challenged the
original §2 conclusion) is a cross-agent binding in AgentMux's own
identity-resolver system (`identity/resolver/`), which injects
`GITHUB_TOKEN`/`GH_TOKEN` from a linked account at agent launch — a
*third* git/GitHub identity system in this codebase, distinct from both
the shared local `gh` keyring (§2's original, now-secondary theory) and
`retro-shared-git-identity-committer-misattribution-2026-08-22.md`'s
`git commit` author metadata (a different bug, already fixed, PR #2777).

---

## 1. Symptom

Over the course of this session, `agenty` received four `TIER=coord` jekts
from `github-consumer` about ReAgent/Codex review activity on
`agentmuxai/agentmux-corp#5` ("docs(trademark): log Office Action #1 +
counsel response packet") — a PR agenty never opened, touched, or has any
memory of. The branch name (`agentx/office-action-1-response-packet`) and
the PR's own content (USPTO trademark filing response work) both point
unambiguously at AgentX, not agenty.

## 2. Root cause — revised after review (see §2a): a more specific, better-evidenced mechanism than originally reported

**Correction, 2026-08-24, same day:** the section below (kept for the
record) was this retro's *original* conclusion. Codex's review of the PR
that shipped this retro (PR #2791) correctly challenged it: "these
observations establish the PR author, the existence of AgentX's PAT, and a
later keyring state, but they do not prove that the creator invoked plain
`gh`... an inherited AgentY token... would produce the same author even if
the wrapper were used." That's right — I had proven the *symptom* and one
*plausible* vector, not the actual causal chain. Digging further to answer
that challenge directly turned up harder evidence for a different,
more specific mechanism — see §2a below, which supersedes this section's
"Conclusion" paragraph. Original text, unedited:

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

~~**Conclusion:** whatever process opened PR #5 called plain `gh pr create`
directly instead of going through `scripts/gh-agent.sh`.~~ **Superseded —
see §2a.** (Reasoning about `gh-agent.sh`'s fallback behavior below is
still correct on its own terms, just not the actual mechanism here.) Since
AgentX has a correctly-registered dedicated PAT, going through the wrapper
would have authenticated as `AgentX-asaf` with no fallback involved at all
— there is no code path in `gh-agent.sh` that could produce a DIFFERENT
agent's own dedicated identity as a "fallback" (its only fallback is the
shared `GenericAgentX-<host>` account, never another named agent's PAT).

## 2a. What actually explains it: a cross-agent GitHub identity binding

Investigating Codex's challenge (was there real evidence of a bypassed
wrapper, or only of the outcome?) turned up a live, concrete artifact
instead of another plausible-but-unproven theory:

```
$ echo $GH_TOKEN
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
```

**My own (agenty's) shell already has `GH_TOKEN`/`GITHUB_TOKEN` set** —
not something I exported this session. Traced the source:
`agentmux-srv/src/identity/resolver/{inject.rs,provider.rs}` is a real,
deliberate AgentMux feature — for a linked `github`-provider identity
account, its resolver injects **both** `GITHUB_TOKEN` and `GH_TOKEN` into
a launched agent process's environment (`provider.rs:53`,
`env_vars: &["GITHUB_TOKEN", "GH_TOKEN"]`). This is a *third* git/GitHub
identity system in this codebase — distinct from both the 2026-08-22
git-commit-author fix and the manual `gh-agent.sh` + `secrets` CLI PAT
system used throughout this session.

Called `IdentityAccounts` (my own linked-identity list — same MCP surface
every agent has for its own account bindings):

```
{
  "account_id": "15f7fe0a-7827-4c27-8456-f08da8df9ae5",
  "name": "AgentX GitHub",
  "provider": "github",
  "kind": "api_key",
  "status": "valid"
}
```

**My own agent identity has an account literally named "AgentX GitHub"
linked to it.** `IdentityValidate` on that account_id confirms it's the
exact same token as the `GH_TOKEN` in my shell (`masked_tail:
"••••••••THHB"` matches), and that GitHub itself now rejects it
(`401 Unauthorized` — the credential is currently dead, whether from
rotation or revocation, not evidence either way about its state when PR #5
was opened).

**This is a materially different, more specific finding than §2's
"bypassed the wrapper" theory** — and doesn't require anyone to have done
anything wrong at the command level. If AgentX's own identity bindings
reciprocally include an account bound to AgentY's GitHub credential
(unverifiable from here — `IdentityAccounts` only returns the calling
agent's *own* linked accounts, and I have no access to AgentX's), then
AgentX's own normal, correctly-invoked tooling — no `gh-agent.sh` bypass
needed at all — would have had `GITHUB_TOKEN`/`GH_TOKEN` pointing at my
credential injected automatically at launch, and picked up by `gh`
transparently (env-var tokens take precedence over keyring — the same `gh`
behavior Codex's review cited). This would fully explain PR #5's
attribution without invoking "someone forgot to use the wrapper" at all.

**Still not fully closed — stating plainly what remains unverified:**
whether this cross-linking (an account named for one agent, bound to
another) is a genuine bug in AgentMux's identity-resolver/binding layer,
or some intentional shared/pooled-account design not documented anywhere
this retro's author has found. Confirming that — and whether it's the
*actual* mechanism behind PR #5 specifically, as opposed to a related but
separate misconfiguration — needs either AgentX's own `IdentityAccounts`
output (not accessible from this agent) or the repo owner's direct
knowledge of how these bindings are supposed to work.

This is the exact failure mode `gh-agent.sh`'s own header comment names as
the reason it exists — *"Agent2's shell inheriting Agent-Y's login...
silently wrong"* — just via a different, previously-unconsidered injection
path than the one that comment was written about.

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

**Not a code fix from this session — this needs someone with visibility
this agent doesn't have** (either the repo owner directly, or an agent
that can inspect AgentX's own `IdentityAccounts`). Two distinct threads,
not one, per §2/§2a — don't collapse them:

- **Leading hypothesis (§2a) — the identity-resolver cross-binding:**
  confirm whether AgentX's own linked-identity list contains an
  agenty-owned (or agenty-named) GitHub account, the same shape as my own
  "AgentX GitHub" entry. If so, this is a real bug in how AgentMux's
  identity resolver (`identity/resolver/{inject.rs,provider.rs}`) binds
  `github`-provider accounts to agents — accounts should almost certainly
  be exclusive per-agent, not cross-linked, for the exact reason this
  incident demonstrates. If confirmed, the fix belongs in that resolver's
  binding logic, not in `gh-agent.sh` or any workflow-discipline change —
  no amount of correctly using `gh-agent.sh` protects against a launched
  process's OWN environment carrying another agent's injected
  `GITHUB_TOKEN`/`GH_TOKEN`.
- **Secondary, unconfirmed either way (§2 original) — a bypassed
  wrapper:** still can't be ruled out without AgentX's own command/
  environment logs, which this agent doesn't have access to. If §2a's
  cross-binding turns out to be unrelated or coincidental, this remains
  the fallback explanation, and `gh-agent.sh`'s own existing guidance
  already covers it.
- **Immediate, either way:** flag directly to AgentX (and the repo owner)
  that PR #5 on `agentmux-corp` is attributed to the wrong GitHub account
  and should probably be corrected/re-attributed if that matters for the
  trademark filing's own record-keeping.
- **Not designed here, worth considering once §2a is confirmed or ruled
  out:** should identity-account bindings be validated for
  exclusivity/uniqueness (an account shouldn't resolve to two different
  agents), the way `db_lan_peer_pubkey_pins` pins a first-seen key and
  rejects a later mismatch? A structural guard here would be a more
  durable fix than relying on every future binding staying correct by
  convention.

## 6. Verification once addressed

- For §2a: get (or have someone with access run) AgentX's own
  `IdentityAccounts` output and check whether it contains an account
  reciprocal to my "AgentX GitHub" one — an "AgentY GitHub"-named or
  otherwise agenty-owned entry. That's the piece that would fully close
  the loop.
- For either hypothesis: re-run this retro's own check on a fresh
  `agentmux-corp` PR from AgentX —
  `gh pr view <n> --repo agentmuxai/agentmux-corp --json author --jq .author.login`
  should read `AgentX-asaf`, not `AgentY-asaf`.

## 7. Confirmation (2026-08-25, AgentX) — §2a's missing piece, and it's not a reciprocal pair

Ran the exact check §6 asked for: my own (AgentX's) `IdentityAccounts`
output, via the same MCP surface AgentY used for theirs.

```json
{
  "account_id": "15f7fe0a-7827-4c27-8456-f08da8df9ae5",
  "name": "AgentX GitHub",
  "provider": "github",
  "kind": "api_key",
  "status": "valid"
}
```

**Same `account_id` as the one AgentY found injected into their own shell
— not a separate, symmetrically-misnamed "AgentY GitHub" entry on my
side.** §2a's framing ("if AgentX's own identity bindings reciprocally
include an account bound to AgentY's GitHub credential") assumed a pair of
mirrored mis-bindings; what's actually there is a **single identity-provider
account object, named "AgentX GitHub," visible/bound in both agents'
`IdentityAccounts` lists simultaneously**.

One difference from AgentY's observation worth noting: they found the token
already rejected by GitHub (`401 Unauthorized`) when they checked it on
2026-08-24. As of this check (2026-08-25), the same account shows
`status: "valid"` — either rotated/re-validated since, or `status` here
reflects something other than live GitHub-side validity (not verified
either way from this agent).

**Correction (Codex's review of the PR shipping this section): the
paragraph originally here called this "a confirmed resolver bug." That
overreached — struck and replaced.** Codex cited real code this section
didn't check first: `IdentityAccount` is explicitly documented as a
*reusable* credential (`identities.rs:103-105`); `agent_identity_link`'s own
uniqueness constraint is scoped to `(agent_id, provider)` only — nothing
prevents the same `account_id` from being linked to multiple different
agents (`identities.rs:544-561`); and there's a dedicated test,
`identity_delete_captures_all_affected_agents` (`identities.rs:894-928`),
that deliberately links two different agents to one account and asserts
both are correctly captured. Confirmed all three directly. **One account
linked to two agents is tested, intentional, supported behavior — not
evidence of a resolver malfunction.** The resolver injecting AgentY's
`GITHUB_TOKEN` from "AgentX GitHub" is exactly what it's supposed to do
*given that a link from AgentY to that account exists*.

What this confirmation actually establishes, then, is narrower than §5
concluded: the shared-account link is real (not a AgentY-side
misobservation), but **whether that specific AgentY→"AgentX GitHub" link
is a deliberate shared-credential setup or a mistaken/accidental binding is
still completely open** — and that's the actual thing worth investigating
next, not the resolver's binding-enforcement logic (which, per the code
above, is working as designed). Concretely: check `db_agent_identity_links`
for when/how that specific row was created, and by what — a manual UI
action, a seeding/migration script, or something else — rather than
assuming a code fix is needed in `identity/resolver/`.
