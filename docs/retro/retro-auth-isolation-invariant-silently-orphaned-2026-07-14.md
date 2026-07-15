# Retro — the "never use the user's global auth" invariant existed, was written down, and was silently orphaned by an unrelated refactor

**Date:** 2026-07-14
**Trigger:** Live repro during auth stress-testing (documented in
`docs/analysis/ANALYSIS_ACCOUNT_DELETE_AUTH_LIFECYCLE_GAP_2026_07_14.md`):
deleting an Anthropic account in the Armory left a running agent fully
authenticated, and — the part this retro is about — a *respawned* agent
with no bound account fell all the way through to the user's real,
personal `~/.claude` login. That fallback should never have been reachable
at all.
**Audience:** anyone touching `agentmux-srv/src/identity/`, provider
credential injection, or the Armory account/identity UI. Read this before
changing how a spawn decides what credentials to use.

---

## 1. The invariant, as written

`docs/specs/SPEC_PROVIDER_ISOLATION_2026_06_20.md` states it as **INV-A**:

> the agent's live credential dir (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, …) is
> an **AgentMux-owned** path under `~/.agentmux/…`. It is **never** the
> user's `~/.<P>` dir. Tokens are read and **refreshed in the AgentMux dir
> only**.

Plus **INV-R** (the user's real dir may be read exactly once, on explicit
opt-in import, never written, never the live run target) and **INV-M**
(memory/state follow the same relocated dir, for free, once INV-A holds).

This is not a vague goal — it's a named, numbered invariant in a spec that
is still referenced by current code (`agentmux-common/src/data_paths.rs:378`
cites it by name). It was implemented. It worked. And nine days ago it
quietly stopped applying to a growing share of agents, with no test,
warning, or spec update to say so.

## 2. The five-commit arc

| Date | Commit | What it did |
|---|---|---|
| 2026-05-22 | `136f49fa` (#983) | **First attempt, incomplete.** Auto-seeds a "Default" identity bundle at every srv startup, binding every oauth-class provider to it — but the bundle's `secret_ref` pointed straight at the ambient `~/.claude`. Isolated in structure, not yet in substance. |
| 2026-06-20 | `1b91ec7b` (#1626) | **The invariant, actually delivered.** Repoints the Default bundle's account at `~/.agentmux/shared/providers/<provider>/` (`DataPaths::provider_auth_dir`), one-time read-only import of the real dir, sweep to repoint any stragglers. This is `SPEC_PROVIDER_ISOLATION_2026_06_20.md` and INV-A/R/M as shipped. From here forward, every agent — bound or not — ran isolated by construction, because *every* agent got the Default bundle binding at startup. |
| 2026-07-08 | `e5ab2d09` (#1624 PR-B) | **The orphaning commit — not malicious, explicitly flagged by its own author.** Flips the resolver from "direct link, fall back to bundle binding" to "direct link only." The commit's own doc comment names the exact gap: two live write paths (`bindidentityaccount`/`unbindidentityaccount`, and OAuth-Connect-into-bundle) still wrote *only* a bundle binding with no direct-link fan-out, and *"a binding created through either surface... will show as bound in the Armory UI but silently stop injecting at spawn."* The author's mitigation was a `tracing::warn!` + a `WaveEvent` with "nothing subscribing to it live yet" — visibility without a backstop — and an explicit note that closing it was **PR-C's job**. |
| 2026-07-12 | `e6cc2ed6`/`d2518d93` (#2133, "Armory Phase 4b/4c") | **Made it permanent.** Deletes `identity/migration.rs` outright — the whole Default-bundle auto-seed function — because it had "zero remaining consumers." True, but only because PR-B had already cut the read path four days earlier. This is the point of no return: after this commit there is no code left that would auto-provision *any* bundle binding, let alone an isolated one, for an agent that never explicitly bound an account. |
| 2026-07-14 | `537d86d7` (#2164, today) | **Band-aid, not a restoration.** Adds a binary spawn gate: block by default, or explicit `use_ambient_login=true` opt-in (true CLI-default ambient, no `CLAUDE_CONFIG_DIR` set at all). The governing spec (`SPEC_ACCOUNT_DELETE_DEAUTH_LAYERS_2_4_2026_07_14.md` §1) lists "hard-fail-always" and "keep-fallback-but-loud" as the alternatives considered — it never evaluates "auto-provision the isolated dir," because by the time it was written, nobody involved knew that used to be the default behavior nine days earlier. The m0017/m0018 migration then grandfathers every pre-existing linkless agent into `use_ambient_login=true` — meaning a real cohort of agents now runs with **zero** isolation, on purpose, going forward.

None of these five commits is individually wrong reasoning. PR-B's tradeoff (skip the fan-out, since PR-C would deprecate the old write paths "imminently") was reasonable in isolation. #2133's cleanup was reasonable in isolation ("zero remaining consumers" was true). Today's PR #2164 fixing the *visible* symptom (silent auth swap) was the right call given what was known. The failure is that **nobody, across five commits and eight weeks, re-checked INV-A** — a written, numbered, still-cited invariant — against the cumulative effect of unrelated refactors that each had a locally-good reason to touch the same code path.

## 3. Why this matters more than a normal regression

This is functionally identical to the lesson already written down once before, in
`docs/retro/retro-provider-auth-isolation-regression-2026-06-05.md` (a
different, earlier isolation bug — the "validate-spin" regression):

> A written invariant is worthless if nothing re-checks it when the ground
> moves.

That retro exists. It was read by whoever wrote `SPEC_PROVIDER_ISOLATION_2026_06_20.md`
two weeks later — the spec explicitly cites it. And the exact same failure
mode recurred anyway, on the exact same invariant, because the enforcement
mechanism was still "a spec exists" rather than "a test fails."

## 4. What's already there to build on (no new plumbing needed)

The isolated-directory infrastructure was never deleted — only its
automatic invocation at spawn time was:

- `DataPaths::provider_auth_dir(auth_dir_name)` — `~/.agentmux/shared/providers/<provider>/`,
  the exact shared dir `1b91ec7b` pointed the Default bundle at.
  Still referenced today (`agentmux-cef/src/commands/providers.rs`), by a
  comment that is now stale — it says Default agents "resolve to the shared
  dir," which is no longer true for any agent.
- `DataPaths::identity_dir(account_id)` — per-account isolated dirs under
  `~/.agentmux/shared/identities/<account_id>/`.
- `compute_and_ensure_account_dir` (`agentmux-srv/src/server/identity_handlers.rs:1096`) —
  mints an account id, creates its isolated dir, wires the config-dir env
  var. Real, working, tested — but only reachable from the interactive
  "Connect" button in the Armory, never automatically at spawn time.

The gap is entirely in `agentmux-srv/src/identity/resolver.rs`'s
`inject_identity_env_with_broker` / `gate_oauth_failure`
(`resolver.rs:676-702`): today it has exactly two outcomes for an
unresolvable oauth-class provider — block, or true ambient (no injection at
all). The third outcome that used to exist implicitly — auto-route to an
AgentMux-owned dir, no user action, no global-auth exposure — was never
re-implemented as an explicit option when the gate was designed today.

## 5. Reinforcement — how this doesn't come back a third time

Writing "don't do this" in a spec has now failed twice on this exact
invariant. The fix is to stop relying on specs as the enforcement
mechanism:

1. **A standing test that asserts the invariant directly, not a behavior
   that happens to satisfy it.** Add a resolver test — independent of
   `use_ambient_login`, independent of bindings existing or not — that
   spawns (or simulates spawning) an agent with **zero** identity
   configuration whatsoever and asserts the resulting env vars either (a)
   contain no `CLAUDE_CONFIG_DIR`-equivalent pointing outside
   `~/.agentmux/`, or (b) the spawn is blocked. The assertion should be
   phrased as "never resolves to a path outside `~/.agentmux`," not "matches
   today's specific code path" — so it survives the *next* refactor the way
   this one didn't survive PR-B.
2. **Decide, explicitly, whether the third outcome (auto-isolate) should
   return.** This retro is not the place to make that product call — flagging
   it as a follow-up decision, since `use_ambient_login=true` today means
   *true* ambient (zero isolation) rather than *isolated-but-automatic*, and
   the grandfather migration already moved a real cohort of agents onto that
   setting believing it was preserving their prior behavior. It wasn't:
   their prior behavior (pre-2026-07-08) was isolated auto-injection, not
   raw ambient. If the product decision is "yes, restore auto-isolate," the
   correct fix is a third `gate_oauth_failure` outcome that calls
   `compute_and_ensure_account_dir`-equivalent logic instead of `continue`-ing
   with nothing injected, and the migration's grandfather semantics need
   re-examination against what those agents *actually* did before.
3. **Cross-reference discipline**: when a PR's own doc comment names a known
   gap and defers it to a future PR ("PR-C" in `e5ab2d09`'s case), that
   deferral needs to survive as a tracked, blocking item — not a comment
   that a later deletion commit can render moot without anyone connecting
   the two. A `TODO`/tracking-issue reference in the code, checked by
   whatever eventually deletes the code it was deferred against, would have
   caught this before #2133 shipped.
4. **This file itself** — link it from `SPEC_PROVIDER_ISOLATION_2026_06_20.md`
   and from `identity/resolver.rs`'s module doc comment, so the next person
   editing the gate function reads this history before touching it.

## 6. What this retro is explicitly not

Not a claim that any individual commit was a mistake given what its author
knew at the time. Not a claim that PR #2164 (today) is wrong — its
fail-closed default is strictly safer than what it replaced, and its own
existence is evidence the team still cares about this invariant. It is a
claim that **the invariant needs a test, not just a spec**, because this is
the second time a spec alone has failed to prevent its own regression.
