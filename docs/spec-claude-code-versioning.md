# Spec: Claude Code Version Management

**Status:** Active  
**Current pinned version:** `2.1.197`  
**Previous default:** `latest` (floating)

## Problem

The container image for `ghcr.io/agentmuxai/agent-claude` previously defaulted to
installing whatever `@anthropic-ai/claude-code@latest` resolved to at image build
time. This meant two builds triggered minutes apart could embed different Claude Code
versions, breaking reproducibility and making regressions harder to bisect.

## Version pins (two locations)

| File | Location | Purpose |
|------|----------|---------|
| `docker/Dockerfile.agent-agentmux` line 36 | `ARG CLAUDE_VERSION=2.1.197` | Fallback for local `docker build` without passing the arg |
| `.github/workflows/container-image.yml` line 12 | `default: '2.1.197'` | Default used when CI is triggered via `workflow_dispatch` without an explicit version input |

The CI workflow's "Resolve Claude Code version" step (`id: claude_ver`) has a special case:
- Input non-empty and not `"latest"` → use the input value verbatim (shell injection safe via `env:`)
- Input empty or `"latest"` → resolve via `npm view @anthropic-ai/claude-code version` at build time

Both pins must be updated together when bumping.

## How to bump

1. Check the latest release: `npm view @anthropic-ai/claude-code version`
2. In `docker/Dockerfile.agent-agentmux`: update `ARG CLAUDE_VERSION=<new>`
3. In `.github/workflows/container-image.yml`: update `default: '<new>'`
4. Open a PR; the CI workflow's `workflow_dispatch` on merge will build and push
   the image tagged with the release version and `:latest`.

## Version history

| Version | Date | Notes |
|---------|------|-------|
| `2.1.197` | 2026-06-30 | First explicit pin; replaced floating `latest` default |

## Escape hatch

To build with a one-off version without changing the pins, trigger the
`Container Agent Image` workflow manually and enter the version in the
`claude_version` input field. This produces a `dispatch-<sha>` image tag.

## Why `DISABLE_AUTOUPDATER=1`

The Dockerfile sets `DISABLE_AUTOUPDATER=1` and `NO_UPDATE_NOTIFIER=1`. Version
management is done at image build time only. In-container auto-updates are
explicitly disabled so that a running agent's Claude Code version matches what the
image tag advertises.
