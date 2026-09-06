# Provider CLI version upgrade (2026-09-06 drift report)

**Status:** proposed → implementing

## 1. Source

`a5af/shared-infrastructure`'s new `provider-reporter` Lambda (PR #458,
`shared-infrastructure/provider-reporter/`) ran its scheduled drift check
overnight and archived the result at
`s3://infrastructure-provider-reporter-reports/reports/agentmux/2026-09-06.json`
(`generated_at: 2026-09-06T13:15:32Z`). It compares each provider's
`pinnedVersion` in AgentMux's `catalog.ts` against the latest version
published on npm.

## 2. Findings

| Provider | Package | Pinned | Latest | Action |
|---|---|---|---|---|
| claude | `@anthropic-ai/claude-code` | 2.1.247 | 2.1.263 | **Bump** |
| codex | `@openai/codex` | 0.116.0 | 0.153.4 | **Bump** |
| gemini | `@google/gemini-cli` | 0.32.1 | 0.58.0 | **Bump** |
| qwen | `@qwen-code/qwen-code` | 0.19.2 | 0.23.0 | **Bump** |
| openclaw | `openclaw` | 2026.6.10 | 2026.9.2 | **Bump** |
| copilot | `@github/copilot` | 1.0.65 | 1.0.83 | **Bump** |
| pi | `@mariozechner/pi-coding-agent` | 0.73.1 | 0.73.1 | none — current |
| kimi | (none, pip-based) | — | — | none — intentionally unmonitored |
| muxcode | `@agentmuxai/muxcode` | 0.1.0 | lookup-failed | **not a real drift** — see §3 |
| antigravity | `@google/antigravity-cli` | 1.0.0 | lookup-failed | **not a real drift** — see §3 |

Model catalog section of the report: all four curated Claude rows
(`opus`/`sonnet`/`haiku`/`claude-fable-5-1`) are `current` against the
Anthropic-authoritative source. No model catalog changes needed.

## 3. `lookup-failed` is expected here, not a defect

Verified directly against the npm registry: both
`https://registry.npmjs.org/@agentmuxai/muxcode` and
`https://registry.npmjs.org/@google/antigravity-cli` return **404** — the
packages are genuinely unpublished (muxcode is AgentMux's own first-party
CLI, not yet released to npm; antigravity is Google's harness, not yet
public on npm either). The reporter's `lookup-failed` severity (`warn`)
already exists precisely to distinguish this from a registry error rather
than silently reporting "no drift" (see `provider-reporter/README.md`
"Known limitations"). There is nothing to bump for either provider — their
`pinnedVersion` stays as-is until the package actually publishes.

## 4. Fix — bump six pins across all locations the pin-consistency test enforces

Per `frontend/app/view/agent/providers/pin-consistency.test.ts`'s header
comment, the **claude** pin is duplicated across five locations that must
agree; **codex** and **gemini** across three (no container/Dockerfile
copy — those are Claude-specific because the container image is a Claude
agent image); **qwen**, **openclaw**, and **copilot** are not covered by
that test at all today (the cef host installer's `get_pinned_version` only
handles claude/codex/gemini) — their pin lives in exactly two places
(`catalog.ts` + `agentmux-srv/providers.rs`), both updated here for
consistency even though nothing currently machine-checks them.

| Location | claude | codex | gemini | qwen | openclaw | copilot |
|---|---|---|---|---|---|---|
| `frontend/app/view/agent/providers/catalog.ts` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `agentmux-srv/src/backend/providers.rs` | ✅ | ✅ | ✅ | ✅ | ✅ | ✅ |
| `agentmux-cef/src/commands/providers.rs` | ✅ | ✅ | ✅ | — | — | — |
| `.github/workflows/container-image.yml` (`claude_version` default) | ✅ | — | — | — | — | — |
| `docker/Dockerfile.agent-agentmux` (`ARG CLAUDE_VERSION`) | ✅ | — | — | — | — | — |

New pins: claude `2.1.263`, codex `0.153.4`, gemini `0.58.0`, qwen `0.23.0`,
openclaw `2026.9.2`, copilot `1.0.83`.

## 5. Non-goals

- No change to `muxcode`/`antigravity`/`kimi`/`pi` pins (§3, and pi/kimi
  need no change per §2).
- No model catalog changes (all current per §2).
- Not fixing `provider-reporter`'s own known limitation that `catalog.ts`
  is scraped rather than consumed as a generated contract — tracked
  separately in `shared-infrastructure/specs/SPEC_REPORTER_PLATFORM_CONSOLIDATION_2026_09_05.md`,
  out of scope for this repo.

## 6. Testing

- `npx vitest run frontend/app/view/agent/providers/pin-consistency.test.ts` —
  the existing drift guard must pass with the new claude/codex/gemini pins.
- `npx tsc --noEmit -p .`
- `cargo check -p agentmux-srv -p agentmux-cef` (or `task build:backend`) to
  confirm the Rust pin literals compile.
- Manual: none required — pin bumps don't change install/launch behavior
  beyond which concrete version string is passed to `npm install -g
  <pkg>@<version>`.
