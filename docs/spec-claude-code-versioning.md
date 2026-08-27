# Spec: Claude Code Version Management

**Status:** Active  
**Current pinned version:** `2.1.247`  
**Previous default:** `latest` (floating)

## Problem

The container image for `ghcr.io/agentmuxai/agent-claude` previously defaulted to
installing whatever `@anthropic-ai/claude-code@latest` resolved to at image build
time. This meant two builds triggered minutes apart could embed different Claude Code
versions, breaking reproducibility and making regressions harder to bisect.

## Version pins (five matching-string locations, plus one curated label — six sync points total)

| File | Location | Purpose |
|------|----------|---------|
| `docker/Dockerfile.agent-agentmux` line 36 | `ARG CLAUDE_VERSION=2.1.247` | Fallback for local `docker build` without passing the arg |
| `.github/workflows/container-image.yml` line 16 | `default: '2.1.247'` | Default used when CI is triggered via `workflow_dispatch` without an explicit version input |
| `agentmux-srv/src/backend/providers.rs` | `pinned_version: "2.1.247"` (CLAUDE static) | Version the backend sidecar installs |
| `agentmux-cef/src/commands/providers.rs` | `const CLAUDE_VERSION: &str = "2.1.247"` | Version the host installer installs |
| `frontend/app/view/agent/providers/catalog.ts` (re-exported via `./index`) | `pinnedVersion: "2.1.247"` (PROVIDERS.claude) | Version surfaced in the UI. Corrected 2026-08-27 — this file used to be a single `providers/index.ts`, split into `types.ts`/`catalog.ts`/`model-overlay.ts` for readability; the pin moved with it but this doc wasn't updated at the time. |
| `frontend/app/view/agent/providers/catalog.ts` (same object) | `models: [{ value: "opus", label: "Opus 5", ... }]` | The curated UI label for the `opus` family alias — **not itself version-locked to the CLI pin**, but should be re-checked on every pin bump per the field's own doc comment ("kept in sync on a pin bump"): whichever concrete snapshot Anthropic's API currently resolves `--model opus` to. |

The CI workflow's "Resolve Claude Code version" step (`id: claude_ver`) has a special case:
- Input non-empty and not `"latest"` → use the input value verbatim (shell injection safe via `env:`)
- Input empty or `"latest"` → resolve via `npm view @anthropic-ai/claude-code version` at build time

`frontend/app/view/agent/providers/pin-consistency.test.ts` enforces agreement
across the **five matching-version-string** locations (the first five rows
above), including the Dockerfile `ARG` — added 2026-08-27, closing a gap this
doc itself had warned about (in this same paragraph) for over a month without
it becoming a test. It does **not**, and structurally cannot, check the sixth
row (the model `label`) — that's not a version string to compare, it's a
semantic claim about upstream state; see `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`
§3.3 for the open question of whether/how to make that check less manual too.
All five version pins must still be updated together, and the test only
catches a *mismatch* — not a location someone forgot to touch at all. (That
drift-in-a-warning — plus this doc having separately drifted on the frontend
file path, plus this exact paragraph ALSO originally mis-stated "all six" as
if the label were part of the same check when it was first corrected — is
exactly the kind of thing `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`
generalizes a fix for: prefer a durable check over a doc a human has to
remember to re-read accurately, since even three successive edits to this
same paragraph, by the same author, in the same review cycle, kept
introducing a version of the same imprecision.)

## How to bump

1. Check the latest release: `npm view @anthropic-ai/claude-code version`
2. In `docker/Dockerfile.agent-agentmux`: update `ARG CLAUDE_VERSION=<new>`
3. In `.github/workflows/container-image.yml`: update `default: '<new>'`
4. In `agentmux-srv/src/backend/providers.rs`: update the CLAUDE static's `pinned_version`
5. In `agentmux-cef/src/commands/providers.rs`: update `CLAUDE_VERSION`
6. In `frontend/app/view/agent/providers/catalog.ts`: update `PROVIDERS.claude.pinnedVersion`
7. Also in `catalog.ts`: re-check each model alias's curated `label`/`description` still
   matches what the pinned CLI currently resolves that alias to (e.g. `opus` → "Opus 5")
   — a label can go stale even when the alias `value` itself never changes.
8. Run `pin-consistency.test.ts` to confirm the five version pins agree
   (includes the Dockerfile `ARG` as of 2026-08-27; does not check the label
   from step 7 — that one's on you).
9. Open a PR and merge it.
10. To publish the image, either:
    - **Push a `v*` git tag** (e.g. `git tag v0.50.0 && git push origin v0.50.0`) — this triggers the workflow automatically and publishes both the semver tag and `:latest`.
    - **Manually dispatch** the `Container Agent Image` workflow — this builds and pushes a `dispatch-<sha>` tag only; `:latest` is *not* updated by a manual dispatch.

## Version history

| Version | Date | Notes |
|---------|------|-------|
| `2.1.197` | 2026-06-30 | First explicit pin; replaced floating `latest` default |
| `2.1.198` | 2026-07-02 | Bump; initially missed the cef host installer and workflow default (see `pin-consistency.test.ts` history note) |
| `2.1.247` | 2026-08-27 | Bump (verified via `npm view @anthropic-ai/claude-code version` against the real registry); paired with relabeling the `opus` alias from "Opus 4.8" to "Opus 5" in the UI catalog. First bump done against a written checklist (this doc) rather than tribal knowledge — found this doc's own frontend file path had drifted (`index.ts` → `catalog.ts`) and that the Dockerfile `ARG` (the 5th matching-version-string pin, distinct from the model label's separate, non-string check below) wasn't covered by `pin-consistency.test.ts`; both corrected here, and the test extended to cover the Dockerfile going forward. This paragraph and the "all N locations" prose above it took three separate review-flagged edits in this same cycle to get precise — see the retro's addendum for the honest accounting. Full retro: `docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md`. Forward-looking process: `docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`. |

## Escape hatch

To build with a one-off version without changing the pins, trigger the
`Container Agent Image` workflow manually and enter the version in the
`claude_version` input field. This produces a `dispatch-<sha>` image tag.

## Why `DISABLE_AUTOUPDATER=1`

The Dockerfile sets `DISABLE_AUTOUPDATER=1` and `NO_UPDATE_NOTIFIER=1`. Version
management is done at image build time only. In-container auto-updates are
explicitly disabled so that a running agent's Claude Code version matches what the
image tag advertises.
