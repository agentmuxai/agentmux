# Retro: First Claude CLI + Model-Catalog Upgrade Done as a Deliberate Exercise

**Date:** 2026-08-27
**Severity:** N/A — not an incident. This is the first time a Claude Code CLI /
model-catalog bump was done as a scoped, retro'd exercise instead of an
incidental side effect of other work; logged so the *next* one benefits from
what this one found.
**Observed by:** Manoz (Claude agent)
**Related:** `docs/spec-claude-code-versioning.md` (the pre-existing pin
checklist this exercise followed and corrected), `docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`
(the forward-looking process this retro's findings feed into),
`frontend/app/view/agent/providers/pin-consistency.test.ts` (extended here),
`docs/specs/SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` (original
pin-consistency-test proposal), PR TBD.

---

## TL;DR

Bumped the pinned `@anthropic-ai/claude-code` CLI from `2.1.198` → `2.1.247`
(verified against the real npm registry) and relabeled the `opus` model-family
alias from "Opus 4.8" to "Opus 5" in the UI catalog. The mechanical part of
the bump was uneventful. The valuable finding was procedural: **a written,
human-maintained checklist (`docs/spec-claude-code-versioning.md`) had already
drifted** — it named a source file (`providers/index.ts`) that had been split
into `catalog.ts`/`types.ts`/`model-overlay.ts` weeks earlier, and it had
correctly *documented* a known gap (the Dockerfile `ARG` isn't covered by the
automated pin-consistency test) but that gap had sat undocumented-as-a-TODO
for over a month without becoming a test. Both are now fixed, and the second
one is now enforced, not just written down.

---

## What Happened

1. User asked to add Opus 5 to the model-select panel and upgrade the bundled
   Claude Code CLI, plus write a report and a forward-looking process spec —
   explicitly framed as "the first time we've really done this, but it will
   be the first of many."
2. Research pass found the model catalog's real shape: `frontend/app/view/agent/providers/catalog.ts`,
   a curated static fallback overlaid at runtime by a live `GET /v1/models`
   fetch (`model-overlay.ts` + `agentmux-srv/src/backend/model_catalog.rs`).
   `ProviderModel.value` is a stable family alias (`opus`/`sonnet`/`haiku`)
   that the CLI's own `--model` resolution maps to a concrete snapshot; only
   the curated `label` needs to track which snapshot that currently is — a
   pattern the file's own doc comments already document clearly.
3. Two decisions genuinely needed the user, not a guess: (a) relabel the
   existing `opus` entry vs. add Opus 5 as a second, independently-selectable
   entry, and (b) which concrete CLI version to pin, since the real npm
   registry (checked via `WebFetch`) doesn't know about a fictional-future
   "Opus 5" — asked both via `AskUserQuestion` rather than fabricating either.
4. Made the code change: `catalog.ts` (pin + label), `agentmux-srv/src/backend/providers.rs`,
   `agentmux-cef/src/commands/providers.rs`, `.github/workflows/container-image.yml`
   — the four locations `pin-consistency.test.ts` already covered.
5. **While updating `docs/spec-claude-code-versioning.md` to record the bump**
   (not part of the original plan — the file was found only because it
   showed up in a broader grep for stale "2.1.198" references), two gaps
   surfaced that the automated test had never caught:
   - The doc's own version-pin table pointed at `providers/index.ts`, a file
     that hadn't held the pin directly since the catalog module was split.
     Nobody had re-read the doc closely enough, on any prior bump, to notice.
   - The doc's own text already said, in plain language, "it does **not**
     cover the Dockerfile `ARG`, so that one location can still drift
     silently; double check it by hand when bumping" — a known, named,
     written-down gap that had persisted since the doc was created without
     ever becoming a test assertion.
6. Fixed both: bumped the actual Dockerfile `ARG CLAUDE_VERSION`, corrected
   the doc's file reference, and — instead of just re-writing the checklist
   text again — added a 6th assertion to `pin-consistency.test.ts` so a
   future missed Dockerfile bump is a CI failure, not a doc that says "double
   check it by hand" for another two months.
7. Ran the full provider/catalog/context-window test suite (139 tests) and
   `tsc --noEmit` — all green — before treating the code change as done.

---

## Root Cause (of the process gap, not a bug)

**A checklist that lives only in prose has no enforcement mechanism of its
own.** `docs/spec-claude-code-versioning.md` did everything a good runbook
should: it named all the pin locations, it explained *why* the Dockerfile
`ARG` existed as a separate case, and it even flagged its own blind spot in
writing. None of that stopped the blind spot from persisting for over a
month across at least one prior bump (`2.1.197` → `2.1.198`, per the doc's
own version-history table) — because "double check it by hand" only works if
every future bump is done by someone who (a) reads the doc closely and (b)
remembers to actually do the manual check it asks for, every single time,
forever. `pin-consistency.test.ts` already proved the better pattern for the
other four locations — it turned "remember to check" into "CI fails if you
don't" — but that pattern wasn't extended to the fifth location the doc had
already identified as a gap. The gap was *known* the entire time; the fix
was just never applied to itself.

This generalizes past just the CLI pin: **any process that depends on a
human re-reading a document accurately, under time pressure, once per
upgrade, will eventually fail exactly the way this one did** — not
catastrophically, just quietly, in a way that only surfaces the next time
someone reads the doc closely enough to notice it's wrong. This is the
central finding this retro exists to hand off to `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`.

---

## What Went Well

- The model catalog's existing design (alias `value` decoupled from a
  curated `label`, explicitly documented as needing a manual re-check "on a
  pin bump") made the Opus 5 relabel a one-line, low-risk change — the
  original author of that comment correctly anticipated this exact task.
- `pin-consistency.test.ts` caught zero regressions because there *were*
  none in the four locations it already covered — it did its job silently,
  which is the point.
- Asking the two genuinely-user-owned questions (relabel-vs-dual-entry,
  exact CLI version) up front, instead of guessing on version-critical files
  the repo's own review gate scrutinizes closely, avoided a wasted round
  trip.
- The gap that WAS found came from following the existing written checklist
  closely enough to update it accurately, not from a separate audit — a
  point in favor of always updating the doc as part of the bump rather than
  treating it as optional paperwork.

## Addendum: the review caught the retro repeating its own mistake

ReAgent's review of the PR this retro describes (P2, non-blocking) found that
step 6 above was incomplete: the Dockerfile `ARG` gap was closed in the
*test*, but the prose paragraph in `docs/spec-claude-code-versioning.md` that
had originally warned about the gap was left unedited — still stating, after
the fix, that `pin-consistency.test.ts` "does not cover the Dockerfile ARG...
double check it by hand." A doc describing behavior the code no longer had.
Fixed in the same PR. This is worth recording rather than quietly amending
away: it's a small-scale repeat of this retro's own central finding — a
human (agent, here) editing prose under time pressure missed updating one of
several places making the same claim, even while writing a document *about*
that exact failure mode. It's further evidence for §"Root Cause": review
gates that check the artifact (a test assertion, in this case) catch drift
that a human re-reading prose does not reliably catch, including when that
human is specifically primed to look for it.

**It happened a second and third time in the same review cycle.** Codex's
independent review of the same PR found the *same fix* still hadn't gone far
enough: the corrected paragraph now said `pin-consistency.test.ts` "enforces
agreement across all six locations," but the test only checks the **five**
locations that are actual matching version strings — the sixth row in the
doc's own table is the model catalog `label`, explicitly marked in that same
table as "not itself version-locked to the CLI pin." Self-reviewing to fix
Codex's finding, the exact same miscount ("all six") turned up in `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`
itself — this document's own §3.2 proposed a bump script that would "edit
all six pin locations via the same regex," which is not even mechanically
possible, since the label isn't a version string a regex can match. Three
edits to functionally the same claim, by the same author, across one review
cycle, each one individually plausible and each one wrong in a slightly
different way. Fixed all three; see both docs' current "five matching-string
locations plus one curated label" framing.

This is the strongest evidence this exercise produced for its own thesis.
It's not that checklists are hard to get right once — it's that "get the
count right in prose" is not a task that stays solved once you've solved it
one time; every subsequent edit to the same claim is a fresh chance to
re-introduce a slightly different version of the same imprecision, and nothing
about having just fixed it once makes the next edit safer. The fix that
actually holds is the one from §"What Went Well" and Tier 1 of the spec: a
test that parses the real files and asserts a real count, which cannot drift
no matter how many more times this paragraph gets edited by hand.

## What Would Help Next Time

- See `docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md` for the full
  proposal. Headline items: prefer a single script/test over a
  human-maintained checklist wherever the check can be mechanized (this
  retro's own Dockerfile fix is the template); track a per-model-alias
  "verified against pinned CLI as of version X" marker so a label going
  stale is at least detectable, not just a comment asking someone to
  remember; and treat "the doc says there's a known gap" as a tracked
  follow-up item with an owner, not permanent prose.
