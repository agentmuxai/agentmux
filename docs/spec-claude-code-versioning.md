# Spec: Claude Code Version Management

**Status:** Active  
**Current pinned version:** `2.1.247`  
**Previous default:** `latest` (floating)

## Problem

The container image for `ghcr.io/agentmuxai/agent-claude` previously defaulted to
installing whatever `@anthropic-ai/claude-code@latest` resolved to at image build
time. This meant two builds triggered minutes apart could embed different Claude Code
versions, breaking reproducibility and making regressions harder to bisect.

## Version pins (six locations)

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

`frontend/app/view/agent/providers/pin-consistency.test.ts` enforces agreement across
the last three (frontend, srv, cef) plus the workflow default — it does **not**
cover the Dockerfile `ARG`, so that one location can still drift silently; double
check it by hand when bumping. All six must be updated together. (This gap —
plus this doc itself having drifted on the frontend file path — is exactly the
kind of thing `SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md` generalizes a fix
for: a durable checklist/script instead of a doc a human has to remember to
re-read accurately.)

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
8. Run `pin-consistency.test.ts` to confirm 2-6 agree (it won't catch the Dockerfile —
   diff it by hand).
9. Open a PR and merge it.
10. To publish the image, either:
    - **Push a `v*` git tag** (e.g. `git tag v0.50.0 && git push origin v0.50.0`) — this triggers the workflow automatically and publishes both the semver tag and `:latest`.
    - **Manually dispatch** the `Container Agent Image` workflow — this builds and pushes a `dispatch-<sha>` tag only; `:latest` is *not* updated by a manual dispatch.

## Version history

| Version | Date | Notes |
|---------|------|-------|
| `2.1.197` | 2026-06-30 | First explicit pin; replaced floating `latest` default |
| `2.1.198` | 2026-07-02 | Bump; initially missed the cef host installer and workflow default (see `pin-consistency.test.ts` history note) |
| `2.1.247` | 2026-08-27 | Bump (verified via `npm view @anthropic-ai/claude-code version` against the real registry); paired with relabeling the `opus` alias from "Opus 4.8" to "Opus 5" in the UI catalog. First bump done against a written checklist (this doc) rather than tribal knowledge — found this doc's own frontend file path had drifted (`index.ts` → `catalog.ts`) and that the Dockerfile `ARG` (a 6th pin location) isn't covered by `pin-consistency.test.ts`; both corrected here. Full retro: `docs/retro/retro-claude-cli-and-opus-5-upgrade-2026-08-27.md`. Forward-looking process: `docs/specs/SPEC_DEPENDENCY_UPGRADE_PROCESS_2026_08_27.md`. |

## Escape hatch

To build with a one-off version without changing the pins, trigger the
`Container Agent Image` workflow manually and enter the version in the
`claude_version` input field. This produces a `dispatch-<sha>` image tag.

## Why `DISABLE_AUTOUPDATER=1`

The Dockerfile sets `DISABLE_AUTOUPDATER=1` and `NO_UPDATE_NOTIFIER=1`. Version
management is done at image build time only. In-container auto-updates are
explicitly disabled so that a running agent's Claude Code version matches what the
image tag advertises.
