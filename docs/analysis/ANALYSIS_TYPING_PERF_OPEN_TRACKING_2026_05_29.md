# ANALYSIS — Open typing-perf tracking items + consolidation proposal

**Date:** 2026-05-29
**Owner:** Agent1
**Scope:** open Issues + Discussions in `agentmuxai/agentmux` related to **typing/keystroke responsiveness** in the terminal and agent panes. Adjacent perf concerns (focus latency, pane open serialization, opacity drag, etc.) are explicitly out of scope and listed separately at the bottom.

---

## TL;DR

Two open issues directly relate to typing perf:

| # | Title | Real status | Recommendation |
|---|---|---|---|
| **#782** | Agent pane virtualization redesign + intelligent perf probing | **Mostly shipped** — Phases 1–3 merged (PRs #783, #784, #787). Phase 4 (hardening) has no PR. | Close if Phase 4 work is folded into the new consolidated discussion; otherwise leave open and link |
| **#950** | Bulletproof terminals — `term.type` agent API + never-fail open guarantees | **Mostly NOT shipped** — only the immediate seq-buffer fix (PR #951) merged. `term.type`, open timeouts, retry, structured errors all pending. | Leave open as the implementation tracker for those items; link from the consolidated discussion as a child |

Two new docs from today add the umbrella context that didn't exist before:
- `SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` (PR #1151)
- `PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md` (same PR)

These give a forward-looking contract for "what does a fast input path look like" across both panes. The existing #782 + #950 are *implementation surface* under that umbrella. A consolidated discussion makes the relationship explicit and gives future readers one entry point instead of three.

**Proposed action:** open one Discussion (Ideas category) titled "Typing performance — terminal + agent pane umbrella" that pins the new SPEC + PLAN, summarizes #782 + #950 status, and tracks the two profiling-gated items from the plan (#6 + #7). Leave #782 + #950 open as referenced subtasks unless you want to close them outright in favor of the discussion.

---

## How I searched

1. `gh issue list --state open` → 38 open issues; greped title + body for `typing|keystroke|input.*lag|latency|composer|textarea|echo|stutter|jitter|sluggish|perf|slow|hang|freez|smooth|responsive`.
2. `gh api graphql` on Discussions → 11 open; same grep.
3. Cross-referenced PRs via `timelineItems` GraphQL to know which issues already had merged work.
4. Verified actual shipped state in main (`grep` for `term.type` in `agentmux-srv/src` — not present; spec files in `docs/specs/` — only stubs that copy the issue body).

---

## Items in scope (typing perf)

### Issue [#782 — Agent pane virtualization redesign + intelligent perf probing](https://github.com/agentmuxai/agentmux/issues/782)

**Filed:** 2026-05-10. 0 comments. No labels.

**What it proposes (from the body):**
- Per-kind size estimators (instead of one global `estimateSize: 80`)
- Hybrid render: last 50 nodes unvirtualized (streaming buffer)
- Scroll state in store, not DOM (`headAnchor` + `stickToBottom`)
- `table-layout: fixed`, `overflow-anchor: auto` belt-and-suspenders
- **Integrated perf probing** — per-kind marks (p50/p95/max), estimator-miss detection, layout-shift attribution scoped to agent pane, dev HUD extension

**Shipped state (per merged PRs):**
- PR #783 — Phase 1 foundation (store, anchor, partition, registry) ✅
- PR #784 — Phase 2 virtualization layer + streaming buffer ✅
- PR #785 (closed) → PR #787 — Phase 3 intelligent perf probing ✅
- Phase 4 (hardening) — **no PR found**, may be open work

**Overlap with new SPEC_INPUT_RESPONSIVENESS:**
- The per-kind perf marks in PR #787 are the same `frontend/perf` / `markStart` / `markEnd` infrastructure our new `agent-keystroke` / `agent-input-raf*` marks plug into.
- The virtualization itself directly relates to spec rule #1 (keystroke handler ≤16 ms) — a poorly-virtualized document above the composer is the main risk to that budget when the agent pane is long.
- Our new SPEC adds the contract; #782's work provides the load-bearing implementation.

**Status verdict:** **Mostly done.** Should be closed once Phase 4 is either shipped or explicitly deferred. The new consolidated discussion is a natural place to track Phase 4.

---

### Issue [#950 — Bulletproof terminals — `term.type` agent API + never-fail open guarantees](https://github.com/agentmuxai/agentmux/issues/950)

**Filed:** 2026-05-21. 0 comments. No labels.

**What it proposes (four phases):**
- **Phase 1** — Fix `bench-term-echo.mjs` `measureStreamThroughput` to use `blockinput` (no raw seq RPCs) so the bench can't freeze user terminals
- **Phase 2** — Add `term.type` to the agent App API (opaque, no seq exposure)
- **Phase 3** — Terminal opening reliability (5 s timeout, structured PTY-spawn errors, retry button)
- **Phase 4** — Benchmark isolation (dedicated pane via `pane.open` + `pane.close` on exit)

Plus six "Never-Fail Guarantees" (G1–G6).

**Shipped state:**
- PR #951 merged — seq-based input reorder buffer + threshold session reset (this is the immediate corruption fix from Phase 1 / G3 / G4)
- **`term.type` not in the codebase** (`grep '"term\.type"' agentmux-srv/src` = empty)
- **No 5 s open timeout** in `ShellController::start()` (would need verification)
- **No structured "Terminal failed to start" UI** in `TermView`
- **No benchmark isolation** — `bench-term-echo.mjs` still operates on the bench operator's active terminal

**Overlap with new SPEC_INPUT_RESPONSIVENESS:**
- Different surface. The new spec covers the **rendering / handler** path; #950 covers the **transport reliability + agent API surface** for terminal input. Both contribute to "fast and reliable typing" but address different layers.
- One concrete overlap: the new spec's item #7 (ACK-based PTY flow control) sits in the same code path #950's G4 (channel-full doesn't deadlock) protects. They're complementary; both belong in the same conversation.
- The new spec also lacks the agent-typing API consideration entirely — #950 fills that gap.

**Status verdict:** **Mostly NOT shipped.** Phases 2 + 3 + 4 are the real work and remain unfinished. Should stay open as the active tracker; the new discussion should link to it as the implementation child.

---

### New SPEC + PLAN (just landed via PR #1151)

- `docs/specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` — three structural rules + per-surface contracts
- `docs/specs/PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md` — 7-item action plan

Items 1–5 shipped via PRs #1146 / #1148 / #1150. Items 6 (terminal `targetFps` coalescer) and 7 (ACK-based PTY flow control) are profiling-gated and need data before code.

---

## Overlap matrix

| Concern | New SPEC | Issue #782 | Issue #950 |
|---|---|---|---|
| Agent textarea handler cost | **Rule #1, item #2** (marks) | Indirect — virt'd doc is the load above the textarea | — |
| Layout-thrash regression prevention | **Rule #3, item #4** (lint) | Phase 3 perf probing helps detect | — |
| IME composition (CJK Enter bug) | **Item #1** (shipped) | — | — |
| Agent-pane virtualization | (assumed in place — that's why rules work) | **Core scope** | — |
| Slash autocomplete scaling | **Item #5** (TODO comment) | — | — |
| Agent-keystroke benchmark | **Item #3** (shipped) | — | — |
| Terminal echo latency benchmark | (existing — `bench-term-echo.mjs`) | — | Phase 1 (fix harness so it can't freeze terminals) |
| Terminal flow control / write-buffer pressure | **Item #7** (gated) | — | G4 (drops, doesn't deadlock — partially shipped) |
| Terminal `targetFps` GPU/thermal cap | **Item #6** (gated) | — | — |
| Agent `term.type` API for terminal | — | — | **Phase 2 (unshipped)** |
| Terminal-open timeout + retry | — | — | **Phase 3 (unshipped)** |
| Benchmark isolation (no shared pane) | — | — | **Phase 4 (unshipped)** |

Reading: rows that have entries in **two or three columns** are the consolidation opportunities. Rows owned solely by #950 remain implementation tasks under that issue.

---

## Recommended consolidation

### Single new Discussion

**Title:** "Typing performance — terminal + agent pane umbrella"
**Category:** Ideas (matches existing perf-track discussions like #1097 "Menu hover lag vs VS Code")
**Body:** see §"Draft discussion body" below.

This becomes the **single entry point** for anyone asking "what's the typing-perf story?" It pins:
- The new SPEC + PLAN (the contract)
- Links to issue #782 (mostly-done virtualization, with Phase 4 noted)
- Links to issue #950 (unshipped term.type + reliability work)
- Tracks profiling-gated items 6 + 7 from the plan
- Status-summarizes what shipped today (PRs #1146/1148/1150/1151)

### What to do with the existing issues

| Item | Action | Rationale |
|---|---|---|
| Issue #782 | **Comment + leave open** with a link to the new discussion and a note clarifying Phase 4 is the only open child. Close once Phase 4 is shipped or explicitly cancelled. | The Phases 1–3 work has shipped; the issue is a real implementation tracker with one remaining child. Closing it now loses Phase 4 visibility. |
| Issue #950 | **Comment + leave open**, link to the new discussion as parent. Reaffirm Phases 2–4 are open. | This is the active implementation tracker. The new discussion is the umbrella; this issue is the child for the terminal-input agent API + reliability work. |

### What NOT to do

- **Don't close #782 today.** Phase 4 hasn't been explicitly resolved.
- **Don't merge #782 + #950 into one issue.** They're different surfaces (agent pane render path vs terminal transport + API). The umbrella discussion is the right consolidator; the issues remain the implementation trackers.
- **Don't open a third issue for items 6 + 7** from the new plan — track them in the discussion until profiling justifies promoting them to issues.

---

## Items intentionally NOT in scope (adjacent perf, but not typing)

Listed here so the next person searching doesn't think I missed them. Each has its own story.

| # | Title | Why not consolidated |
|---|---|---|
| [#1136](https://github.com/agentmuxai/agentmux/issues/1136) | perf(pane-focus): reduce click→focus-border paint latency | Different interaction (click → focus). Cross-cutting INP concern but its own implementation. |
| [#789](https://github.com/agentmuxai/agentmux/issues/789) | Markdown TOC reactivity when text streams | Streaming-render reactivity bug, not typing. Adjacent to #782 virt work but distinct. |
| [#871](https://github.com/agentmuxai/agentmux/issues/871) | opacity drag lag + capture_hwnd_for_label invariant | Drag, not typing. |
| [#121](https://github.com/agentmuxai/agentmux/issues/121) | PTY spawn serializes on concurrent pane creation | Pane initialization, not typing latency once a pane is open. |
| [#769](https://github.com/agentmuxai/agentmux/issues/769) | Perf: new-window bootstrap pays serialization tax on later tearoffs | Window/pane setup, not typing. |
| [#258](https://github.com/agentmuxai/agentmux/issues/258) | WebContent XPC OOM-killed after multi-day run (551 MB → UI freeze) | Memory leak → freeze, not typing-path lag. |
| [#779](https://github.com/agentmuxai/agentmux/issues/779) | Closed browser pane keeps eating keyboard input (zombie HWND) | Input *routing* bug, not typing latency. Different surface. |
| [#740](https://github.com/agentmuxai/agentmux/issues/740) | feat: voice input for agent pane | Different input modality. Already shipped per `SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md`. |
| [#376](https://github.com/agentmuxai/agentmux/issues/376) | Two crash signatures on v0.33.130 | Crashes, not perf. |
| Disc [#1097](https://github.com/agentmuxai/agentmux/discussions/1097) | Menu hover lag vs VS Code | Hover-open, not keystroke. Similar INP family but different code path. |

Possible future move: roll the menu hover discussion (#1097) into a broader "AgentMux INP / responsiveness umbrella" instead of having one umbrella for typing and one for hover. Not proposed today — let the typing discussion bed in first.

---

## Draft discussion body (paste-ready)

> # Typing performance — terminal + agent pane umbrella
>
> *(Ideas category — single entry point for keystroke / typing responsiveness across both input surfaces.)*
>
> ## What this is
>
> AgentMux has two keyboard-input surfaces and we've been managing typing-perf for them as one-off chases: catch a 22 ms reflow, fix it; catch a buffer overflow, fix it. PR #1151 landed a forward-looking spec — `SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` — that writes down the structural rules so future input-path code doesn't keep paying the same debugging cost.
>
> This discussion is the **umbrella** for all open typing-perf work, organized under that spec. It pulls together what just shipped (today) and what's still in flight.
>
> ## The contract (what every input-path change must obey)
>
> Three rules from the SPEC §4:
>
> 1. The keystroke handler runs in ≤16 ms, always.
> 2. Anything that can run later, must run later.
> 3. Never read layout after touching style on the keystroke path.
>
> Plus surface contracts: terminal — never let xterm.js's write buffer saturate. Agent — handle IME composition before treating Enter as submit. Preserve the uncontrolled-DOM textarea.
>
> ## Shipped today (2026-05-29)
>
> - PR #1146 — IME composition handling (real user-facing bug fix for CJK / IME users), `agent-keystroke` perf marks, slash-matcher TODO
> - PR #1148 — CI lint guardrail against layout reads in `frontend/app/view/term/**` + `AgentFooter.tsx`
> - PR #1150 — `tools/tests/bench-agent-keystroke.mjs` (CDP-driven P50/P95/P99 bench)
> - PR #1151 — SPEC + execution plan
>
> ## Open implementation children
>
> ### Issue #782 — agent-pane virtualization + perf probing
> **Status:** Phases 1–3 shipped (PRs #783 / #784 / #787). Phase 4 (hardening) has no PR. This work is the load-bearing implementation under SPEC rule #1 — a poorly-virtualized document above the composer is the main risk to the 16 ms budget. Once Phase 4 is resolved (shipped or explicitly dropped), close the issue.
>
> ### Issue #950 — bulletproof terminals + `term.type` agent API
> **Status:** Only PR #951 (seq-reorder-buffer fix) has shipped. Still open:
> - Phase 2 — add `term.type` to agent App API (opaque, no seq exposure)
> - Phase 3 — 5 s open timeout, structured PTY-spawn errors, retry UI
> - Phase 4 — benchmark isolation (dedicated pane via `pane.open`/`pane.close`)
>
> This is a different surface from the agent pane — it covers terminal transport reliability + the agent-side typing API. Same umbrella because both contribute to "typing always works, always fast."
>
> ### Profiling-gated items from PLAN §6 + §7
>
> - **Item 6 — terminal `targetFps` coalescer for non-input writes.** GPU/thermal benefit on laptops. Profile first (PLAN §6 has the methodology). Promote to an issue only if profile shows ≥10°C delta under heavy output.
> - **Item 7 — ACK-based PTY flow control end-to-end.** Cross-stack: backend pause/resume + WS ack protocol + frontend `term.write(chunk, ack)`. Profile first (PLAN §7). Promote to an issue only if P95 keystroke echo > 100 ms under sustained `yes`-style output.
>
> ## Adjacent but distinct (NOT folded in)
>
> See `docs/analysis/ANALYSIS_TYPING_PERF_OPEN_TRACKING_2026_05_29.md` for the full list. Highlights:
> - #1136 (pane-focus paint latency) — different interaction
> - #789 (Markdown TOC streaming reactivity) — affects agent pane but not typing
> - #258 (XPC OOM-kill → UI freeze) — memory, not typing
> - Discussion #1097 (Menu hover lag) — different interaction. May later roll up with this one under a broader "AgentMux INP" umbrella.
>
> ## How to contribute
>
> - Touching `frontend/app/view/term/**` or `AgentFooter.tsx`? The CI lint will catch layout reads in input handlers. Use the `// perf:allow-layout-read — <why>` escape hatch only with justification.
> - Adding a feature that hooks into the keystroke handler? Cite the SPEC and show a perf-mark span proving P95 < 5 ms for your feature.
> - Adding a completion source to slash autocomplete? Cite PLAN §6.3 and time-slice with `scheduler.yield()` once your source has ≥ 50 entries.
> - Profiling items 6 or 7? Use the methodology in PLAN §6 / §7 and post results here.

---

## What I'm waiting on you for

- Approve the consolidation plan above before I open the discussion or post comments on #782 / #950.
- Decide whether you want #782 closed today (Phase 4 explicitly dropped) or left open as a one-child tracker.
- Decide whether the discussion is "Ideas" or somewhere else.
