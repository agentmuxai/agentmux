# Spec: Claude Code Version Management

**Status:** Active  
**Current pinned version:** `2.1.198`  
**Previous default:** `latest` (floating)

## Problem

The container image for `ghcr.io/agentmuxai/agent-claude` previously defaulted to
installing whatever `@anthropic-ai/claude-code@latest` resolved to at image build
time. This meant two builds triggered minutes apart could embed different Claude Code
versions, breaking reproducibility and making regressions harder to bisect.

## Version pins (five locations)

| File | Location | Purpose |
|------|----------|---------|
| `docker/Dockerfile.agent-agentmux` line 36 | `ARG CLAUDE_VERSION=2.1.198` | Fallback for local `docker build` without passing the arg |
| `.github/workflows/container-image.yml` line 16 | `default: '2.1.198'` | Default used when CI is triggered via `workflow_dispatch` without an explicit version input |
| `agentmux-srv/src/backend/providers.rs` | `pinned_version: "2.1.198"` (CLAUDE static) | Version the backend sidecar installs |
| `agentmux-cef/src/commands/providers.rs` | `const CLAUDE_VERSION: &str = "2.1.198"` | Version the host installer installs |
| `frontend/app/view/agent/providers/index.ts` | `pinnedVersion: "2.1.198"` (PROVIDERS.claude) | Version surfaced in the UI |

The CI workflow's "Resolve Claude Code version" step (`id: claude_ver`) has a special case:
- Input non-empty and not `"latest"` → use the input value verbatim (shell injection safe via `env:`)
- Input empty or `"latest"` → resolve via `npm view @anthropic-ai/claude-code version` at build time

`frontend/app/view/agent/providers/pin-consistency.test.ts` enforces agreement across
the last three (frontend, srv, cef) plus the workflow default — it does **not**
cover the Dockerfile `ARG`, so that one location can still drift silently; double
check it by hand when bumping. All five must be updated together.

## How to bump

1. Check the latest release: `npm view @anthropic-ai/claude-code version`
2. In `docker/Dockerfile.agent-agentmux`: update `ARG CLAUDE_VERSION=<new>`
3. In `.github/workflows/container-image.yml`: update `default: '<new>'`
4. In `agentmux-srv/src/backend/providers.rs`: update the CLAUDE static's `pinned_version`
5. In `agentmux-cef/src/commands/providers.rs`: update `CLAUDE_VERSION`
6. In `frontend/app/view/agent/providers/index.ts`: update `PROVIDERS.claude.pinnedVersion`
7. Run `pin-consistency.test.ts` to confirm 2-6 agree (it won't catch the Dockerfile).
8. Open a PR and merge it.
9. To publish the image, either:
   - **Push a `v*` git tag** (e.g. `git tag v0.50.0 && git push origin v0.50.0`) — this triggers the workflow automatically and publishes both the semver tag and `:latest`.
   - **Manually dispatch** the `Container Agent Image` workflow — this builds and pushes a `dispatch-<sha>` tag only; `:latest` is *not* updated by a manual dispatch.

## Version history

| Version | Date | Notes |
|---------|------|-------|
| `2.1.197` | 2026-06-30 | First explicit pin; replaced floating `latest` default |
| `2.1.198` | 2026-07-02 | Bump; initially missed the cef host installer and workflow default (see `pin-consistency.test.ts` history note) |

## Escape hatch

To build with a one-off version without changing the pins, trigger the
`Container Agent Image` workflow manually and enter the version in the
`claude_version` input field. This produces a `dispatch-<sha>` image tag.

## Why `DISABLE_AUTOUPDATER=1`

The Dockerfile sets `DISABLE_AUTOUPDATER=1` and `NO_UPDATE_NOTIFIER=1`. Version
management is done at image build time only. In-container auto-updates are
explicitly disabled so that a running agent's Claude Code version matches what the
image tag advertises.
