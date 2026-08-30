# Status: GitHub Issue/Discussion Cleanup Pass (2026-07-24)

> **HISTORICAL as of 2026-08-29 (docs-cleanup Phase 3).** This is the record
> of a completed one-time triage, not a plan with open work — it should be
> read as "what was true on 2026-07-24," which §5 already asks of the reader.
>
> **Its one outstanding action item is now closed:** PR **#2291**
> (`fix(term): drop await on init resync to stop N-pane spawn
> serialization`) — flagged here as "still needs human review/merge" — was
> **merged 2026-07-25**, the day after this pass.
>
> **Its headline numbers have since drifted, as §5 predicted they would:**
> this pass ended at 25 open issues; there are **40** open today. That is
> expected drift from five weeks of normal work, not a regression against
> the cleanup — and §5's own recommendation ("re-triage periodically — the
> last general cleanup appears to have been ad hoc, not scheduled") still
> stands as the live follow-up.

A full triage of every open issue and discussion in `agentmuxai/agentmux`, done at the user's
request ("clean up our open issues/discussions"). 5 parallel agents read every item's full body
+ comments and cross-checked it against current repo state (code, CLAUDE.md, merged PRs, git
log) before classifying — nothing was closed/merged on title alone.

**Starting state:** 36 open issues + 13 open discussions = 49 items.
**Ending state:** 25 open issues + 8 open discussions = 33 items open, 16 closed (11 issues + 5
discussions, across §1's 13 and §2's 3), 1 code fix implemented with a PR open (§3, not yet
merged — see §5).

## 1. Closed as stale / already shipped / superseded (13)

Each closed with an explanatory comment citing the specific evidence (merged PR, shipped code,
or a maintainer decision already on record) — see the issue/discussion for the full comment.

| # | Title | Evidence |
|---|---|---|
| [#2218](https://github.com/agentmuxai/agentmux/issues/2218) | tracking: renderer pool commit-charge growth | All 3 fix PRs (#2220/#2221/#2222) merged 2026-07-18, shipped in v0.54.4 |
| [#2092](https://github.com/agentmuxai/agentmux/issues/2092) | Launcher-side teardown backstop | Phase 2 shipped 2026-07-16 per `SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11.md` |
| [#1718](https://github.com/agentmuxai/agentmux/issues/1718) | Nightly cross-platform CI builds | Both workflow files exist, cite this issue directly |
| [#1675](https://github.com/agentmuxai/agentmux/issues/1675) | "Please stop tagging me" | One-off bot-etiquette nudge on a PR thread, not a code issue |
| [#1207](https://github.com/agentmuxai/agentmux/issues/1207) | Obsolete Tauri/Electron docs cleanup | Docs rewritten, `package:macos` shipped; residual is out-of-repo/account-level |
| [#870](https://github.com/agentmuxai/agentmux/issues/870) | dev:serve TOCTOU race | Deterministic per-clone port + reap-and-fail-fast guard shipped |
| [#778](https://github.com/agentmuxai/agentmux/issues/778) | GPU process crashes 3× at startup | `--disable-gpu` retry ladder + SwiftShader fallback shipped |
| [#102](https://github.com/agentmuxai/agentmux/issues/102) | feat: OpenClaw platform | Absorbed as an ACP provider instead; standalone-product framing superseded |
| [#101](https://github.com/agentmuxai/agentmux/issues/101) | feat: Swarm Orchestration | "Swarm" name now belongs to a shipped, different-purpose observability widget; maintainer agreed in prior triage |
| [Discussion #2020](https://github.com/agentmuxai/agentmux/discussions/2020) | Messenger integrations status | 4/5 platforms shipped; Teams deliberately deferred by design |
| [Discussion #1707](https://github.com/agentmuxai/agentmux/discussions/1707) | Custom MCP tooling | Author explicitly shelved it |
| [Discussion #874](https://github.com/agentmuxai/agentmux/discussions/874) | Architecture comparison (VS Code/Electron) | Purely informational research post, no open question |
| [Discussion #707](https://github.com/agentmuxai/agentmux/discussions/707) | Reducer-stack architecture tracking | At its own declared "practical end-state"; residual issue #864 already closed |

## 2. Merged into a canonical thread (3)

Closed with a comment pointing to the surviving thread, rather than losing the history —
GitHub keeps the closed item's discussion intact and cross-linked.

| Closed | Merged into | Why |
|---|---|---|
| [#1681](https://github.com/agentmuxai/agentmux/issues/1681) Floating-pane DnD lifecycle rethink | [#1205](https://github.com/agentmuxai/agentmux/discussions/1205) | Only R2 (floater→CEF-Views conversion) remained, already tracked in #1205 |
| [#1461](https://github.com/agentmuxai/agentmux/issues/1461) Browser-pane redock: black page | [#1205](https://github.com/agentmuxai/agentmux/discussions/1205) | Superseded by #1205's deeper 3-race diagnosis and structural §9 fix plan |
| [Discussion #943](https://github.com/agentmuxai/agentmux/discussions/943) Service Supervision (Ideas) | [#942](https://github.com/agentmuxai/agentmux/issues/942) | Split thread — real status updates reverted to and continued on #942 |

## 3. Easy fix — implemented, PR open (1)

**[#121](https://github.com/agentmuxai/agentmux/issues/121)** — perf: PTY spawn serializes on
concurrent pane creation (opening N terminal panes stalled the UI by N×~10ms).

- **Fix A implemented and PR'd**: [PR #2291](https://github.com/agentmuxai/agentmux/pull/2291)
  drops the `await` on `resyncController("init")` in `frontend/app/view/term/termwrap.ts`,
  reordering `hasResized = true` ahead of it per the issue's own review-comment guidance (avoids
  missing a reconnect event during the fire-and-forget window). Verified with `tsc --noEmit` —
  no new type errors. This removes the frontend-side serialization, which per the issue's own
  root-cause analysis is the dominant contributor (N×10ms → ~10ms regardless of N).
- **Fix B intentionally NOT done in this pass**: wrapping the backend's `spawn_command`
  (fork+exec) in `tokio::task::spawn_blocking`. On inspection this touches 3 separate async RPC
  call sites (`websocket.rs:794`, `agent_open.rs:103`, `agent_open.rs:421`) with Send/`'static`
  implications across the PTY-spawn hot path — real work, more than the issue's own "~20 lines"
  estimate suggested, and not something to land without dedicated review/testing. Left #121 open
  with a comment scoping it down to Fix B (+ Fix C, opt-in `--norc`) only.
- **Lesson for future easy-fix passes**: the survey agent flagged this as EASY_FIX based on the
  issue's own comments describing a small diff; actually reading the current call graph revealed
  the "easy" label applied to only half the proposed work. Verify diff scope against live code,
  not just issue-comment estimates, before committing to "easy."

## 4. Hard/pending — verified still valid, left open (32)

Everything below was independently re-verified against current code/docs (not just "still open
therefore still valid") — every one of these has either live recent discussion or was directly
confirmed unimplemented via grep/code-read. Grouped by theme for orientation; full per-item
evidence lives in this session's agent transcripts if anyone needs to re-derive it.

**Permission / tool-approval UI chain** (sequential, not duplicates):
[#551](https://github.com/agentmuxai/agentmux/issues/551) decision-prompt UI (scaffolding shipped, not wired — `parsePermissionRequest()` is a stub) →
[#1469](https://github.com/agentmuxai/agentmux/issues/1469) Phase 2 routing (`can_use_tool` currently auto-allows everything).

**Cross-platform lifecycle verification** (3 platform-scoped issues, kept separate — different windowing backends, real independent risk):
[#2190](https://github.com/agentmuxai/agentmux/issues/2190) Ubuntu auth blocker,
[#2189](https://github.com/agentmuxai/agentmux/issues/2189) Linux window-close/quit,
[#2188](https://github.com/agentmuxai/agentmux/issues/2188) macOS window-close/quit.

**CLI-specific permission/auth bugs**:
[#1250](https://github.com/agentmuxai/agentmux/issues/1250) gemini pane never authenticates (fake auth commands still in `catalog.ts`),
[#1247](https://github.com/agentmuxai/agentmux/issues/1247) codex ignores non-bypass permission modes (`buildRuntimeArgs.ts` confirmed still hard-skipping).

**Identity/OAuth/Armory**:
[#2115](https://github.com/agentmuxai/agentmux/issues/2115) OAuth client-ID provisioning (blocked on non-code ops work),
[#2024](https://github.com/agentmuxai/agentmux/issues/2024) Armory Brain/Bundle tab decision + Bundle v2 (Identity fold done, 2 items remain, canonical Armory tracker).

**Floating panes / browser / editor lifecycle**:
[#768](https://github.com/agentmuxai/agentmux/issues/768) phantom browser pane (tracked separately from #1205 by design),
[#2155](https://github.com/agentmuxai/agentmux/issues/2155) editor rename-input caret/focus (half fixed by PR #2268 yesterday, caret bug remains),
[#1190](https://github.com/agentmuxai/agentmux/issues/1190) browser-pane keyboard shortcuts (frontend done, CEF/Rust side unimplemented),
[#871](https://github.com/agentmuxai/agentmux/issues/871) opacity drag lag + HWND TOCTOU (part 2 still present, touches a file with a known regression history — not conservatively easy),
[Discussion #1205](https://github.com/agentmuxai/agentmux/discussions/1205) floating-pane tracking (the canonical thread above two items merged into).

**Memory/typing perf/misc trackers**:
[#1569](https://github.com/agentmuxai/agentmux/issues/1569) macOS/Linux orphan reconciler crash-orphan detection,
[#950](https://github.com/agentmuxai/agentmux/issues/950) bulletproof terminals (G1/G5 + `term.type` API still missing),
[Discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161) typing-performance umbrella,
[Discussion #1413](https://github.com/agentmuxai/agentmux/discussions/1413) Memory & Identity tracking (Identity half resolved, Memory half live),
[Discussion #493](https://github.com/agentmuxai/agentmux/discussions/493) how AI CLIs load context files (research done, `AGENTS.md` dual-write action item unshipped).

**Architecture tracking**:
[#1549](https://github.com/agentmuxai/agentmux/issues/1549) architecture refactor A1-A15 (12/15 shipped, A6/A9 remain),
[#942](https://github.com/agentmuxai/agentmux/issues/942) service supervision & recovery (Phases 0-2 shipped, 3-4 + non-Windows remain; now the sole merge target after #943 folded in),
[#1814](https://github.com/agentmuxai/agentmux/issues/1814) agents managing long-running commands (most MCP tooling shipped; PTY `\r` rendering + cleanup wiring remain),
[#1400](https://github.com/agentmuxai/agentmux/issues/1400) container agents Phase 3 (all 3 gaps confirmed still open),
[#1916](https://github.com/agentmuxai/agentmux/issues/1916) MuxBus cross-channel delivery (spec drafted, unimplemented),
[#851](https://github.com/agentmuxai/agentmux/issues/851) Wave*→Mux* rename (deliberately deferred after a cost/benefit review),
[Discussion #1095](https://github.com/agentmuxai/agentmux/discussions/1095) agent data-model consolidation (Phase 3c/R/O unstarted, concrete plan exists),
[Discussion #1033](https://github.com/agentmuxai/agentmux/discussions/1033) Warden Widget RFC (L1/L2 shipped, L3 + governance design open),
[Discussion #1026](https://github.com/agentmuxai/agentmux/discussions/1026) per-version data isolation (Increments A/B shipped, C — import wizard — remains),
[Discussion #857](https://github.com/agentmuxai/agentmux/discussions/857) multi-agent version coordination RFC (Phases 0-3 shipped, 4-5 automation remain).

**Standalone bugs/features, no cluster**:
[#2215](https://github.com/agentmuxai/agentmux/issues/2215) bashwrap idle-timeout false-positive on redirected output (needs a design pick among 3 proposed directions),
[#1591](https://github.com/agentmuxai/agentmux/issues/1591) server-side speech-to-text (escalated in priority — Web Speech API confirmed non-viable in CEF),
[#261](https://github.com/agentmuxai/agentmux/issues/261) Computer Use pane (greenfield feature, no code exists yet).

## 5. Notes for whoever picks this up next

- The 5 survey agents' full per-item evidence (including things ruled *against* closing/merging)
  lives in this session's transcript if a more detailed rationale is ever needed for a specific
  item than what's summarized above.
- Nothing above should be treated as permanently settled — this is a snapshot as of 2026-07-24.
  Re-triage periodically (the last general cleanup appears to have been ad hoc, not scheduled).
- PR #2291 (the #121 partial fix) still needs human review/merge — nothing in this pass bypassed
  branch protection.
