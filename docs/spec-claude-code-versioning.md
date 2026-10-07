# Spec: Claude Code Version Management

**Status:** Active  
**Current pinned version:** `2.1.288`
**Previous default:** `latest` (floating)

## Problem

Claude Code used to be installed with `@anthropic-ai/claude-code@latest` at container
image build time, so two builds minutes apart could embed different versions. It was
then pinned in the image build.

As of 2026-10-07 the published container image (`ghcr.io/agentmuxai/agent-base`) no
longer contains Claude Code at all (proprietary license; redistribution is not
cleared). AgentMux installs the pinned version from npm into the agent's container the
first time it starts (`crates/srv/src/backend/container_cli.rs`), so the pin that
matters for containers is the same one host installs use. See
`docs/specs/SPEC_CONTAINER_AGENTS_WORK_FOR_EVERYONE_2026_10_07.md`.

## Version pins (two matching-string locations, plus one curated label)

| File | Location | Purpose |
|------|----------|---------|
| `crates/srv/src/backend/providers.rs` | `pinned_version: "2.1.288"` (CLAUDE static) | Version the backend installs for host agents, and into a container agent's container on first start |
| `frontend/app/view/agent/providers/catalog.ts` (re-exported via `./index`) | `pinnedVersion: "2.1.288"` (PROVIDERS.claude) | Version surfaced in the UI |
| `frontend/app/view/agent/providers/catalog.ts` (same object) | `models: [{ value: "opus", label: "Opus 5.5", ... }]` | The curated UI label for the `opus` family alias: **not itself version-locked to the CLI pin**, but should be re-checked on every pin bump per the field's own doc comment ("kept in sync on a pin bump"): whichever concrete snapshot Anthropic's API currently resolves `--model opus` to. |

`frontend/app/view/agent/providers/pin-consistency.test.ts` enforces agreement
across the two version-string locations. It also checks that the image build
(`docker/Dockerfile.agent-agentmux` and `.github/workflows/container-image.yml`) does
not bundle Claude Code or grow a pin of its own again. It does **not**, and
structurally cannot, check the model `label` row: that is a semantic claim about
upstream state, not a version string; see
`SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md` section 3.3 for the open question of
making that check less manual. Both pins must still be updated together, and the test
only catches a *mismatch*, not a location someone forgot to touch at all.

An existing container keeps the version it installed until its marker changes: a bump
changes the install marker, so the next start of each container installs the new
version into its volume. A container whose image still has a baked-in `claude` (the
retired `agent-claude` image) keeps that one.

## How to bump

1. Check the latest release: `npm view @anthropic-ai/claude-code version`
2. In `crates/srv/src/backend/providers.rs`: update the CLAUDE static's `pinned_version`
3. In `frontend/app/view/agent/providers/catalog.ts`: update `PROVIDERS.claude.pinnedVersion`
4. Also in `catalog.ts`: re-check each model alias's curated `label`/`description` still
   matches what the pinned CLI currently resolves that alias to (e.g. `opus` -> "Opus 5")
   — a label can go stale even when the alias `value` itself never changes.
5. Run `pin-consistency.test.ts` to confirm the two version pins agree (it does not
   check the label from step 4; that one's on you).
6. Open a PR and merge it. No image needs publishing: container agents install the new
   pin on their next start.

## Publishing the base image

The base image only changes when its contents do (system packages, the `agentmux-mcp`
and `agentmux-bashwrap` binaries). Either:
- **Push a `v*` git tag**: publishes the semver tags and `:latest`.
- **Manually dispatch** the `Container Agent Image` workflow: pushes a `dispatch-<sha>`
  tag, and also `:latest` when the `tag_latest` input is on.

The workflow's last step pulls the image anonymously and fails, naming the UI step, if
the package is not public. A new package is private until an organization owner changes
its visibility in the GitHub UI (there is no API for it).

## Version history

| Version | Date | Notes |
|---------|------|-------|
| `2.1.197` | 2026-06-30 | First explicit pin; replaced floating `latest` default |
| `2.1.198` | 2026-07-02 | Bump; initially missed the cef host installer and workflow default (see `pin-consistency.test.ts` history note) |
| `2.1.247` | 2026-08-27 | Bump (verified via `npm view @anthropic-ai/claude-code version` against the real registry); paired with relabeling the `opus` alias from "Opus 4.8" to "Opus 5" in the UI catalog. First bump done against a written checklist (this doc) rather than tribal knowledge — found this doc's own frontend file path had drifted (`index.ts` → `catalog.ts`) and that the Dockerfile `ARG` (the 5th matching-version-string pin, distinct from the model label's separate, non-string check below) wasn't covered by `pin-consistency.test.ts`; both corrected here, and the test extended to cover the Dockerfile going forward. This paragraph and the "all N locations" prose above it took three separate review-flagged edits in this same cycle to get precise — see the retro's addendum for the honest accounting. Full retro: `docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md`. Forward-looking process: `docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`. |
| `2.1.280` | 2026-09-22 | Bump (verified via `npm view @anthropic-ai/claude-code version` against the real registry); paired with relabeling the `opus` alias from "Opus 5" to "Opus 5.5" following Anthropic's same-day release of Claude Opus 5.5. Considered adding Opus 5.5 as a second, independently-selectable entry alongside Opus 5 instead, but `model-overlay.ts`'s family-grouping (`familyKey()` strips digits, so `opus`/`claude-opus-5`/`claude-opus-5-5` are all one family) always collapses a family's curated row(s) to whichever the live API reports as newest — the same one-row-per-family behavior already enforced for Fable by `model-overlay.test.ts`. Keeping both rows independently selectable long-term would need a real change to that grouping logic (no precedent in this repo); relabeling in place instead matches the established pattern exactly and required no code beyond this bump. |
| `2.1.285` | 2026-09-30 | Bump (verified via `npm view @anthropic-ai/claude-code version`, and by downloading the `@anthropic-ai/claude-code-win32-x64@2.1.285` binary directly and confirming its embedded model catalog — dated `2026-09-29T01:02:40Z` — lists `claude-sonnet-5-5`); paired with relabeling the `sonnet` alias from "Sonnet 5" to "Sonnet 5.5" following Anthropic's 2026-09-29 release of Claude Sonnet 5.5. Prompted by a live incident where an agent's pane showed "Sonnet 5.5 · high" while running Opus 5.5 at medium — see `docs/retro/RETRO_RESUMED_AGENT_SPAWNS_WITHOUT_RUNTIME_FLAGS_2026_09_30.md` (that mismatch's own cause, a missing `--model`/`--effort` at spawn, is unrelated to this bump) and `docs/reports/REPORT_AGENT_RUNTIME_STATE_RECONCILIATION_2026_09_30.md`. Separately, once that pane's own model/effort selection was correctly applied (`--model sonnet --effort xhigh` confirmed in the spawned process's command line), the CLI's `sonnet` alias on `2.1.280` still resolved to `claude-sonnet-5`, not `claude-sonnet-5-5` — confirmed by the harness's own live model self-report (`claude-sonnet-5`), the actual evidence this relabel and version bump rest on. `opus`/`haiku`/`fable` ids and labels unchanged (all three ids still present verbatim in the new binary). |
| `2.1.287` | 2026-10-01 | Bump (verified via `npm view @anthropic-ai/claude-code version`; every flag and auth subcommand AgentMux passes was compared between the 2.1.285 and 2.1.287 `--help`, and the only removal is `--client-data-url`, which AgentMux doesn't use; the `@anthropic-ai/claude-code-win32-x64@2.1.287` binary still contains `claude-opus-5-5`, `claude-sonnet-5-5`, `claude-haiku-4-5` and `claude-fable-5-1`, so no catalog relabel). Driven by the 2026-10-01 drift report — `docs/specs/SPEC_VERSION_DRIFT_UPGRADES_AND_PROVIDER_HARNESS_TESTS_2026_10_01.md` phase 1a. |
| `2.1.288` | 2026-10-03 | Bump from the 2026-10-03 drift report (verified via `npm view @anthropic-ai/claude-code version`). Installed 2.1.287 and 2.1.288 side by side and diffed `--help` and `auth --help`: no flag added or removed; the only change is a new `purge` subcommand, which AgentMux doesn't use. Engines unchanged (`node >=22.0.0`). |

## Escape hatch

To build with a one-off version without changing the pins, trigger the
`Container Agent Image` workflow manually and enter the version in the
`claude_version` input field. This produces a `dispatch-<sha>` image tag.

## Why `DISABLE_AUTOUPDATER=1`

The Dockerfile sets `DISABLE_AUTOUPDATER=1` and `NO_UPDATE_NOTIFIER=1`. Version
management is done at image build time only. In-container auto-updates are
explicitly disabled so that a running agent's Claude Code version matches what the
image tag advertises.
