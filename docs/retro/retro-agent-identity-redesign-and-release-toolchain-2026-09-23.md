# Retro: the agent-identity migration, and what blocked the release

**Date:** 2026-09-23
**Status:** retro
**Scope:** #3497 / #3501 / #3506 / #3520 / #3526 / #3527 / #3529, plus the
release-toolchain blocker hit while cutting v0.56.12.

---

## 1. What happened

Nine PRs merged. Phases 0, 1 and 4 of the canonical-agent-id migration landed
and are sound. **Phase 2 — the largest surface — was implemented, reviewed five
times, and abandoned as unworkable.** The spec was then redesigned twice more,
and the redesign is still not implemented.

Alongside that: the CI doc gate was made to actually block, a long-standing
`ETXTBSY` test flake was diagnosed, and a release bump was blocked by a
permissions gap and an npm bug.

## 2. The central lesson

> **Every failure in this sequence was a confident claim about code that
> nobody had run.**

Not one was a logic error, a race, or a misunderstanding of requirements. Each
was a plausible statement — usually *true of the function in front of me*, and
false of the system.

| Claim | Reality | Cost |
|---|---|---|
| "Launching one template twice yields duplicate slugs" | 0 duplicates across 47 databases; launches never inherit a template's slug | #3500 shipped with a false severity, corrected in #3510 |
| "The work queue has no authentication" | It sits behind `auth_middleware`; the key is just not per-agent | Public issue #3501 filed wrong, corrected |
| "`record_supervisor_decision` is broken in `main`" | `main` has no resolver; every key site agrees | Merged into a spec, corrected in revision 3 |
| "The frontend already carries the right UID" | Documented as going stale on pane reuse, with a regression test | Killed revision 1 of the redesign |
| "`agent_credentials.rs` can mint local tokens" | Cloud Cognito, needs a human login, falls back to a shared token | Killed revision 2's §6 |
| "Registry files have no `0o600`" | They do — taken from a reviewer and propagated unchecked | P1 on #3527 |

The last row is the sharpest: it was written **inside the section correcting
this exact failure**, and it came from trusting a reviewer who had been right
about four other things.

### 2.1 The reliable tell

Measured claims held up. Inferred ones did not — *especially* ones that felt
like natural consequences of something already verified. The pattern is not
carelessness; it is that a verified fact makes the next inference feel
pre-verified.

**Practice:** verify what you propagate, not only what you author. Treat "this
follows from what I just checked" as unchecked.

## 3. Why Phase 2 could not work

Worth recording because the conclusion is counter-intuitive and was reached
only by implementing it.

`derive_slug` lowercases and collapses. Measured:

```
resolve_agent_id("agenty")  = Ok("def-a")
resolve_agent_id("AgentY")  = Err(unknown agent)   <- what callers actually pass
```

And normalizing first is worse, not better:

```
Agent B (name "AGENTY") -> slug "agenty-2"
Agent C (name "AgentY") -> slug "agenty-3"
derive_slug(either)      -> "agenty" -> resolves to def-a   <- a third agent
```

**The lossiness of `derive_slug` *is* the collision.** Any resolver whose only
input is a display name either fails to resolve or misroutes. The information
needed is not in its argument — so no amount of implementation care helps.

The predecessor spec framed the task as *"the slug is used as a key in ~12
places; migrate them."* That framing is what made an impossible plan look like
a tractable one.

## 4. On review

Five rounds on #3520 found four P1s and a P0. **None would have been caught by
CI** — the suite was green at every step, including when the feature was doing
nothing.

Two properties of those findings:

- **Three were self-inflicted in sequence.** The fix for P1 #1 created P1 #2
  (pinning made lookups fail closed, which was then wrongly reused for
  teardown). Each fix moved a failure mode rather than removing one — a signal
  the design was wrong, which took too long to read as such.
- **Review quality tracked the concreteness of the artifact.** A 12-section
  design doc got a bare "LGTM". The same reviewer, given code, found real
  defects repeatedly. Judging a reviewer as "weak on design" from one empty
  first draft was itself a premature inference.

**Practice:** an LGTM on a design document is not a second opinion. Two
independent adversarial passes on the redesign found four P0s between them,
including one — deleting a map that resolves immutable GitHub PR-body tags —
that would have silently broken review routing with no possible backfill.

## 5. Tests that pass while testing nothing

Adding the §4 safety net silently invalidated three existing tests: they kept
passing because the lookup found *nothing matching*, not because the guard
refused an *ambiguous* one. Both return `None`.

The fix generalises: **assert the positive case first.** Force one row, assert
it resolves; add the second, assert it now refuses. The denial is then
provably caused by the second row rather than by an empty fixture.

Mutation testing earned its keep throughout — every guard in this sequence was
checked by reverting it and confirming exactly one test failed.

## 6. What blocked the release

`task release:patch` requires `@a5af/bump-cli`, which is unavailable:

1. **Permissions.** The `genericagentx-workflow` App (ID `4994463`) is
   installed on `a5af` (installation `162853501`) but lacks `Packages:
   Read-only`. The error distinguishes the two cases cleanly — an
   `agentmuxai`-scoped token gives *"the requested installation does not
   exist"*, an `a5af`-scoped one gives *"installation not allowed to Read
   organization package"*. The second means the package exists and only the
   grant is missing. **Human action required.**
2. **An npm bug blocks the source-build fallback.** `npm install` fails with
   `Cannot read properties of null (reading 'edgesOut')` on npm 10.9.9 / node
   22.22.1. Bisected: it needs **both** `bin` and `devDependencies` present —
   removing either fixes it, on an otherwise-identical manifest. Not
   workspace-related and not caused by stale state; it reproduces in an empty
   directory from the manifest alone.

Also worth noting: `release.sh` forces a patch bump while the pending
changesets compute a **minor** (there is a `feat(memory)` among them). The tool
warns, but the warning is easy to miss.

## 7. Actions

- [ ] **Human:** grant `Packages: Read-only` to App `4994463` and accept it on
      the `a5af` installation (`162853501`).
- [ ] Pin or document a working npm version for `dev-tools`, or restructure
      `bump-cli`'s manifest so the bootstrap build does not hit the arborist
      bug.
- [ ] Decide patch-vs-minor for the pending release — the changesets ask for
      minor.
- [x] Record why Phase 2 cannot work, so it is not re-attempted
      (`SPEC_CANONICAL_AGENT_ID_MIGRATION_2026_09_21.md` §6).
- [x] Make the doc gate actually block (#3511) — it had been advisory, and
      #3510 was merge-ready with it red.
- [ ] Implement the redesign only after a further adversarial pass on §4.1,
      which is the newest and least-attacked part.

## 8. What went right

- The data check that disproved my own claim was the highest-value hour spent.
  **Cheap measurement beats careful reasoning**, and it was available the whole
  time.
- Stopping was correct. Phase 2 could have been merged as a green, approved,
  no-op; recording *why* it cannot work is worth more than shipping it.
- Corrections were made in place, publicly, including in merged documents. A
  spec that records being wrong twice is more trustworthy than one that reads
  as though it was right first time.
