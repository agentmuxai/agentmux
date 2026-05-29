# PLAN: Execute SPEC_INPUT_RESPONSIVENESS — terminal + agent pane

**Date:** 2026-05-29
**Owner:** Agent1
**Tracks:** [`SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`](./SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md) §9 action items
**Status:** Draft (awaiting approval before execution starts)

This document is the execution roadmap for the spec's seven action items. Items 1, 2, 5 need essentially no design — capturing them here for completeness. Items 3, 4 need a small design decision before writing code. Items 6, 7 are gated on a profiling pass and get a profiling plan, not a code plan.

**Recommended order:** #1 → #2 → #4 → #3 → (profile pass for #6/#7) → #5 last (or never). Rationale: #1 fixes a real bug today; #2 unblocks #3 (can't bench what you can't mark); #4 prevents the next regression. #5 is preemptive and arguably premature.

---

## Item 1 — IME composition fix (HIGH)

**Goal:** Pressing Enter to confirm an IME candidate (CJK, Vietnamese, etc.) must NOT submit the in-progress preedit string.

**Planning needed:** None. Spec §6.2 has the exact snippet.

**Concrete steps:**

1. Locate the Enter handler in `frontend/app/view/agent/components/AgentFooter.tsx` (grep for `Enter` in `onKeyDown`).
2. Add the guard:
   ```ts
   if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
     submit();
     e.preventDefault();
   }
   ```
3. Gate the slash-autocomplete prefix tracker on composition state — add `compositionstart`/`compositionend` listeners that pause input processing while composing.
4. Manual test: open agent pane, switch input source to a CJK IME (or use Windows IME emulator), type pinyin/romaji, press Space to bring up candidates, press Enter to confirm — should commit to textarea, NOT submit the agent prompt. Then press Enter again — should submit.

**Files touched:** `frontend/app/view/agent/components/AgentFooter.tsx` (one file).

**Acceptance:**
- Enter-during-composition does not call submit (verified via manual test + a unit test that synthesizes a `KeyboardEvent` with `isComposing: true`).
- Slash-autocomplete dropdown doesn't flicker during composition.
- No regression to current `Shift+Enter = newline` behavior.

**Risk:** Negligible. Standard CJK web-input pattern, no API surface change.

**Branch / PR:** `agent1/agent-ime-composition-fix` → small focused PR with manual-test screenshot.

---

## Item 2 — Add `agent-keystroke` perf marks to `AgentFooter.tsx`

**Goal:** Mirror the terminal's `term-keypress` / `term-echo-render` / `term-raf-write` pattern in the agent composer, so the same Perf HUD surfaces both surfaces and item 3's bench has marks to read.

**Planning needed:** None. Spec §7.1 lists the four mark pairs verbatim.

**Concrete steps:**

1. Read `frontend/app/view/term/termwrap.ts` to see the existing `performance.mark()` / `performance.measure()` shape (look for `term-keypress`, `term-echo-render`, `term-raf-write`).
2. Add identical pattern to `AgentFooter.tsx`:
   - `agent-keystroke` — `onInput` entry → return
   - `agent-input-raf` — RAF enqueue → callback start
   - `agent-input-raf-cb` — RAF callback start → end
   - `agent-submit` — submit handler entry → WS send return
3. Wire each pair into `performance.measure()` so they show up in the Perf HUD's Timings row.
4. Verify in DevTools: type a character, see the marks appear in the Timings row; the `agent-keystroke` span should be <2 ms per the current target.

**Files touched:** `frontend/app/view/agent/components/AgentFooter.tsx`. Possibly extend the Perf HUD label list if it has a hardcoded set (check `frontend/app/perf/` or similar).

**Acceptance:**
- All four marks visible in CEF DevTools Performance tab.
- All four marks visible in dev-mode Perf HUD (`Ctrl+Shift+P`).
- Production-safe overhead (~50 ns per mark, same as terminal pattern).

**Risk:** Negligible. Marks are no-cost in production builds.

**Branch / PR:** `agent1/agent-composer-perf-marks` → small focused PR.

---

## Item 3 — `tools/tests/bench-agent-keystroke.mjs`

**Goal:** CI-runnable P95 keystroke-latency benchmark for the agent composer, counterpart to `bench-term-echo.mjs`.

**Planning needed:** Yes — three design decisions.

### Design decisions

**Q1: What does "echo" mean in the agent textarea?**

Three candidates:
- (a) Browser paint of the typed character in the textarea (analogous to terminal echo).
- (b) `agent-keystroke` mark span (`onInput` entry → return) — JS handler cost only.
- (c) End-to-end: keypress dispatch → next frame paint of the new character.

**Decision: (c).** The terminal bench measures end-to-end keystroke → paint, so the agent bench should too. (a) is too noisy (browser-internal), (b) is too narrow (misses RAF + paint). End-to-end is what the user actually experiences. Implementation: subscribe to the `agent-keystroke` mark + the next `requestAnimationFrame` callback after it, time from synthetic keydown → RAF callback.

**Q2: How to drive synthetic keystrokes without firing the agent?**

Two options:
- (a) Use the App API (`pane.open agent` + send synthetic keys via CDP `Input.dispatchKeyEvent`).
- (b) Bench the textarea in isolation, mounted in a fixture page outside the agent pane.

**Decision: (a).** Same approach as the terminal bench — it drives a real pane via App API. Isolating the textarea would miss interaction with the surrounding pane (virtualized document above, status strip below). Need a "dry-run" mode: type characters into the textarea, never actually submit (never trigger an agent invocation, never spend tokens). Spec §6.1 already establishes the textarea is uncontrolled, so typing without submitting is the natural pattern.

**Q3: What latency target does the bench enforce?**

Per spec §2: P50 ≤ 16 ms, P95 ≤ 50 ms (the "internal snappy" budget for agent keystrokes).

### Approach

1. Copy `tools/tests/bench-term-echo.mjs` skeleton.
2. Open an agent pane via App API (`pane.open` with widget `agent`).
3. Focus the textarea (CDP `Element.focus` on the `.agent-input` selector).
4. Drive N synthetic keystrokes (e.g. 200) via CDP `Input.dispatchKeyEvent`, each spaced ≥ 50 ms apart (avoid burst collapse).
5. For each keystroke: timestamp at dispatch → wait for the matching `agent-keystroke` mark + next RAF callback → compute latency.
6. Compute P50, P95, max. Report.
7. Exit non-zero if P95 > 50 ms (configurable via `--p95-threshold-ms`).

**Files touched:** `tools/tests/bench-agent-keystroke.mjs` (new). Possibly a small `tools/tests/lib/bench-common.mjs` if shared logic with `bench-term-echo.mjs` is worth extracting.

**Acceptance:**
- Bench runs from `agent1` container against a `task dev` instance.
- Reports P50, P95, max latency; produces a JSON line consumable by CI.
- Reproducible run-to-run (P95 variance < 5 ms).
- Documented in `tools/tests/README.md` next to the term bench.

**Risk:** Synthetic key dispatch may not perfectly emulate real OS keystrokes (timing jitter, IME interactions). Mitigation: state explicitly in the bench README that it measures the *handler + render path*, not OS input pipeline.

**Branch / PR:** `agent1/agent-keystroke-bench` (depends on item 2 being merged first).

---

## Item 4 — ESLint / CI guardrail against layout reads in input handlers

**Goal:** Prevent the 22 ms `scrollHeight` regression from recurring. Make any new layout-read inside a keystroke/input handler fail CI.

**Planning needed:** Yes — one design decision.

### Design decision

**Q: Custom ESLint rule, or grep-based CI step?**

| Approach | Pros | Cons | Effort |
|---|---|---|---|
| **Custom ESLint rule** | AST-aware (catches `el.scrollHeight` regardless of receiver var name), inline `// eslint-disable-next-line` escape hatch, no false positives on string literals or comments | Half-day to write, must be maintained, needs unit tests | ~4 hrs |
| **Grep CI step** | Ships in 30 min, easy to understand, easy to modify | Coarse (catches `scrollHeight` in comments, strings, unrelated files), escape hatch needs custom marker, more false positives | ~30 min |

**Decision: Start with grep, evolve to ESLint rule if false-positive volume becomes annoying.** The banned property list is short (~10 names), the scope is narrow (two directories), and the goal is "make the regression visible," not "perfect AST analysis." Grep with a `# allow:` comment escape hatch is good enough for v1.

### Approach (v1: grep)

1. Add `tools/lint/check-input-handler-layout-reads.sh`:
   - Scans `frontend/app/view/term/**/*.ts*` and `frontend/app/view/agent/components/AgentFooter.tsx`.
   - Identifies functions attached to `onInput|onKeyDown|onKeyPress|onKeyUp|onBeforeInput|onCompositionUpdate` props.
   - Within those functions' line ranges, greps for the banned identifiers: `scrollHeight`, `scrollTop`, `scrollWidth`, `scrollLeft`, `offsetHeight`, `offsetTop`, `offsetWidth`, `offsetLeft`, `clientHeight`, `clientTop`, `clientWidth`, `clientLeft`, `getBoundingClientRect`, `getClientRects`, `getComputedStyle`.
   - Exits non-zero with a diff-style report on match.
2. Escape hatch: any line containing `// perf:allow-layout-read <reason>` is skipped. PR review judges whether the reason is sound.
3. Add CI step in the relevant GitHub Actions workflow (find via `find .github -name "*.yml" | xargs grep -l "frontend"`).
4. Document in `frontend/app/view/term/README.md` and `frontend/app/view/agent/README.md` (or create) so future contributors know the rule exists.

### Approach (v2, IF v1 has too many false positives)

Custom ESLint rule under `tools/eslint-rules/no-layout-read-in-input-handler.js`:
- Hooks into the AST.
- Identifies JSX attributes `onInput`/`onKeyDown`/etc.
- Tracks ParameterReferences flowing into property reads of the banned set.
- Same escape hatch via `// eslint-disable-next-line no-layout-read-in-input-handler`.

Decision point for v2: if grep produces >5 false positives in the first month, escalate to ESLint.

**Files touched:** `tools/lint/check-input-handler-layout-reads.sh` (new), `.github/workflows/<frontend-ci>.yml` (modify).

**Acceptance:**
- Grep script runs in <1 s on the full frontend tree.
- Catches `el.scrollHeight` inside an `onInput` lambda — verified with a synthetic violating diff.
- Skips lines marked `// perf:allow-layout-read`.
- CI step blocks merge on violation.
- Tested on current `main` — must pass (no current violations expected, since the spec is grounded in the current code).

**Risk:**
- False positives on legitimate `scrollHeight` reads in non-input contexts that happen to be in the same file. Mitigation: scope check to functions attached to input event props.
- Future SolidJS syntax changes could break the function-extraction regex. Acceptable — escalate to ESLint at that point.

**Branch / PR:** `agent1/lint-input-handler-layout-reads` (independent of items 1-3).

---

## Item 5 — `scheduler.yield()` in slash-autocomplete matcher

**Goal:** Time-slice the matcher so large completion sources don't block keystrokes.

**Planning needed:** None — but **recommend NOT doing it now.**

**Why defer:** Current matcher operates on a small fixed slash-command list (~5 entries). Adding `scheduler.yield()` today adds complexity without measurable benefit. Per the spec's own principle ("Don't add features beyond what the task requires"), this is premature.

**Concrete action today:** Add a one-line TODO comment in `SlashAutocomplete.tsx`:
```ts
// TODO(perf): when completion source grows past ~50 entries, time-slice
// with scheduler.yield() per SPEC_INPUT_RESPONSIVENESS §6.3.
```

**Revisit trigger:** Any PR adding a new completion source (history, agent-specific commands, semantic match) MUST cite this item and either add the slicing OR justify why it's not needed yet.

**No branch needed.** Folds into whichever PR first expands the matcher.

---

## Item 6 — `targetFps` coalescer for terminal non-input writes (PROFILING REQUIRED)

**Goal:** Reduce GPU / thermal load on laptops during heavy terminal output (e.g. `cat` of large files, build streaming).

**Planning needed:** Yes — but profiling, not code. Spec §5.2 explicitly says "Measure first."

### Profiling plan

**Hypothesis to test:** Sustained xterm.js WebGL rendering causes measurable thermal throttling on laptop hardware during heavy output.

**Method:**

1. **Setup:** A laptop, plugged in, ambient temperature ~22°C. Open AgentMux portable build, one terminal pane.
2. **Baseline (idle):** Record CPU temp, GPU temp, package power for 60 s. Use `pwsh` `Get-Counter` on Windows, `powermetrics` on macOS, `sensors` + `intel_gpu_top` on Linux.
3. **Heavy output run:** Stream output to the terminal — `yes` for 60 s, then `cat /usr/share/dict/words` ×100, then a real build (e.g. `task build:frontend`). Capture the same metrics throughout.
4. **Battery run:** Repeat on battery to isolate AC-vs-battery throttling behavior.
5. **Report:** Peak temp delta, sustained power delta, any frequency throttling observed (Windows `wpr`, macOS `powermetrics -s cpu_power`, Linux `turbostat`).

**Decision criteria:**
- If peak GPU temp delta > 15°C OR sustained throttle observed → implement coalescer.
- If delta < 5°C → skip; complexity not justified.
- If 5–15°C → judgment call, lean toward implementing on battery only.

### Code plan (only if profiling justifies)

If profiling triggers implementation:

1. Add `TermWrap.targetFps: number | null` config (default null = no coalescing).
2. In `scheduleRafWrite`, if `targetFps !== null` AND the incoming data is NOT a small input echo (existing fast-path predicate), batch into a `setTimeout`-based coalescer with window `1000 / targetFps`.
3. Wire `targetFps` to a setting (`terminal:target_fps` or auto-derived from `navigator.getBattery()`).
4. Re-run profile to confirm the predicted thermal benefit.

**Files touched (if coding):** `frontend/app/view/term/termwrap.ts`, settings schema.

**Acceptance (if coding):**
- Profile shows ≥10°C reduction in sustained GPU temp under heavy output, with coalescer at 60 fps AC / 30 fps battery.
- Bench from item #3 shows no keystroke-echo regression (input fast path bypasses coalescer).
- Default setting is "off"; opt-in or auto-on-battery only.

**Branch / PR:** `agent1/term-thermal-profile` (profiling pass; results go in `docs/perf/`) → if justified, `agent1/term-targetfps-coalescer`.

---

## Item 7 — ACK-based PTY flow control (PROFILING REQUIRED)

**Goal:** End-to-end backpressure between PTY producer and xterm.js renderer, so fast producers can't starve the keystroke loop.

**Planning needed:** Yes — profiling first, then a design pass. This is the largest item.

### Profiling plan

**Hypothesis to test:** AgentMux's `term-rpc` + `termwrap` path can saturate xterm.js's internal write buffer under realistic high-throughput producers, causing measurable keystroke lag.

**Method:**

1. Add a temporary instrumentation hook to `termwrap.ts` to log `rafBuffer.length` and the xterm.js internal `_inputBuffer.size` (if accessible) every second.
2. Run three load scenarios:
   - Sustained: `yes | head -n 10000000` (cap at ~10M lines so the test terminates).
   - Bursty: `for i in {1..50}; do cat /var/log/syslog; done` (or equivalent Windows: `Get-Content -Path C:\Windows\Logs\*.log -Tail 100000`).
   - Realistic: a live build emitting ANSI color output (`task build:backend` from a fresh state).
3. During each run, type into the terminal every 2 s. Record `agent-keystroke` / `term-echo-render` P95.
4. Compare to baseline (idle terminal).

**Decision criteria:**
- If P95 keystroke echo under heavy load > 100 ms → implement flow control.
- If P95 stays < 50 ms → skip; xterm.js's internal scheduling is sufficient for our load profile.

### Design pass (only if profiling justifies)

Cross-stack change:

1. **WS protocol:** add an `ack` message type from frontend → backend, with a sequence number.
2. **Frontend** (`term-rpc.tsx` + `termwrap.ts`): every Nth chunk written (e.g. N=128 KiB cumulative), call `terminal.write(chunk, () => sendAck(seq))` instead of the fire-and-forget form.
3. **Backend** (`agentmux-srv/src/backend/blockcontroller/shell.rs` PTY pump): track outstanding (unacked) byte count per-block. When it exceeds a watermark (e.g. 256 KiB), pause reading from the PTY. Resume on ack.
4. **Tuning:** ACK granularity vs flow-control responsiveness vs WS overhead. Start with N=128 KiB / watermark=256 KiB; tune from bench results.

This needs its own spec doc — outline only here. Cite the [xterm.js flow control guide](https://xtermjs.org/docs/guides/flowcontrol/) as the reference pattern.

**Files touched (if coding):** `agentmux-srv/src/backend/blockcontroller/shell.rs`, `agentmux-srv/src/ws/term.rs` (or wherever WS term messages live), `frontend/app/view/term/term-rpc.tsx`, `frontend/app/view/term/termwrap.ts`.

**Acceptance (if coding):**
- Bench from item #3 + a load-generator script show P95 keystroke echo < 50 ms even under sustained `yes` output.
- No regression on quiet-terminal keystroke latency.
- No measurable WS message-rate increase under light load (acks coalesced).

**Branch / PR:** `agent1/term-flow-control-profile` (profiling; results in `docs/perf/`) → if justified, `agent1/term-flow-control-design` (spec) → then implementation PRs (likely 2-3, split front/back).

---

## Cross-cutting: dependencies and ordering

```
#1 (IME fix)        ──────► ship immediately, independent
#2 (perf marks)     ──────► unblocks #3
   │
   └──► #3 (agent bench)  ──────► reads marks from #2; also useful for verifying #6/#7
#4 (lint guardrail) ──────► independent; prevents regressions across all items
#5 (slash yield)    ──────► defer (TODO comment only)
#6 (targetFps)      ──────► profile first; code only if justified
#7 (flow control)   ──────► profile first; design spec second; implementation last
```

**Critical path to "this spec is enforced":** #1 + #2 + #4 + #3 (in that order, ~2 days total).
**Conditional follow-ups:** #6, #7 after one profiling pass each.
**Skip until needed:** #5.

---

## Open questions for you

1. **Item 4 — grep first, or jump to ESLint?** I recommended grep-first for shipping speed, but if you'd rather invest the half-day for a proper AST rule from the start, say so.
2. **Item 6/7 — who owns the profiling pass?** Profiling laptop hardware ideally happens on a real laptop. I can outline the script and bash through it in the container, but the thermal readings need a real machine. Want me to write the profiling script for you to run, or skip the profile and just implement coalescing on faith (not recommended)?
3. **Item 5 — confirm "defer" decision?** I'm 80% sure adding `scheduler.yield()` to a 5-item matcher is overkill. Confirm you agree it's just a TODO comment for now.
