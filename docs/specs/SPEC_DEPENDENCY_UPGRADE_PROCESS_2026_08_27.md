# SPEC — A repeatable process for Claude model catalog + CLI version upgrades

**Date:** 2026-08-27
**Type:** Process spec (not primarily a code-change spec — most action items are checks/automation, small in scope individually)
**Status:** Proposed — item 1 (Dockerfile pin-consistency check) already implemented as part of the exercise that prompted this doc; the rest is unimplemented and intentionally incremental.
**Owner:** unassigned
**Scope:** `frontend/app/view/agent/providers/catalog.ts` (model catalog), the six Claude CLI version pin locations documented in `docs/spec-claude-code-versioning.md`, and — by extension, since the mechanism generalizes — any other provider's pinned CLI (`codex`, `gemini`, etc.).
**Related:** `docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md` (the concrete exercise this generalizes from), `docs/spec-claude-code-versioning.md` (the existing, narrower CLI-pin checklist this spec builds on top of, not replaces), `docs/specs/SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md` (the live API-catalog-overlay design), `docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` (original pin-consistency-test proposal), `frontend/app/view/agent/providers/pin-consistency.test.ts`.

> This is explicitly framed as the **first** of a recurring exercise (model
> releases and Claude Code CLI releases will keep happening), not a one-off.
> The goal is a process that gets *cheaper and safer* each time it's used —
> not a document that has to be perfectly remembered each time.

---

## 1. Problem

AgentMux pins two independent things per provider, and both need periodic
bumping as upstream ships:

1. **The CLI version** (`@anthropic-ai/claude-code@X.Y.Z`) — six source
   locations that must literally match (see `docs/spec-claude-code-versioning.md`).
2. **The model catalog** — a curated fallback list (`catalog.ts`) whose
   `label`/`description` fields describe what a stable alias (`opus`,
   `sonnet`, `haiku`) currently resolves to, live-overlaid at runtime by an
   API fetch (`SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md`) that can silently
   fail closed (missing keychain entry, 401, offline) back to that curated
   fallback.

Today's exercise (`retro-claude-cli-and-opus-5-upgrade-2026-08-27.md`) found
that the existing process — a well-written, human-maintained checklist doc —
had already drifted on two points: a stale file-path reference, and a
**known, explicitly self-documented gap** (the Dockerfile `ARG` wasn't
covered by the automated pin-consistency test) that had sat unfixed for over
a month despite being written down in plain English. Neither drift was
malicious or even surprising — it's what happens to any checklist that
depends on a human reading it closely, under time pressure, once per
upgrade, indefinitely. That failure mode, not any single missed file, is
what this spec is actually about.

---

## 2. Research: how the industry approaches this

Three distinct bodies of practice are relevant here, since AgentMux's
situation straddles two categories at once — a **pinned CLI dependency**
(ordinary software supply-chain problem) and a **model catalog that tracks
an upstream provider's own model lifecycle** (an AI-specific problem with its
own emerging conventions).

### 2.1 Dependency/CLI pin upgrades (general software practice)

- **Update small and often, not big and rare.** Frequent bumps have small,
  fast-to-read changelogs; infrequent bumps accumulate risk into one large,
  hard-to-review jump.
- **Automate the mechanical part; keep a human gate for judgment.** The
  Renovate project's own upgrade-best-practices guidance (the most widely
  used automated-dependency-PR tool) recommends grouping by risk tier
  (security-critical alone; routine minor/patch batched), a minimum release
  age before auto-merge (their example: two weeks, to let upstream issues
  surface before you adopt them), and always running the full test suite on
  every update PR rather than trusting the diff by eye.
  ([Renovate: Upgrade Best Practices](https://docs.renovatebot.com/upgrade-best-practices/))
- **A central dashboard beats notification spam.** Track pending/available
  updates in one visible place (a tracking issue, a dashboard) rather than
  relying on someone remembering to periodically check `npm view`.
- **Pin where reproducibility matters, prune pins that don't earn their
  keep.** Google Cloud's dependency-management guidance frames this as a
  cost/benefit call per dependency — CI/CD and anything shipped to users
  should pin; low-stakes local dev tooling often shouldn't, since over-pinning
  blocks timely security patches.
  ([Google Cloud: Best practices for dependency management](https://cloud.google.com/blog/topics/developers-practitioners/best-practices-dependency-management))

### 2.2 Safe rollout of AI model version changes specifically

- **Canary/staged rollout, not instant cutover.** The standard pattern:
  route a small percentage of traffic to the new model version, watch
  latency (p50/p95/p99), error/refusal rate, cost per request, and
  user-feedback signals (regenerations, thumbs-down, session abandonment);
  widen the percentage only if metrics stay within threshold; auto-rollback
  on any metric breach — ideally before a human notices.
  ([MLflow: Canary Deployment for AI Models](https://mlflow.org/articles/what-is-canary-deployment-ai), [apxml: Safe Deployment and Rollout Strategies](https://apxml.com/courses/llm-alignment-safety/chapter-7-building-safer-llm-systems/safe-deployment-rollout-strategies))
- **The hard part is defining "worse," not routing traffic.** Multiple
  sources converge on this: the mechanics of a canary are well-understood;
  the actual difficulty is picking metrics that reliably capture a model
  regression under real production load.
- **AgentMux's own architecture already has a usable canary primitive.**
  Per-build channel isolation (`local-<branch>-<hash>-<build-id>`, see
  `CLAUDE.md`'s Multiple Instances section) means a version bump can already
  be built and run as its own fully isolated instance — data dir, cef-cache,
  auth — alongside the currently-shipping build, with zero risk of
  cross-contamination. This is most of a canary environment already built
  for an unrelated reason (multi-instance dev support); it just isn't
  currently *used* as a pre-merge verification step for pin bumps
  specifically (see §3.3).

### 2.3 Model alias / version lifecycle (Anthropic's own conventions)

Directly relevant since AgentMux's `opus`/`sonnet`/`haiku` catalog values
mirror this exact design one layer up:

- **Aliases resolve to dated snapshots; snapshots never change underfoot.**
  Anthropic does not silently update the weights behind an existing model
  ID — a new capability ships under a *new* ID, and an alias like
  `claude-sonnet-4-5` is a pointer Anthropic moves forward, not a live
  target that mutates in place.
  ([Claude Platform Docs: Model IDs and versioning](https://platform.claude.com/docs/en/about-claude/models/model-ids-and-versions))
- **A four-stage lifecycle with a committed notice period.** Active →
  Legacy (no more updates, may be deprecated) → Deprecated (still works,
  has an assigned retirement date and recommended replacement) → Retired
  (no longer available), with **at least 60 days' notice** before a public
  model is retired.
  ([Claude Platform Docs: Model deprecations](https://platform.claude.com/docs/en/about-claude/model-deprecations))
- **Later model generations move from floating aliases toward fixed,
  dateless canonical IDs** — i.e. the industry direction is fewer silently-
  moving targets over time, not more. AgentMux's catalog comment already
  anticipates this exact transition ("when a family ever has TWO live
  versions, switch those entries to concrete `--model` IDs").

**Takeaway for AgentMux:** the catalog's `label` field is functionally
tracking Anthropic's own alias-resolution state, but AgentMux has no
equivalent of Anthropic's own "Active/Legacy/Deprecated" staleness signal —
today, a label is either correct or silently wrong, with the only detection
mechanism being a human noticing during an unrelated task (exactly what
happened here).

---

## 3. Proposed process

Three tiers, ordered by how much of today's exercise they would have caught
automatically. **Tier 1 is implemented as part of this spec's own exercise;
Tiers 2–3 are proposed, not yet built** — deliberately incremental, per
§2.1's "small and often" principle, rather than one large tooling project.

### 3.1 Tier 1 — Close known gaps by turning prose into assertions (done today)

Every time a hand-maintained checklist doc says "X isn't automatically
checked, verify by hand" — as `docs/spec-claude-code-versioning.md` already
did for the Dockerfile `ARG` — that sentence is itself the spec for a missing
test. `pin-consistency.test.ts` now has 6 assertions instead of 5, covering
that exact gap. **Standing rule going forward: a checklist doc is allowed to
say "not yet automated," but that sentence should link a tracked follow-up,
not just persist as permanent prose.** A checklist gap that's been known for
over a month (as this one was) is a process failure regardless of whether it
was ever actually hit.

### 3.2 Tier 2 — A single bump script (proposed, not built)

Replace `docs/spec-claude-code-versioning.md`'s "How to bump" numbered list
(6 manual file edits + 2 manual verification steps) with one script,
`scripts/bump-provider-cli.sh <provider> <version>`, that:

1. Looks up the real latest version via `npm view @anthropic-ai/<pkg> version`
   when no version is given (matches §2.1's "verify against the real
   registry" practice this exercise followed by hand).
2. Edits all six pin locations via the same regex `pin-consistency.test.ts`
   already parses (so the script and its own test share one definition of
   "where the pins live" — the test would then be validating the script's
   output shape, not a second, independently-hand-maintained pattern).
3. Runs `pin-consistency.test.ts` immediately and fails loudly on mismatch,
   the same way `scripts/release.sh` already re-reads all five version
   locations after a release bump and fails loudly on disagreement
   (`CLAUDE.md`'s "Release consistency invariant" — same pattern, same
   author intent, not yet applied here).
4. Prints a reminder — not yet an enforced check, see §3.3's open question —
   to re-verify each model alias's curated `label` against what the new
   pinned version actually resolves that alias to.

This directly answers the user's "streamlined process" ask: today's bump was
6 file edits done by hand across two tool calls plus a doc read; a script
makes it one command with a built-in correctness check.

### 3.3 Tier 3 — Staleness signal for catalog labels (proposed, open question)

Unlike the CLI version pins (which are exact strings that either match or
don't — a mechanical check), a model catalog `label` like `"Opus 5"` is a
*claim about upstream state* that can go stale independent of any AgentMux
code change at all — Anthropic could ship a new snapshot under the `opus`
alias with zero AgentMux involvement. Two options, not yet decided between:

- **(a) Lean entirely on the existing live-overlay mechanism**
  (`SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md`) — the API-fetched catalog
  already corrects a stale label at runtime whenever the fetch succeeds; the
  curated fallback only matters when it's offline/unauthenticated. Under
  this view, the curated `label` is best-effort presentation for a
  degraded-mode fallback, not a correctness-critical value, and no further
  automation is warranted — just keep re-checking it by hand on each CLI
  pin bump (today's process, made slightly more visible by the Tier 2
  script's reminder).
- **(b) Add a lightweight "last verified" marker per model entry** (e.g. a
  comment or a structured field noting the pinned CLI version a label was
  last confirmed against), so a future pin bump can mechanically flag "this
  label hasn't been re-verified since 3 CLI versions ago" instead of relying
  on a human to think to check. Mirrors Anthropic's own Active/Legacy
  distinction (§2.3) one layer up, at the cost of a small amount of catalog
  schema growth.

**Recommendation:** start with (a) — it's zero-cost and the existing
degraded-mode framing is sound — and revisit (b) only if a stale-label bug
actually ships to a user (i.e. don't build the staleness-tracking machinery
speculatively; this spec's own §2.1 citation warns against exactly that kind
of premature process weight).

---

## 4. Non-goals

- **Full canary/A/B infrastructure for model rollouts** (§2.2's traffic-
  percentage/auto-rollback pattern) is not being proposed for AgentMux as
  built today. AgentMux ships a desktop app to individual users, not a
  multi-tenant inference service — there's no shared traffic to split. The
  closer analogue already exists (per-build channel isolation, §2.2) and is
  already available as a manual verification step (build a portable, smoke-
  test it) — formalizing that into an automated gate is a larger, separate
  effort out of scope here.
- **This spec does not change how the LIVE model-catalog API overlay works**
  (`SPEC_MODEL_CATALOG_REFRESH_2026_07_02.md` is unaffected) — only the
  curated fallback and the CLI pin process around it.
- **Not proposing Renovate/Dependabot-style full automation** of the CLI
  version bump itself (auto-opening a PR on every upstream release) — §2.1's
  own guidance to prefer deliberate, reviewed bumps over reflexive
  auto-updates applies especially strongly here, since a CLI pin bump can
  change agent behavior mid-session for every AgentMux user, not just swap
  a build-time dependency.

---

## 5. Progress tracking

| Item | Status | Notes |
|---|---|---|
| 1. Bump Claude CLI pin `2.1.198` → `2.1.247` (all 6 locations) | ✅ Done | Verified against the real npm registry via `WebFetch`; see the retro. |
| 2. Relabel `opus` catalog entry "Opus 4.8" → "Opus 5" | ✅ Done | Per user direction (relabel, not a dual entry) — `catalog.ts`. |
| 3. Fix stale file-path reference in `docs/spec-claude-code-versioning.md` | ✅ Done | `providers/index.ts` → `providers/catalog.ts`. |
| 4. Extend `pin-consistency.test.ts` to cover the Dockerfile `ARG` (Tier 1) | ✅ Done | 6th assertion; closes the gap the doc had named but never enforced. |
| 5. Retro this exercise | ✅ Done | `docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md`. |
| 6. This spec | ✅ Done | |
| 7. `scripts/bump-provider-cli.sh` (Tier 2) | ⬜ Not started | Proposed in §3.2; deliberately deferred past this exercise per "small and often." |
| 8. Catalog-label staleness signal (Tier 3) | ⬜ Open question | §3.3 — recommend deferring until/unless a real stale-label bug ships. |
