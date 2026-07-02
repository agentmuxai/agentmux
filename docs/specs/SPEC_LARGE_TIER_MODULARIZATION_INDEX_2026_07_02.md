# Large-Tier Modularization — Index & Sequencing

**Date:** 2026-07-02
**Context:** Follow-on to the critical-tier run (PRs #1880, #1881, #1887, #1888, #1889, #1891, #1893, #1897 — all merged). Those cleared every file >3,000 lines except the four below plus the already-done set. This batch targets the "Large" (1,500–3,000) tier's top offenders.

## The four targets

| File | Lines | Spec | Risk | Notes |
|------|-------|------|------|-------|
| `agentmux-srv/src/backend/agent_session.rs` | 2,801 | [agent_session](SPEC_MODULARIZE_AGENT_SESSION_2026_07_02.md) | **Low** | No trait impls, no platform cfg. Do first. |
| `agentmux-cef/src/client/mod.rs` | 2,377 | [cef_client](SPEC_MODULARIZE_CEF_CLIENT_2026_07_02.md) | **Medium** | Inherent impls split cleanly; watch struct field visibility. |
| `agentmux-srv/src/backend/blockcontroller/shell.rs` | 2,394 | [shell](SPEC_MODULARIZE_SHELL_2026_07_02.md) | **Medium** | Trait impl must stay whole; extract helper bodies only. Platform cfg + hot PTY path. |
| `agentmux-launcher/src/main.rs` | 2,264 | [launcher_main](SPEC_MODULARIZE_LAUNCHER_MAIN_2026_07_02.md) | **High** | Touches isolation invariants I1–I6. Pure move; explicit invariant statement in PR body; consider 2-PR split. |

## Recommended order

1. **agent_session.rs** (Low) — warm up, prove the pattern on a clean file.
2. **cef_client** (Medium) — isolated to `client/`, no invariant surface.
3. **shell.rs** (Medium) — trait-impl care; keep `impl Controller` intact.
4. **launcher/main.rs** (High) — last, most careful; invariant gate.

## Shared rules (from the critical-tier run's lessons)

- **Sequential only.** The agent workdir shares ONE git checkout (no worktree isolation) — run one branch-mutating agent at a time. Parallel branch work collided last time (see memory `shared-worktree-agent-collision`).
- **Agent1 identity for all git/gh:** prefix with `GH_TOKEN=<agent1 PAT>`; NEVER `gh auth login`.
- **No `#![allow(unused_imports)]`** — reagent flags leftover unused imports (P2 on #1880). Trim via `cargo check` output.
- **Inline-test symbol visibility** — when moving `#[cfg(test)] mod tests` to a `tests.rs`, ensure symbols it reaches via `use super::*` are re-exported or `pub(crate)`. reagent caught this as a P1 on #1880 (RecentSessionRow).
- **Preserve every `#[cfg]` guard byte-for-byte.** We compile-verify Windows locally; CI covers ubuntu/mac.
- **Changeset per PR:** `task changeset -- patch "refactor(...): ..."`. No version-file bumps.
- **PR body:** include `<!-- agentmux:agent_id=agent1 -->`. For launcher, add the explicit I1–I6 "no lifecycle/naming logic changed, only relocated" statement.

## Verification gate (every PR)

- `cargo check` + `cargo check --tests` (or `-p <crate>`) clean, zero new warnings
- Relevant `cargo test <module>` passes
- reagent review; expect only import/comment nits on a clean pure-reorg
