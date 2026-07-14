# SPEC — CLI pin consolidation + contract tests against the pinned Claude CLI

**Date:** 2026-07-14
**Status:** Part A implemented (this PR); Parts B–D proposed
**Motivation:** `docs/analysis/ANALYSIS_SUBAGENT_SPAWN_TAXONOMY_2026_07_14.md`
§4 — Anthropic's own docs (`code.claude.com/docs/en/sessions`) state the JSONL
transcript format "is internal to Claude Code and changes between versions, so
scripts that parse these files directly can break on any release."
`agentmux-srv/src/backend/subagent_watcher.rs` parses exactly those files, so
**every CLI version bump is a standing risk of silently breaking the Swarm
pane** (and anything else built on transcript observation). Today nothing
verifies a new CLI version still emits what AgentMux expects — we find out
from user-visible breakage.

**Related:** `SPEC_AGENT_MODEL_DROPDOWN_CLI_PIN_LOG_2026_07_02.md` (the pin
bump whose "single-source-of-truth follow-up" Part A finally implements),
`SPEC_PROVIDER_ISOLATION_2026_06_20.md` (INV-X: agents must run the pinned,
AgentMux-installed binary).

---

## 1. Current state (pre-PR)

The Claude CLI pin lived in **four places, already drifted three ways**:

| Site | Pin (pre-PR) | Consumer |
|---|---|---|
| `agentmux-srv/src/backend/providers.rs` | 2.1.198 | srv-side npm install (`install_handlers.rs`) |
| `frontend/app/view/agent/providers/index.ts` | 2.1.198 | install modal display + request |
| `agentmux-cef/src/commands/providers.rs` | **2.1.185** (stale) | host-side installer |
| `.github/workflows/container-image.yml` | **2.1.197** (stale) | Docker agent-image default |

The 2026-07-02 bump updated the first two, missed the last two — exactly the
drift trap that spec's follow-up item predicted. Additionally, the pin only
governs the **install** flow; `agents/runner.rs` launches whatever `claude`
resolves on PATH, so the pin is "the version we bless," not a runtime
guarantee.

## 2. Part A — pin consolidation (implemented in this PR)

- Bump `agentmux-cef` `CLAUDE_VERSION` 2.1.185 → 2.1.198 and
  `container-image.yml` default 2.1.197 → 2.1.198.
- Cross-referencing keep-in-sync comments at all four sites.
- **Drift guard:** `frontend/app/view/agent/providers/pin-consistency.test.ts`
  — reads all four sites (TS registry by import, the two Rust registries and
  the workflow YAML by source regex) and asserts equality for claude (all
  four) and codex/gemini (the three registries). Also asserts pins are
  concrete semvers, never `"latest"` (INV-X). Runs in the normal vitest suite
  on every PR, so the next partial bump fails CI instead of shipping.

A generated single-source file was considered and rejected for now: four sites
span TS, two Rust crates, and workflow YAML; codegen plumbing costs more than
the assertion test and the test fails equally loudly.

## 3. Part B — recorded-fixture contract tests (proposed, offline, every PR)

**Goal:** `subagent_watcher.rs`'s parsing assumptions are today encoded only as
hand-built inline JSON in its test module — synthesized from memory of what
the CLI emits, never validated against real output. Replace/augment with
**recorded real transcripts** from the pinned CLI version.

1. `scripts/record-cli-fixtures.sh` (dev-run, not CI): installs
   `@anthropic-ai/claude-code@<pin>` into a temp prefix, runs it headless
   (`--print`) in a scratch project with prompts crafted to produce each
   taxonomy shape from `ANALYSIS_SUBAGENT_SPAWN_TAXONOMY_2026_07_14.md`:
   - solo loose subagent (one Task/Agent call),
   - ad-hoc parallel batch (2+ same-turn calls → shared `slug`, no workflow dir),
   - dynamic-workflow run (`subagents/workflows/<id>/` + `journal.jsonl`),
   - nested subagent (≥2 levels, supported since CLI 2.1.172).
2. Sanitize (strip API-key-adjacent fields, normalize timestamps/ids where
   they don't carry shape) and check in under
   `agentmux-srv/tests/fixtures/cli/<version>/`.
3. Rust contract tests feed the recorded files through the **real**
   `subagent_watcher` parsing paths and assert the extracted
   `SubagentInfo` fields (slug present, workflow_id Some/None per shape,
   status transitions on `type:"result"`). These run in normal `cargo test` —
   no network, no key, no flakiness.

## 4. Part C — pin-bump-time live verification (proposed)

The pin is the contract boundary, so the **PR that bumps the pin** must prove
the new version. Process (documented in the bump checklist, enforced by
review):

1. Bump the pin (one logical change; drift test keeps the four sites honest).
2. Re-run `scripts/record-cli-fixtures.sh` against the new version; commit the
   regenerated fixtures alongside the bump.
3. The Part B contract tests now run against the new recordings — a format
   change surfaces as a red test **in the bump PR**, with the fixture diff
   showing exactly what the CLI changed, before any user installs it.

This is deliberately human-in-the-loop (like snapshot-test updates), because a
shape change usually needs a code decision, not a mechanical accept.

## 5. Part D — optional scheduled canary (proposed, alert-only)

A nightly/weekly CI job installing `@anthropic-ai/claude-code@latest`,
re-running the recording prompts with a real API key (CI secret), and diffing
shapes against the pinned fixtures. Early warning that a *future* bump will
hurt. Requirements that keep it honest: alert-only (never a merge gate — it's
network-bound and nondeterministic), token budget cap, and normalization of
known-nondeterministic fields (slugs, ids, timings) before diffing. Skip
until Parts B/C exist; it's an optimization of lead time, not correctness.

## 6. Non-goals

- Runtime version enforcement (`runner.rs` refusing an unpinned PATH `claude`)
  — a real gap but a product/UX decision (breaks users who intentionally
  self-update), tracked separately from test infrastructure.
- Contract tests for codex/gemini transcripts — same pattern applies later;
  Claude is where the Swarm/subagent surface actually parses output today.

## 7. Definition of done

- **A (this PR):** four sites agree at 2.1.198; `pin-consistency.test.ts`
  green; partial future bumps fail CI.
- **B:** recorded fixtures for the four shapes checked in; `cargo test`
  contract tests exercising real `subagent_watcher` parsing against them.
- **C:** bump checklist documented; first pin bump after B lands regenerates
  fixtures in the same PR.
- **D (optional):** scheduled workflow with secret + budget cap, alerting via
  GH issue on shape drift vs latest.
