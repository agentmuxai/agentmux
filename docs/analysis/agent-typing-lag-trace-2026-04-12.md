# Agent Pane Typing Lag — Trace Analysis & Fix

**Date:** 2026-04-12
**Version under test:** 0.33.105 portable
**Trace file:** `~/Desktop/Trace-20260412T232248.json.gz` (17 MB uncompressed)
**Trace window:** 8.28 seconds, 34 keystrokes, no agent streaming, no WebSocket traffic
**Verdict:** Single cause identified. One function, one line. Three-line CSS fix.

---

## 1. Context

The user reported that typing in the agent pane composer still feels slow after the
entire 5-PR ultra-long-sessions plan shipped. We had already ruled out memory pressure
(stable 121 MB working set, no leak). The user captured a Chrome DevTools performance
trace while typing and asked for analysis.

This document captures the analysis process, the finding, and the fix.

---

## 2. Trace analysis — three passes

All analysis done with plain Node.js scripts against the unzipped `.json`. No DevTools
UI used — the JSON format is self-describing and the numbers are more trustworthy than
a visual flame graph when chasing a specific bottleneck.

### Pass 1 — shape of the trace

| Metric | Value |
|---|---|
| Window | 8.28 s |
| Total events | 61,674 |
| Main-thread complete events (`ph: "X"`) | 40,480 |
| RunTask total | 5,514 ms (67% of wall time busy) |
| EventDispatch total | 3,859 ms |
| WidgetBaseInputHandler total | 1,747 ms |
| Layout total | **1,521 ms (118 layouts, avg 12.89 ms)** |
| V8 garbage collection (all phases) | ~1,600 ms (20% of wall time) |

20% of the wall time was spent in GC. That's a *very* high rate for an idle typing
session and indicates heavy allocation on the hot path.

### Pass 2 — event dispatches by type

| Event type | Count | Total | Avg | Max |
|---|---:|---:|---:|---:|
| `keydown` | 34 | 12.4 ms | **0.36 ms** | 0.8 ms |
| `beforeinput` | 34 | 0.2 ms | 0.00 ms | — |
| `keypress` | 34 | **1,534 ms** | **45.12 ms** | 60.0 ms |
| `textInput` | 34 | **1,532 ms** | **45.07 ms** | 59.9 ms |
| `input` | 34 | **739 ms** | **21.75 ms** | 32.7 ms |
| `keyup` | 11 | 0.1 ms | 0.01 ms | — |
| `pointermove` | 119 | 11.4 ms | 0.10 ms | — |

**Finding #1: `keydown` is fast. `keypress`/`textInput`/`input` are catastrophic.**

This rules out the `window.addEventListener("keydown", ...)` global handler I added
in PR #340/#341 for Ctrl+B/Ctrl+F as the primary bottleneck. That handler runs in
`keydown`, which is averaging 0.36 ms — the fast half. The ~45 ms per keystroke is
happening *after* keydown, in the input handling chain that runs on the actual
text-input path.

**Finding #2: all three of keypress, textInput, and input are slow.** They're nested
inside each other (`textInput` fires inside `keypress`, `input` fires inside
`textInput`). So the user-visible latency per keystroke is ~45–60 ms, not 45+45+21.

### Pass 3 — drill into the slowest keypress

Picked the max-duration keypress (60.0 ms) and dumped every nested event on the same
thread during its window:

```
60.0 ms   EventDispatch  {type: "keypress"}
├─ 22.4 ms  Layout            (dirtyObjects: 21, partialLayout: true)   ← forced layout #1
└─ 59.9 ms  EventDispatch  {type: "textInput"}
   └─ 25.9 ms  EventDispatch  {type: "input"}
      └─ 25.8 ms  v8.callFunction
         └─ 25.8 ms  FunctionCall  xZ  at index-BSUSLm8h.js:2 col 36563
            ├─ 22.1 ms  Layout          (dirtyObjects: 18, partialLayout: true)   ← forced layout #2
            ├─ GC incremental marking (~5 ms cumulative)
            └─ 2.7 ms  MajorGC (stop-the-world)
```

**Two forced synchronous layouts per keystroke, 22 ms each.** Plus a stop-the-world
major GC pass *during* a single keystroke (2.7 ms). The JavaScript function `xZ` at
`index-BSUSLm8h.js:2` column 36563 runs for 25.8 ms, triggering the second layout
from inside its body.

### Aggregated hot functions across the 8-second window

```
751.1 ms  n=335  xZ  at col 36563
  6.4 ms  n= 34  ?   at col 16995
  4.8 ms  n= 69  O   at col 53004
  4.0 ms  n= 11  open.noReconnect.wsConn.onmessage  col 18898
  1.7 ms  n=119  listener  col 4557
```

**`xZ` is called 335 times for 34 keystrokes (~10 calls each) and owns 751 ms of the
trace.** That's 9% of wall time from ONE function, hit from typing.

### Sanity check — is the stream flush interfering?

```
WebSocket 'message' dispatches: 0
RequestAnimationFrame events: 0
Background Layouts: 16 (132 ms)
In-keypress Layouts: 102 (1,389 ms)
```

**No.** The agent was not streaming. No WebSocket traffic, no RAF callbacks, no
background work competing with typing. The lag is caused entirely by the keystroke
handler itself. 102 of 118 layouts are inside keypress events. The background
layouts are cheap (avg 8 ms, incidental).

---

## 3. Source resolution — what is `xZ`?

The minified function name `xZ` at `index-BSUSLm8h.js:2 col 36563` is the JavaScript
in the input handler path. Searched the agent pane source for layout-property reads:

```
$ grep -rn 'scrollHeight\|offsetHeight\|getBoundingClientRect\|style.height' \
    frontend/app/view/agent

frontend/app/view/agent/components/AgentFooter.tsx:24:    el.style.height = "auto";
frontend/app/view/agent/components/AgentFooter.tsx:25:    el.style.height = el.scrollHeight + "px";
```

`AgentFooter.tsx` has the autoGrow function used by the composer's `onInput`:

```ts
// AgentFooter.tsx — current implementation
const autoGrow = (el: HTMLTextAreaElement) => {
    el.style.height = "auto";                  // invalidates layout
    el.style.height = el.scrollHeight + "px";  // ← read forces sync layout
};

const handleInput = (e: Event) => {
    // Only auto-grow — do NOT update a signal here. Keeping the DOM as
    // the source of truth means keystrokes don't trigger re-renders.
    autoGrow(e.target as HTMLTextAreaElement);
};
```

The textarea is already uncontrolled (PR #334's fix, preserved). The only hot-path
work is `autoGrow`, and it's doing textbook forced-synchronous-layout:

1. Write `style.height = "auto"` — invalidates current layout
2. Read `el.scrollHeight` — the browser must synchronously run layout to produce
   an accurate scrollHeight, which flows the flex parent, the content-visibility
   regions above, and ~21 layout objects in the pane
3. Write `style.height = "<N>px"` — marks layout dirty again, browser schedules
   a second layout as part of the normal render pipeline

In a simple page, step 2's forced layout is ~0.5 ms. In the agent pane — a flex
column containing a scrollable document view with thousands of
`content-visibility: auto` nodes — it's **22 ms** because the browser has to walk
the whole pane to propagate the flex container's intrinsic-size recalc through its
children.

The ironic bit: the comment above the function reads *"avoid re-rendering the
component tree on every keystroke."* PR #334 correctly made the textarea
uncontrolled so SolidJS doesn't re-render on keystroke — but the real cost wasn't
the SolidJS re-render, it was the **forced layout inside an otherwise cheap event
handler**. The fix that shipped was the right fix for the *previous* lag. This is a
different lag, shadowed underneath.

---

## 4. Why 22 ms for "just 21 dirty objects"?

This number felt off when I first saw it. 21 elements × ~1 ms each = 20 ms is
plausible only if each element triggers something expensive during layout. The
reason it costs so much is specific to this pane:

- The `.agent-input-container` is a flex column inside `.agent-footer`.
- `.agent-footer` is the last flex child of the pane; its sibling above is
  `.agent-document-view`, which holds the streamed output.
- The document view contains thousands of children marked `content-visibility: auto`.
  While most of those nodes skip layout normally, the browser still has to
  **revalidate the near-viewport contain-intrinsic-size regions** whenever the
  scrollable area's height changes. A textarea growing shrinks the document view's
  available height.
- That triggers a re-check of every nearby `content-visibility` region, plus a
  partial paint-command-list rebuild for the affected area.
- Additional cost: the flex container's children may have `min-content` or
  intrinsic sizes, which force a second pass.

**Net:** growing the textarea by one character pulls the entire pane through a
layout pass because the flex container's size is coupled to its children's intrinsic
sizes, and the children contain a virtualized but not-entirely-free document view.

This was always going to be slow as the document grew. PR #338's paginated history
and PR #336's content-visibility-based virtualization both made the *document view*
cheap to render, but neither addressed the fact that *mutating the textarea's height*
forces a layout pass that includes the document view's layout subtree.

---

## 5. The fix — three options, ranked

### Option A — `field-sizing: content` (recommended)

Modern Chromium (≥123) supports a CSS property `field-sizing: content` on
`<textarea>` that makes the browser auto-size the element natively. Zero JS. Zero
forced layout from user code. The browser does the growth synchronously with the
text flow inside its own layout engine, where it's free.

**Change to `agent-view.scss`:**

```scss
.agent-input {
    field-sizing: content;  // Chrome 123+, native auto-grow, no JS
    min-height: 20px;
    max-height: 200px;
    overflow-y: auto;
    // ... rest unchanged
}
```

**Change to `AgentFooter.tsx`:**

```ts
// Delete autoGrow entirely.
// Delete handleInput entirely.
// Remove onInput={handleInput} from the <textarea>.
// handleSend's "reset" line becomes: textareaRef.value = "";
//   (no style.height reset — the browser handles it)
```

Net code change: ~15 lines deleted, 1 line added to SCSS.

**CEF version check:** CEF currently tracks Chromium ~133 (from the bundle script
comment: _"snapshot_blob.bin (removed in CEF 133+)"_). `field-sizing` is supported
since Chromium 123, so we're 10 major versions ahead of the requirement. Shipped and
stable.

**Expected impact:**
- `keypress` / `textInput` / `input` duration drops to ~0.3 ms (browser-native,
  matches `keydown`)
- Zero GC pressure from the input path (no new JS allocations per keystroke)
- Zero forced layouts from the input path
- Typing feels instant regardless of document size

### Option B — debounce `autoGrow` with `requestAnimationFrame`

If we need to support CEF versions before Chromium 123 for any reason, defer the
grow work off the critical path:

```ts
let pendingGrow: number | null = null;
const autoGrow = (el: HTMLTextAreaElement) => {
    if (pendingGrow != null) return;
    pendingGrow = requestAnimationFrame(() => {
        pendingGrow = null;
        el.style.height = "auto";
        el.style.height = el.scrollHeight + "px";
    });
};
```

The keystroke handler returns immediately. The character paints on the next frame.
The grow runs on the frame after. Each keystroke still causes a 22 ms layout, but
it no longer blocks the character from appearing on screen.

**Expected impact:**
- Keystroke latency drops from 45 ms to ~1 ms (the actual input handling)
- Main thread still spends ~22 ms per keystroke on layout, but off-critical-path
- Typing feels fast; continuous typing may still drop frames under sustained pressure

### Option C — drop autogrow entirely

Set the textarea to a fixed height with internal scrolling:

```scss
.agent-input {
    height: 60px;        // ~3 lines
    max-height: 200px;
    overflow-y: auto;
    resize: vertical;    // user can drag to resize
}
```

Delete `autoGrow` and `handleInput`. Shift+Enter still inserts a newline; the
textarea scrolls internally. Simplest fix, fewest moving parts, but the UX is
slightly different (no natural growth as you type).

**Expected impact:**
- Keystroke latency drops to the same ~0.3 ms as Option A
- Different user experience — the textarea doesn't grow, which some users prefer
  and some don't

---

## 6. Recommendation

**Option A.** It's the right long-term fix, it removes ~15 lines of JS, and it
delegates the entire auto-grow concern to the browser where it was designed to live.
CEF's Chromium version supports it by a 10-version margin. No debounce tuning, no
RAF ordering, no future regressions from someone editing the JS handler.

Implementation checklist for the fix PR:

1. Add `field-sizing: content` to `.agent-input` in `agent-view.scss`
2. Delete `autoGrow` from `AgentFooter.tsx`
3. Delete `handleInput` from `AgentFooter.tsx`
4. Remove `onInput={handleInput}` from the textarea JSX
5. Change `handleSend`'s reset from `textareaRef.style.height = "auto"` to just
   `textareaRef.value = ""` (the browser handles the height)
6. Bump patch, build portable, run the smoke test, then capture a fresh
   performance trace doing the same typing pattern to verify the fix
7. If the fresh trace shows `keypress` avg below 2 ms, ship it

Verification target: **`keypress` avg event dispatch time < 2 ms**. That's the only
number that matters. A fresh trace with the same typing pattern should drop the
`xZ` hot function from 751 ms to near-zero (no longer called from the input path).

---

## 7. Meta-lesson — why the plan didn't fix this

The ultra-long-sessions plan (PRs #336-#342) correctly identified and fixed several
real perf issues in the document rendering path:

- `content-visibility: auto` virtualization (PR #338)
- Pagination and read_range (PR #336)
- FileStore LRU + write-through (PR #336)
- Ring buffer cap and session stats debouncing (PR #336)
- History prepend with scroll position restoration (PR #338)

But **none of those touched the input handler**, because the plan assumed the
PR #334 "uncontrolled textarea" fix from weeks earlier had already solved the
typing path. That fix was correct for the 2026-03 lag (controlled-state
re-renders). It wasn't wrong — it just wasn't complete, because the bottleneck had
shifted from the SolidJS re-render to the forced layout inside the otherwise-simple
autoGrow helper.

The debugging rule *"if something fails twice with the same error, STOP, propose
alternatives"* didn't fire because each diagnosis was technically a *different*
failure mode:

| Round | Reported symptom | Actual cause | Fix shipped |
|---|---|---|---|
| PR #334 (Mar) | typing lag | controlled textarea re-rendering on every keystroke | uncontrolled textarea |
| Phase 1–4 (Apr) | typing lag at scale | *assumed* DOM size; shipped virtualization + pagination | ultra-long-sessions plan |
| This analysis (Apr 12) | typing lag remains | forced sync layout in autoGrow pulling the virtualized doc into the layout graph | *this doc* |

Three different bottlenecks stacked on top of each other in the same `onInput`
path. The right fix at each step was real. None of them individually was wrong.
The right general lesson is:

**When "typing lag" is reported, always capture a trace and measure before
writing a plan.** A trace would have pointed at autoGrow immediately. Writing a
4-phase feature plan without measurement was the process error.

For the next time: when the user says "typing is slow," the first action is
*record a 10-second performance trace* in a running portable, before proposing any
fix. That was what happened here — it just happened five weeks too late.

---

## 8. Data files

- Source trace: `~/Desktop/Trace-20260412T232248.json.gz`
- Unzipped working copy: `%TEMP%/agentmux-trace.json`
- Analysis scripts: `%TEMP%/analyze-trace.cjs`, `analyze-trace2.cjs`, `analyze-trace3.cjs`

The scripts are throwaway — keep the trace file itself as the primary artifact so
the fix can be re-verified against the same baseline.
