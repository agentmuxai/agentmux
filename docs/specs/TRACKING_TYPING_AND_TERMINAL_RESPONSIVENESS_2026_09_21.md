# TRACKING — Typing & terminal input responsiveness

**Date:** 2026-09-21 (reviewed 2026-09-22 — no drift found; two Phase-0 audits folded in, see §2.1/§3)
**Type:** Tracking doc — canonical location for this problem family. Not a design spec.
**Status:** Living — a continuously maintained tracking reference with no terminal state. Update it when anything below changes.
**Owner prompt (paraphrased, 2026-09-21):** silky-smooth typing in the agent pane and terminal, under heavy DOM/output processing, was achieved once — has it reverted? What's the current state?

---

## 0. Why this file exists

Six months of typing/terminal-responsiveness work is scattered across 3 April
one-off incident docs, an umbrella GitHub Discussion (#1161), a spec+plan pair,
two cross-pane regression analyses, and a still-open issue (#3361) — with no
single doc a reader can start from. This is that doc, modeled on
`TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`'s pattern.

**Tracking issue: none dedicated — the closest live thread is #3361. Umbrella
discussion: #1161.**

## 1. The short answer: did it revert?

**No single fix regressed.** Every direct code check below shows the shipped
composer/markdown fixes are still in place, byte-for-byte in substance, as of
2026-09-21. What actually happened is three separate things that *feel* like
one continuous regression from the outside:

1. The **agent-pane composer + streaming-markdown** path (the original
   "silky typing" win, April–May) is solid and unregressed — verified in code,
   not just docs, this session. So is **predictive local echo** (#1223,
   2026-06-01) — a separate shipped win this doc's 09-21 draft omitted entirely,
   added 2026-09-22 (§2.1). It's easy to miss because the same Discussion
   #1161 thread that shipped it one day later still reads, out of context, like
   it *rejected* predictive echo — see the correction in §2.2.
2. A **terminal fix that was designed and formally decided** ("Decision:
   proceed with flow control" — Discussion #1161, 2026-05-30) **was never
   implemented.** `SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md` is still
   `Draft — design, pre-implementation`, unchanged, four months later. Nothing
   reverted here either — it just never shipped in the first place.
3. **New code landed since** (`PR #3194`, the `PtyShell*` agent-driven-shell
   feature, 2026-09-11) **introduced a new contention path** that reproduces a
   similar-feeling "typing near a busy pane is laggy" symptom through a
   different mechanism than the one the 09-04 fix closed. One instance of that
   new bug was found and fixed (`PR #3249`); the *original* reported cross-pane
   symptom is still formally undiagnosed past it.

So: nothing to revert, one designed fix never built, and one new mechanism
introduced by unrelated feature work. Issue #3361 (open, unclaimed, 3 days old)
is the live ask to close both loops for real using the repro tool that now
exists to do it (`tools/tests/pane-load.mjs`, merged 09-17 as #3286).

## 2. State of play

### 2.1 Shipped and verified intact (checked against code 2026-09-21)

| | |
|---|---|
| Uncontrolled `<textarea>` composer (root fix for the original re-render-storm lag) | v0.33.91, `AgentFooter.tsx` |
| IME composition handling + `agent-keystroke` perf marks | #1146 |
| CI lint guardrail against layout reads in `frontend/app/view/term/**` + `AgentFooter.tsx` | #1148 |
| `tools/tests/bench-agent-keystroke.mjs` (CDP P50/P95/P99 bench) | #1150 |
| Throttle (~1/90ms) + defer-highlight streaming-markdown render, replacing the rejected per-block-split plan | #1213 — `MarkdownBlock.tsx`'s `STREAM_RENDER_MS`, confirmed present and unmodified in substance |
| Agent-pane virtualization, Phases 1–3 | #783 / #784 / #787 (issue #782) |
| PTY output coalescing before broadcast (cuts renderer/GPU CPU under heavy output) | #3206 |
| Per-pane fair egress dequeue (`fair_drain_priority`/`priority_pane_key`) — the 09-04 cross-pane fix | #2973 — confirmed intact, no commit touching `websocket.rs` since has changed this machinery (diffed all 8) |
| `pane-load.mjs` — on-demand realistic PTY load repro (bulk text / escape-heavy repaint / many-tiny-writes), the tool item 3 below depends on | #3286 |
| In-memory `agent_lock` registry, replacing a blocking `Store` (SQLite) read on the `controllerinput`/`PtyShellInput` path | #3249 (09-15 report, at corrected scope — see §2.3) |
| Phase 0.2 — keydown-path synchronous-IPC audit: baseline was already clean (no `await` IPC/async handlers on the keystroke path); CI guard added to keep it that way | `tools/lint/check-input-handler-sync-ipc.sh` + `.github/workflows/input-handler-sync-ipc.yml` |
| Phase 0.3 — `backdrop-filter: blur` audit: only one always-mounted blur exists over a typed-into pane (`.block-mask`, 0.1px — layer-promotion only, not real blur cost); everything else is `<Show>`-gated or gesture-transient. Verdict: cleared, no code change warranted | analysis-only, no PR |
| **Predictive local echo** — paints a just-typed printable character in the keydown frame, reconciles byte-exact against the authoritative PTY echo; observational arming means it never predicts without a confirmed echo first (no password-prompt flash). **Un-shelves and explicitly supersedes** the 2026-05-30 "rejected" call in §2.2 below, one day later, once armed-observation + reconcile/rollback answered the security objection. On by default (`term:predictiveecho=false` to opt out). Verified: only 2 commits ever, last 2026-06-06, unmodified since | `SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md`, #1223 (merged 06-01), stall-cooldown fix #1242 (06-06) |

### 2.1a Agent pane — a different problem from the terminal, measured 2026-09-22/23

None of §2.1 applies to the agent-pane composer (uncontrolled `<textarea>`, no
WS/PTY round-trip). Its lag is main-thread starvation by the stream render path.
Three layers found and fixed in three days, each measured before and after:

| | | |
|---|---|---|
| Streaming markdown re-parsed the whole message every commit (O(n²)) | #3521 — incremental parse | `ANALYSIS_AGENT_PANE_TYPING_UNDER_LOAD_2026_09_22.md` §2 |
| Backgrounded keep-alive panes kept rendering markdown on the same thread | #3536 — dormancy gate | same doc, §4 |
| **Every finished tool result in the streaming buffer was rebuilt on every flush** (new `dispatchMatches` Map identity read through an inline prop getter; `.md` previews re-parsed + rebuilt an OverlayScrollbars each time). 4 visible streaming panes: 2.3 fps → 55 fps, forced layout 5.9 s → 48 ms per 15 s | #3555 — equality-gated memo in `DocumentRow`, `scrollable={false}` on the previews | `ANALYSIS_AGENT_PANE_FLUSH_REMOUNT_CHURN_2026_09_23.md` |
| The streaming message's markdown DOM was rebuilt per commit (parse was incremental, DOM was not) | #3559 — frozen-prefix DOM reuse, processor built once | same doc, §6 |
| Pin-to-bottom forced layout on every flush, and every pane's flush landed in the same frame | Phase 1 + 2 of the bounded-live-window spec (pin after layout in a ResizeObserver; cross-pane scheduler, one pane per frame while typing): key → paint p95 −32 %, blocking −12–26 % | `TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md` §2.1 |

The remaining agent-pane work — cost that grows with conversation length, and
the ~40 ms per-flush cost of the real stream pipeline — is planned in
`SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md` and tracked phase
by phase in `TRACKING_AGENT_PANE_BOUNDED_LIVE_WINDOW_2026_09_23.md`. Measure
with `scripts/ui-screenshots/full-conversation-bench.mjs` (#3593) — no human
at the keyboard and no agent tokens needed.

### 2.2 Decided but never built

| Item | Status | Notes |
|---|---|---|
| **ACK-based PTY flow control** (`SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md`) | Draft, pre-implementation, unchanged since 2026-05-30 | Discussion #1161's 2026-05-30 update rejected predictive/local echo *at that point* (optimizes a round-trip that doesn't exist locally, security hazard over password prompts) in favor of flow control. **Correction (added 2026-09-22): predictive echo was un-shelved the very next day** — see the shipped row below; this flow-control item is the one that's still undelivered, not both. Gated on a profiling step (PLAN §7: "promote to an issue only if P95 keystroke echo > 100ms under sustained output") that was never run — until #3286 existed, nothing could drive the load needed to run it. |
| Virtualization Phase 4 (hardening) | No PR | Issue #782 still open on this alone. |
| `term.type` agent App API (issue #950 Phases 2–4) | Only Phase 1 equivalent (#951, seq-reorder-buffer) shipped | Different surface (terminal transport reliability / agent-side typing API), same umbrella. |
| Terminal `targetFps` coalescer for non-input writes (PLAN §6) | Not promoted | Profiling-gated on a ≥10°C thermal delta; profiling never run. Lower priority than item 7 above. |

### 2.3 Cross-pane lag — real fixes shipped, original symptom still open

Two analyses, same reported symptom, two different bugs found:

- **`ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md`** — diagnosed a connection-wide FIFO egress lane with no per-pane fairness. Fixed (#2973). **Confirmed still intact** (§2.1).
- **`ANALYSIS_CROSS_PANE_INPUT_DELAY_REGRESSION_2026_09_15.md`** — investigated the *same user-facing symptom returning*. First pass misattributed it to `controllerinput` being "every keystroke" — **wrong, corrected same day (§2a of that doc)**: keystrokes travel over `blockinput`, not `controllerinput`. What survived: a real, unrelated bug (`PR #3194` added a blocking synchronous SQLite read on `controllerinput`/`PtyShellInput`, the same bug class as the historical `sysinfo.rs` fix, #1782) — fixed via an in-memory lease registry (#3249). **But fixing it does not explain the originally reported symptom.** That doc's own conclusion: "the cross-pane typing lag is still undiagnosed."
- Bench coverage gap named in that doc (§4 item 3): extend `bench-term-cross-pane.mjs` (or a sibling) to exercise concurrent `controllerinput`/`PtyShellInput` traffic, not just an output flood — needed to actually catch this bug class in CI. Not built as of 2026-09-21.

**Read the correction (§2a) before the diagnosis (§1) in that doc** — same failure shape as the availability-tracking doc's "read the banner before the section."

### 2.4 Open work — issue #3361 (opened 2026-09-18, still open, no PR references it)

1. Run `pane-load.mjs` for **same-pane** load (`--mode paint`/`mixed`, type directly into the loaded pane) to get a real P95 keystroke-echo number. If > 100ms (PLAN §7's own gate), implement `SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md` for real.
2. Run it for **cross-pane** too, to determine whether §2.3's two fixes fully closed the symptom or whether something is still open past them.
3. Fold the result back into Discussion #1161 either way, so the "profiling-gated" placeholder in PLAN §6/§7 stops being 4 months stale.

This is genuinely unclaimed work, not something already in flight — worth picking up directly if the goal is to actually close the loop rather than re-diagnose it a third time.

## 3. Document index

Nothing is deleted. Nothing below needed a status change beyond one file (§3.3).

### Canonical
| Doc | Role |
|---|---|
| **this file** | tracking, status, open items |
| Discussion **#1161** | the umbrella — contract (§4's three rules), shipped-today log, open children |
| Issue **#3361** | the live, unclaimed ask — see §2.4 |
| `SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md` | the contract every input-path change must obey (≤16ms handler, never read layout after touching style on the keystroke path) |
| `PLAN_INPUT_RESPONSIVENESS_EXECUTION_2026_05_29.md` | the execution plan; §6/§7 are the profiling gates #3361 exists to finally clear |
| `SPEC_TERMINAL_FLOW_CONTROL_2026_05_30.md` | the undelivered design — read before implementing §2.4 item 1 |
| `docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_UNDER_OUTPUT_LOAD_2026_09_04.md` | the egress-fairness diagnosis + fix, intact |
| `docs/analysis/ANALYSIS_CROSS_PANE_INPUT_DELAY_REGRESSION_2026_09_15.md` | the corrected re-diagnosis — read §2a before §1 |
| `docs/analysis/ANALYSIS_AGENT_PANE_TYPING_LATENCY_2026_05_30.md` | the streaming-markdown O(n²) root cause + the shipped (different-shaped) fix, #1213 |
| `docs/analysis/ANALYSIS_KEYDOWN_IPC_AUDIT_2026_05_29.md` | Phase 0.2 — keydown dispatch sync-IPC audit (clean baseline, CI guard shipped) |
| `docs/analysis/ANALYSIS_BLUR_AUDIT_INPUT_FIRST_2026_05_30.md` | Phase 0.3 — `backdrop-filter` audit over typed-into panes (cleared, no fix needed) |
| `docs/specs/SPEC_TERMINAL_PREDICTIVE_LOCAL_ECHO_2026_05_31.md` | the un-shelving design (its own §2 title: "Why this was shelved, and why we are un-shelving it") — shipped as #1223, still live |

### Superseded — banner + `Superseded-by:` added alongside this file
| Doc | What's dead |
|---|---|
| `docs/analysis/ANALYSIS_TYPING_PERF_OPEN_TRACKING_2026_05_29.md` | entirely — it was the 05-29 snapshot of what was open; this file is now that snapshot, kept current |

### History — accurate for their date, not current
`docs/analysis/agent-pane-typing-lag-2026-04-12.md` (the original controlled-textarea root cause, v0.33.91) ·
`docs/analysis/agent-typing-lag-trace-2026-04-12.md` (DevTools trace analysis, v0.33.105) ·
`docs/analysis/agent-pane-extreme-smoothness-2026-04-12.md` (the "5 fixes" bundle behind #334 — its node-cap/content-visibility approach to bounding DOM size was superseded in spirit by real virtualization, #782, though the CSS containment/`content-visibility` pieces are still present in `_document.scss`)

### Not in scope despite similar symptoms
`docs/analysis/ANALYSIS_TEAR_OFF_PERF_2026_06_13.md` and
`docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md` matched a
keyword search for "typing" but are about tear-off/redock window mechanics, not
keystroke latency — a different symptom that happens to share the word.

### Adjacent — grew out of this investigation, tracked separately
`docs/specs/SPEC_TAB_SWITCH_PER_PANE_PROGRESSIVE_REVEAL_2026_09_21.md` — many-agent
tab-switch/reveal-latency design (not keystroke echo). Cites this file's §3 finding
(`useAgentStream.ts` dispatches ungated by visibility) as part of its own root-cause
argument for why the tab-reveal gate's global quiet-window signal breaks down with
several busy panes. Read that file for the reveal/paint side; this one for keystrokes.

---

## 4. If you're about to re-diagnose this a third time

Don't, until you've read §2.3's correction. The pattern that has already burned
two passes here: assume a handler's name describes its call frequency, without
checking the frontend for which WS command actually carries keystrokes. Verify
against `termViewModel.ts` / `AgentShellSubblock.tsx` first.
