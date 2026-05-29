# SPEC: Input Responsiveness — Terminal and Agent Pane

**Status:** Draft
**Date:** 2026-05-29
**Author:** Agent1
**Scope:** Frontend (CEF renderer, SolidJS). Two input surfaces: the terminal pane (xterm.js + PTY) and the agent pane composer (`<textarea>` + WebSocket).
**Type:** Forward-looking architecture spec + best-practices reference. Not a single PR; an enforceable contract for future input-path changes.

---

## TL;DR

Typing must remain fast **as the app grows**. AgentMux already shipped pointed fixes for terminal echo (`PR #926`, `SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19`) and agent textarea reflow (`docs/analysis/agent-typing-lag-trace-2026-04-12.md` → `field-sizing: content`). Those are reactive: one regression, one chase. This spec captures the **structural rules** that must hold across both surfaces so we don't keep paying the same debugging cost.

Three rules to enforce going forward:

1. **The keystroke-handling task must finish in ≤16 ms (one frame), always.** No work scheduled inside the input event handler may block paint.
2. **Any work that *can* run later, *must* run later.** Use `requestAnimationFrame` for coalesced DOM writes, `scheduler.yield()` / time-sliced batching for any logic that could exceed 50 ms.
3. **The keystroke path must never read layout (`scrollHeight`, `getBoundingClientRect`, `offsetTop`, etc.) after touching style.** This is the single most common regression cause in our history — both terminal and agent pane have been bitten by it.

Plus two surface-specific contracts: terminal — never let xterm.js's write buffer saturate (`flow control`); agent — handle IME composition before treating Enter as submit.

---

## 1. Why now — forward-looking framing

We have two production-grade input surfaces today. As we add features (LSP overlays in the agent pane, more shell-integration hooks in the terminal, voice input, multi-pane orchestration), each surface gains co-tenants on the main thread. Without explicit budgets and structural rules, each new feature pays no per-keystroke tax until the cumulative load shows up as visible lag — at which point we trace, blame, and fix one at a time. The history:

- `agent-typing-lag-trace-2026-04-12.md` — `scrollHeight` reads cost ~22 ms per keystroke after content-visibility'd document virtualization went in.
- `SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` — `writeInFlight` guard fell through small keystroke echoes to the RAF coalescer when any large batch was in flight (PR #926).
- `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` — tab switch perf separately.

Each had a different root cause and a one-line fix. The unifying problem is **we didn't have a contract for "what is allowed inside a keystroke event handler"** — so each new caller assumed it could do whatever it needed. This spec writes that contract down.

---

## 2. Numerical targets

Adopting [Core Web Vitals INP](https://web.dev/articles/inp) as the external benchmark, plus a stricter internal "snappy" target consistent with [`SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md`](./SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md):

| Surface | Metric | External "Good" (Web Vitals) | Internal "snappy" |
|---|---|---|---|
| Agent pane textarea | per-keystroke INP | P75 ≤ 200 ms | **P50 ≤ 16 ms, P95 ≤ 50 ms** |
| Terminal pane | echo latency (keystroke → xterm paint) | — (CWV doesn't cover) | **P50 ≤ 25 ms, P95 ≤ 50 ms** (already met per PR #926 bench) |
| Either | longest blocking task on input path | < 50 ms (avoid "long task" classification) | < 16 ms (avoid frame drop) |

Web Vitals' [own data shows keyboard interactions are 56% slower than pointer (75 ms vs 49 ms P75)](https://web.dev/articles/inp) and 7.4% of keyboard interactions are classified "poor" — typing is the hardest case to keep fast. We need explicit discipline, not best-effort.

---

## 3. Current state — what exists, where the contracts live

### 3.1 Terminal pane (`frontend/app/view/term/`)

| File | Role |
|---|---|
| `term.tsx` | xterm.js mount, sizing, theming |
| `termwrap.ts` | Owns the `terminal.write()` schedule — fast-path bypass for small echoes, RAF coalescer for batches |
| `term-rpc.tsx` | PTY data delivery via WS |
| `termagent.ts` | Shell-integration OSC handling (env block, `muxlog` etc.) |

Key invariants already in code:

- **Fast path** (`scheduleRafWrite()` @ `termwrap.ts:494`): `data.length ≤ 512 B` AND `rafBuffer.length === 0` → immediate `terminal.write()`. Bypasses the RAF coalescer for single-character echoes.
- **`writeInFlight` is NOT in the fast-path predicate** anymore (PR #926). xterm.js serializes `write()` internally; the guard was redundant and caused jitter when any large batch was in flight.
- **Perf marks** (`term-keypress`, `term-echo-render`, `term-raf-write`) per `SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md` §2.2.
- **Bench tooling**: `tools/tests/bench-term-echo.mjs` gives a CI-runnable echo-latency P95 number.

### 3.2 Agent pane composer (`frontend/app/view/agent/components/AgentFooter.tsx`)

| Aspect | Current implementation |
|---|---|
| Textarea ownership | **Uncontrolled** — DOM owns the value. Component doesn't re-render on keystroke. Value read via ref on submit. (`AgentFooter.tsx:230-251`) |
| Auto-grow | CSS `field-sizing: content` on `.agent-input`. No `scrollHeight` reads. ([web reference](https://modern-css.com/auto-growing-textarea-without-javascript/)) |
| onInput cost | One boolean check + at-most-once-per-frame `requestAnimationFrame` enqueue. Target <2 ms per keystroke. |
| Scroll-on-type | RAF callback uses `scrollTo({top: Number.MAX_SAFE_INTEGER})` — the browser clamps internally. No JS `scrollHeight` read. |
| Slash autocomplete | Signal-driven dropdown, doesn't control the textarea value. |
| IME composition | **NOT handled** — Enter key with `event.isComposing === true` will incorrectly submit while CJK candidates are open. (Gap, see §6.) |

### 3.3 Cross-cutting: SolidJS reactivity

AgentMux is on SolidJS. Unlike React, [Solid does not re-render the component on every input](https://github.com/solidjs/solid/discussions/416) — fine-grained reactivity only re-runs JSX expressions and effects that depend on a signal that changed. **For an uncontrolled input, the component function executes zero times per keystroke.** This is structurally cheaper than React's controlled-input baseline; we should not give it up.

---

## 4. Three structural rules (contract for all future input-path code)

### Rule 1 — The keystroke handler runs in ≤16 ms, always.

**Why:** one frame = 16 ms @ 60 Hz. Any handler that exceeds that drops a frame and adds visible latency.

**How to apply:**

- The keystroke handler may **dispatch** work (enqueue a RAF, fire-and-forget a WS message, push to a buffer). It may not **wait** for that work.
- Synchronous calls to user code inside the handler must be measured. If a new feature wants to hook into the keystroke handler, owner must show a perf-mark span proving P95 < 5 ms for the feature alone.
- If a feature needs CPU > 16 ms (parsing, regex, autocomplete ranking), it must yield — see Rule 2.

### Rule 2 — Anything that can run later, must run later.

**Why:** the cheapest task is one that runs after paint. The browser will composite the new character in the current frame; everything else can wait.

**How to apply:**

| Work type | Defer mechanism |
|---|---|
| DOM writes that depend on the new character (autocomplete UI, scroll-to-bottom) | Single `requestAnimationFrame`, coalesced across multiple keystrokes |
| Non-DOM logic that might take >50 ms (full slash-command match, semantic search, lint) | [`scheduler.yield()` with 50 ms time-slice deadline](https://web.dev/articles/optimize-long-tasks) |
| Background analytics / telemetry | `scheduler.postTask({ priority: 'background' })` or `requestIdleCallback` |
| State mutations that drive distant subscribers | RAF + microtask coalescing; never spread reactive fan-out across a single keystroke if it crosses a pane boundary |

[`scheduler.yield()`](https://developer.chrome.com/blog/use-scheduler-yield) beats `setTimeout(0)` and the older [`isInputPending()`](https://web.dev/isinputpending/) — continuations are scheduled ahead of newly-queued tasks of the same priority, so deferred work doesn't get starved.

### Rule 3 — Never read layout after touching style on the keystroke path.

**Why:** forced synchronous reflow. This was the literal cause of the 22 ms agent typing lag — `el.style.height = 'auto'` then read `el.scrollHeight` → browser must recompute layout immediately.

**Banned in keystroke and input event handlers (and any RAF callback they enqueue) AFTER a style mutation:**

- `scrollHeight`, `scrollTop`, `scrollWidth`, `scrollLeft`
- `offsetHeight`, `offsetTop`, `offsetWidth`, `offsetLeft`
- `clientHeight`, `clientTop`, `clientWidth`, `clientLeft`
- `getBoundingClientRect()`, `getClientRects()`
- `window.getComputedStyle()`
- `IntersectionObserver` callback that triggers more style writes

**Safe alternatives:**

- Auto-grow → CSS `field-sizing: content` (already in use; keep it). See [CSS field-sizing](https://blog.kalan.dev/en/frontend/css-field-sizing/).
- Scroll-to-bottom → `scrollTo({ top: Number.MAX_SAFE_INTEGER })`, let the browser clamp.
- Visibility checks → `IntersectionObserver` driving a signal read on the *next* frame, not the current handler.

**Enforcement:** ESLint rule + grep-based CI check (see §7). If a future PR needs to read layout for a legitimate reason, the read must happen in a `requestAnimationFrame` callback that runs in a *different* frame than the style write — never in the keystroke handler itself.

---

## 5. Terminal-pane contract (`xterm.js` specifics)

### 5.1 Never let the write buffer saturate

The [xterm.js flow control guide](https://xtermjs.org/docs/guides/flowcontrol/) is unambiguous: when the backend out-produces the renderer, keystrokes get starved because they share the same event-loop window as `write()` processing. Symptoms: terminal stops echoing, appears frozen, eventually buffer overflow.

**Today:** AgentMux runs PTY → WebSocket → `term-rpc.tsx` → `termwrap.scheduleRafWrite()`. No ACK-based backpressure is implemented end-to-end. The 50 MB internal cap is the only safety net.

**Recommendation:**

1. Add an ACK-based flow-control pass when high-throughput producers become common (long agent transcripts, `cat` of large files, build output streaming). Pattern: every Nth chunk, use `term.write(chunk, ack)` instead of the fire-and-forget form, and have `agentmux-srv`'s PTY pump pause when too many ACKs are outstanding.
2. Add a P95 write-buffer-depth metric to the existing perf HUD. Alarm at >5 MB.

### 5.2 Cap GPU work to avoid thermal lag

Per [xterm.js issue #5447](https://github.com/xtermjs/xterm.js/issues/5447): the WebGL renderer can drive sustained GPU load high enough to cause CPU thermal throttling on laptops — at which point *everything* gets slow, including the keystroke loop.

**Recommendation:**

- Add a soft `targetFps` coalescer around `term.write()` for non-input data. First chunk renders immediately (keystroke echo / interactive feedback); subsequent chunks within a `1000 / targetFps` window batch. Configurable; default 60 fps on AC power, 30 fps on battery (detect via `navigator.getBattery()`).
- Never coalesce small writes that arrive from `handleTermData` (user input echo) — those must always take the fast path.

### 5.3 Renderer choice

WebGL renderer is faster than canvas in almost all cases. Canvas renderer has a known [perf pathology with very wide containers](https://github.com/xtermjs/xterm.js/issues/4175). Keep WebGL as default; document the fallback path.

---

## 6. Agent-pane contract (textarea specifics)

### 6.1 Preserve the uncontrolled-DOM pattern

The `AgentFooter.tsx` textarea is intentionally uncontrolled (DOM owns value, read via ref on submit). This is **a load-bearing perf decision**, not a stylistic one — see `agent-typing-lag-trace-2026-04-12.md`. Any future feature that wants to read the textarea value reactively per-keystroke (e.g., live preview, real-time validation) must do so via a debounced effect, NOT by converting `value={signal()}`.

**Rationale:** Solid's fine-grained reactivity means controlled `value={signal()}` doesn't cause a component re-render — but it does cause every dependent effect to re-evaluate, every dependent JSX expression to re-run, and (worse) any `<For>`-driven sibling that derives from the same signal to rebuild. For a textarea inside an agent pane that also renders a virtualized document above, this is a meaningful tax.

### 6.2 IME composition handling (GAP)

**Current bug:** the agent composer treats Enter as submit unconditionally. For CJK / Vietnamese / any IME user, pressing Enter to *confirm a composition candidate* would submit the in-progress preedit string.

**Fix:** in the Enter key handler, check `event.isComposing` (and `event.keyCode === 229` as a Safari-compat fallback). Skip submit when composition is active. See [MDN composition events](https://developer.mozilla.org/en-US/docs/Web/API/CompositionEvent) and [the Firefox IME handling guide](https://firefox-source-docs.mozilla.org/editor/IMEHandlingGuide.html).

```ts
onKeyDown={(e) => {
  if (e.key === 'Enter' && !e.shiftKey
      && !e.isComposing && e.keyCode !== 229) {
    submit();
    e.preventDefault();
  }
}}
```

**Also:** any reactive effect that fires on textarea input (slash-autocomplete prefix tracker today, future grammar / lint highlighters) must skip work during `compositionstart`...`compositionend`. Pattern:

```ts
let composing = false;
textareaRef.addEventListener('compositionstart', () => composing = true);
textareaRef.addEventListener('compositionend',   () => { composing = false; /* now fire deferred work */ });
textareaRef.addEventListener('input', (e) => {
  if (composing) return; // intermediate IME state
  // ... real input handling
});
```

This is **a meaningful UX bug today** for any CJK user. Should ship as a small targeted PR ahead of any larger composer rework.

### 6.3 Slash-autocomplete and future overlays

`SlashAutocomplete.tsx` matches against a small fixed list, costs are negligible today. As we add larger completion sources (history, agent-specific commands, semantic match), the matcher must:

- Run in the deferred (RAF) phase, not in the keystroke handler.
- Time-slice with `scheduler.yield()` if candidate set ≥ 200 entries or per-match cost > 1 ms.
- Cache the active filter result; only re-rank on prefix change, not on every keystroke.

### 6.4 Voice input integration

[`SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md`](./SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md) introduced voice-typed input. The voice handler currently writes directly into `textareaRef.value`. Two rules:

- Voice-injected text must follow the same RAF-coalesced scroll path as keystroke text (it does today).
- If voice transcripts arrive at >60 Hz (some providers stream per-word), throttle injection to one RAF tick. Don't fire one `input` event per word.

---

## 7. Instrumentation, CI, and regression discipline

### 7.1 Always-on perf marks (extend existing pattern)

Add to the existing `term-keypress` / `term-echo-render` / `term-raf-write` set:

| Mark pair | Span | What it catches |
|---|---|---|
| `agent-keystroke` | `onInput` entry → return | Agent textarea handler cost |
| `agent-input-raf` | RAF enqueue → callback start | RAF queue depth |
| `agent-input-raf-cb` | RAF callback start → end | Coalesced scroll cost |
| `agent-submit` | submit handler entry → WS send return | Submit-path cost |

Cost per mark: ~50 ns. Surfaced in the dev-mode Perf HUD per `SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md` §A1.

### 7.2 Per-surface bench

- Terminal: extend `tools/tests/bench-term-echo.mjs` to capture P95 echo latency in CI on every PR that touches `frontend/app/view/term/**`.
- Agent: write a parallel `tools/tests/bench-agent-keystroke.mjs` that drives the agent pane via the App API (`pane.open` → focus → simulated keydown sequence → measure `agent-keystroke` mark P95).

### 7.3 ESLint / grep guardrails

Add an ESLint custom rule (or CI grep step) that flags the following in `frontend/app/view/term/**` and `frontend/app/view/agent/components/AgentFooter.tsx`:

- Direct reads of `scrollHeight`, `offsetHeight`, `getBoundingClientRect`, `getComputedStyle` inside any function attached to `onInput`, `onKeyDown`, `onKeyPress`, `onKeyUp`, `onBeforeInput`, or any function called from such a handler.
- Any new `setTimeout(fn, 0)` — should be `scheduler.yield()` or `queueMicrotask` per intent.
- `value={...signal()}` on the agent textarea — must remain uncontrolled.

Violations get a `// allow: <reason>` escape hatch for the rare legitimate case, but each must be reviewed in PR.

### 7.4 Long-task observer

Subscribe to `PerformanceObserver` for `longtask` entries in dev and prod (with a sample rate). Log any long task ≥100 ms with attribution (script URL, container element if possible). One alarm per session.

The [Long Animation Frames API (LoAF)](https://web.dev/articles/optimize-long-tasks) is more useful than the legacy `longtask` entry type for attribution — adopt when AgentMux's CEF Chromium baseline supports it.

---

## 8. What this spec does NOT cover

- **GPU compositor frame timing** — separate concern. Handled when CPU-bound bottlenecks are eliminated. (Same boundary as `SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md`.)
- **Backend sidecar query latency** — `agentmux-srv` isn't on the synchronous keystroke path.
- **Network latency for remote PTY** — out of scope (we run local PTYs). If/when remote PTY ships, the xterm.js [network latency guidance](https://github.com/xtermjs/xterm.js/issues/887) becomes load-bearing.
- **Specific feature roadmap** — this spec is the contract, not a list of PRs. Individual features cite this spec for their input-path implementation.

---

## 9. Action items (what concretely happens next)

| # | Item | Priority | Notes |
|---|---|---|---|
| 1 | Ship IME-composition fix for agent textarea Enter handler | **HIGH** | Real user-facing bug for CJK / IME users. Tiny diff. |
| 2 | Add `agent-keystroke` + RAF perf marks to `AgentFooter.tsx` | Medium | Mirrors terminal pattern. Enables agent bench. |
| 3 | Write `tools/tests/bench-agent-keystroke.mjs` | Medium | Counterpart to `bench-term-echo.mjs`. |
| 4 | Add ESLint rule / CI grep for layout reads in input handlers | Medium | Prevents the 22 ms reflow regression from recurring. |
| 5 | Adopt `scheduler.yield()` in slash-autocomplete matcher (preemptively) | Low | Before the matcher grows. |
| 6 | Add `targetFps` coalescer to `termwrap.ts` non-input write path | Low | Battery/thermal benefit on laptops. Measure first. |
| 7 | Implement ACK-based PTY flow control end-to-end | Low | Only if profiling shows write-buffer pressure. |

Items 1, 2, 3, 4 are the "do these now" set — they're cheap, they prevent known regression patterns, and item 1 fixes an actual current bug. Items 5-7 are conditional on profiling evidence.

---

## 10. References

### Core Web Vitals / INP
- [Interaction to Next Paint (INP) — web.dev](https://web.dev/articles/inp) — official metric definition, thresholds, three-phase breakdown
- [Optimize Interaction to Next Paint — web.dev](https://web.dev/articles/optimize-inp) — practical guidance
- [Optimize long tasks — web.dev](https://web.dev/articles/optimize-long-tasks) — `scheduler.yield()` recommended pattern

### Main-thread scheduling
- [Use scheduler.yield() to break up long tasks — Chrome for Developers](https://developer.chrome.com/blog/use-scheduler-yield)
- [Better JS scheduling with isInputPending() — web.dev](https://web.dev/isinputpending/) — superseded by `scheduler.yield()`; useful background
- [scheduling-apis explainer — WICG](https://github.com/WICG/scheduling-apis/blob/main/explainers/yield-and-continuation.md)

### Terminal (xterm.js)
- [xterm.js Flow Control guide](https://xtermjs.org/docs/guides/flowcontrol/) — canonical ACK pattern
- [xterm.js issue #280 — refresh limit / RAF binding](https://github.com/xtermjs/xterm.js/issues/280)
- [xterm.js issue #5447 — GPU thermal throttling + `targetFps` coalescing proposal](https://github.com/xtermjs/xterm.js/issues/5447)
- [xterm.js issue #887 — network latency mitigations](https://github.com/xtermjs/xterm.js/issues/887)
- [xterm-benchmark](https://github.com/xtermjs/xterm-benchmark) — official benchmark harness

### Textarea / forced reflow
- [Auto-Resize Textarea in CSS: field-sizing: content — modern-css.com](https://modern-css.com/auto-growing-textarea-without-javascript/)
- [Layout thrashing: what is it and how to eliminate it — DEV](https://dev.to/aayla_secura/layout-thrashing-what-is-it-and-how-to-eliminate-it-n2j)

### IME / composition events
- [MDN — CompositionEvent](https://developer.mozilla.org/en-US/docs/Web/API/CompositionEvent)
- [Firefox IME handling guide](https://firefox-source-docs.mozilla.org/editor/IMEHandlingGuide.html)
- [Handling IME events in JavaScript — Stum](https://www.stum.de/2016/06/24/handling-ime-events-in-javascript/) — cross-browser quirks
- [React #8683 — composition events in controlled components](https://github.com/facebook/react/issues/8683)

### SolidJS reactivity
- [Solid discussion #416 — uncontrolled inputs in Solid](https://github.com/solidjs/solid/discussions/416)
- [Two-way Binding can be a One-way Street — Michael Rawlings (DEV)](https://dev.to/mlrawlings/two-way-binding-can-be-a-one-way-street-1o3)

### General input-handling principles
- [High-performance input handling on the web — Nolan Lawson](https://nolanlawson.com/2019/08/11/high-performance-input-handling-on-the-web/) — RAF for writes, layout reads stay cheap

### AgentMux internal references
- [`SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md`](./SPEC_TERMINAL_LATENCY_BENCHMARK_2026_05_19.md) — PR #926 fix, bench tooling
- [`SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md`](./SPEC_PERFORMANCE_INSTRUMENTATION_AND_OPTIMIZATION.md) — perf-mark scheme, dev-mode Perf HUD
- [`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`](./SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md) — sibling perf spec
- [`SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md`](./SPEC_USER_INPUT_VISIBILITY_AND_STARTUP_COLLAPSE_2026_05_24.md) — user-message rendering
- [`SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md`](./SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md) — voice-driven input integration
- [`docs/analysis/agent-typing-lag-trace-2026-04-12.md`](../analysis/agent-typing-lag-trace-2026-04-12.md) — the 22-ms `scrollHeight` incident
- `frontend/app/view/term/termwrap.ts` — terminal write scheduler (fast path + RAF coalescer)
- `frontend/app/view/agent/components/AgentFooter.tsx` — agent composer textarea
