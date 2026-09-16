# Editor: the first keystroke after focusing is accepted but not rendered

**Status:** implemented — root cause confirmed by live trace and fixed; C1 was
correct, C2 and C3 are ruled out. See §3 and §6.
**Date:** 2026-09-15
**Severity:** HIGH — raised from Medium once the mechanism was confirmed. This
is **silent data loss**, not a rendering annoyance: the first character typed
into any file is discarded outright, not merely unpainted. Typing one character
and saving would have saved the file without it.
**Reported by:** repo owner, on CEF 152 (v0.56.0).
**Related:** `docs/retro/retro-terminal-consecutive-period-input-loss-2026-09-15.md`
(unrelated cause — that one is font shaping; filed separately so the two are not
conflated).

---

## 1. Symptom, and the one detail that narrows it

1. Click into the editor's text area.
2. Type. **Nothing appears.**
3. **But the dirty indicator appears next to the filename in the tab.**
4. Click into the editor again, type — now it works normally.

Step 3 is the whole diagnosis. The keystroke **is** reaching CodeMirror: the
document changed, the model marked the tab dirty, and the tab re-rendered. Only
the *visible text* is missing.

That rules out the obvious first guesses:

- **Not a focus/keyboard-routing bug.** An unfocused editor would not update the
  document at all, so the dirty flag could not flip.
- **Not the CEF 152 shaping bug** from the companion retro. That one renders
  *blanks of the right width* for repeated punctuation only, and affects every
  text surface. This affects the first keystroke of any character, in the editor
  only.
- ~~**Not data loss.**~~ **WRONG — this was the one wrong call in the original
  analysis.** The character reaches CodeMirror, but the view is then rebuilt
  from the on-disk content and the edit is destroyed with it. It never reaches
  disk. Q3 is answered: this is data loss.

So: **CodeMirror's state updates, and is then thrown away.** (The original
framing, "its view does not paint", was a reasonable reading of the symptom but
pointed at rendering rather than at a rebuild.)

## 2. What the code shows

`frontend/app/view/editor/editor-view.tsx`:

- `setupEditor()` (~line 386) constructs `new EditorView({ state, parent: container })`.
- **Nothing ever calls `cmView.focus()`.** A grep for `.focus()` across
  `frontend/app/view/editor/` returns hits only in `editor-tab-strip.tsx:106`
  (the rename input) and `file-tree.tsx:297` (the tree's rename input). The
  editor itself is focused only by the user's click.
- A `createEffect` (~line 466) re-runs on `model.activeIdAtom()`,
  `model.loadingAtom()` and `containerRef()`, and on each run either restores a
  snapshot via `cmView.setState(saved)` or rebuilds the editor. `setState`
  **replaces the whole view state**, including anything typed since the snapshot
  was taken.

`frontend/app/view/editor/editor-model.ts`:

- `dirtyAtom` (line 331) is `this.activeTabAtom()?.dirty ?? false`.
- The **pane title itself is derived from it** (lines 360–369):
  `return this.dirtyAtom() ? `${fp} *` : fp;`

So the first keystroke flips `dirty`, which changes the pane title, which
re-renders pane chrome. There is a real reactive cascade fired by exactly the
keystroke that goes missing.

Note the cascade is **not** a tab-strip *layout* change — the close/dirty
affordance already reserves its box and only animates opacity (see C3 below,
ruled out). What remains is the title-derived re-render, which is what makes C1
worth instrumenting.

## 3. Candidate mechanisms, in rough order

Each is independently testable. C3 has since been ruled out by inspection; C1
and C2 remain open.

**C1 — CONFIRMED. The tab-switch effect re-runs on first edit and overwrites the view.**
If flipping `dirty` causes the `createEffect` at ~466 to re-run (directly, or
because `loadingAtom`/`containerRef` churn during the pane-chrome re-render),
its `cmView.setState(saved)` branch restores a state snapshotted *before* the
keystroke. The document the model already marked dirty stays dirty, but the view
reverts — which is exactly the observed split. Also explains why the second
attempt works: by then the effect has settled and does not re-run.
*Test:* log every entry to that effect with the triggering atom, type one
character into a freshly focused editor, see whether it re-runs.

**C2 — RULED OUT. Stale measurement on a zero-sized container.**
CodeMirror measures its content on creation. If `setupEditor` runs while the
container is hidden or zero-height (the effect's own comment mentions a
"first-open blank-preview race"), the view can hold stale geometry and skip
painting until something forces a re-measure — a click being one such thing.
*Test:* call `cmView.requestMeasure()` after mount and see whether the first
keystroke renders.

**C3 — Layout shift from the dirty affordance — RULED OUT.**
The reporter's original hypothesis was that the dirty indicator / close icon
appearing shifts tab-strip layout on exactly that keystroke, leaving
CodeMirror's cached geometry stale for the frame that should have painted.
**Disproven by code inspection** (codex P2 on PR #3250), recorded here so nobody
spends time re-testing it:

- `.pane-tab-close` is `flex: 0 0 16px; width: 16px; height: 16px` with
  `opacity: 0` — it **permanently reserves its box**; only opacity animates
  (`transition: opacity 80ms`).
- It is rendered whenever `onClose` exists (`<Show when={props.onClose}>`),
  never conditioned on dirty.
- `editor-tab-strip.tsx:72` passes `getAttention={(tab) => tab.dirty}`, and the
  `--attention` modifier only changes opacity.

The clean→dirty transition therefore changes no tab-strip geometry, and the
experiment originally proposed here (reserve the space permanently) would have
been a no-op against an already-reserved box.

**This does not rule out C1.** Dirty still drives a real reactive change
elsewhere: `editor-model.ts:369` derives the *pane title* from it (`${fp} *`).
That path is independent of tab-strip layout, and is the plausible trigger for
C1's effect re-run. What is dead is specifically the *layout-shift* explanation,
not "the dirty flip causes reactive churn".

## 3b. Resolution — what the live trace showed

Instrumented the running CEF 152 dev build (`docChanged` on every update,
`setupEditor` on every rebuild) and reproduced. The trace, to the millisecond:

```
15:12:59.776  setupEditor rebuild {gen:1, docLen:0}     <- file opened, view gkf9i
15:13:01.642  docChanged {viewId:"gkf9i", docLen:1}     <- FIRST KEYSTROKE lands
15:13:01.643  setupEditor rebuild {gen:2, docLen:0}     <- 1ms later: rebuilt from disk
15:13:03.482  docChanged {viewId:"gz0b4", docLen:1}     <- 2nd click, new view, works
```

**C2 is ruled out** by the same trace: `containerH: 1053` at construction, so the
container always had real geometry. Measurement was never the problem, and a
`requestMeasure()` fix built on that premise was written, tested, and discarded.

**C1 is confirmed**, with the precise trigger being narrower than the original
guess. The effect's reactive deps are `activeIdAtom`, `loadingAtom` and
`containerRef` — dirty is not among them. The link is that `loadingAtom` was a
**bare arrow function, not a memo**:

```ts
this.loadingAtom = () => {
    const tab = this.activeTabAtom();
    return tab != null && !tab.contentLoaded && tab.loadError == null;
};
```

Reading it subscribed the caller to the whole **tab object**, not the boolean.
So the chain is:

1. first keystroke → `onContentChange()`
2. → dispatches `MarkDirty`, guarded by `if (!this.dirtyAtom())` — **this is why
   only the FIRST keystroke is affected**; later edits dispatch nothing
3. → tab object changes → `activeTabAtom` emits
4. → un-memoized `loadingAtom` propagates even though `loading` is `false`
   before and after
5. → effect re-runs → CodeMirror rebuilt from `contentAtom()` (on-disk content)
6. → the typed character is destroyed with the old view

**Fix:** memoize `loadingAtom` via `useBlockAtom`/`createMemo`, so tab mutations
that leave `loading` unchanged no longer reach the effect.

**Separate defect found and fixed alongside:** `setupEditor` is async and awaits
`loadLanguage()` *between* destroying the old view and constructing the new one,
so two calls starting before either constructs both skip the destroy and both
append an `EditorView` to the same container — orphaning one (alive, in the DOM,
able to take focus, referenced by nothing, never destroyed). It fired on every
open, ~2ms apart. Closed with a generation guard. The trace shows a single view
id throughout, so this was **not** the cause of the first-keystroke bug — but it
is a real bug.

## 4. Proposed work

1. **Instrument first** — log every `createEffect` re-entry with its trigger, and
   log `cmView.state.doc` before/after the first keystroke. Confirm which of
   C1/C2 fires before writing a fix. This bug has an easy plausible story per
   mechanism, and the companion retro is a case study in what picking one on
   plausibility costs.
2. **Fix the confirmed mechanism only.**
3. **Regression test** — a vitest case that mounts the editor, dispatches one
   input, and asserts the rendered content matches the document. The current
   suite has nothing covering first-keystroke rendering.
4. **Consider focusing the editor on mount/tab-activate.** Independently
   reasonable (VS Code focuses the editor when you open a file) and may make the
   whole class of first-interaction bugs unreachable — but it is a *behavior*
   change, so decide it on its own merits rather than smuggling it in as a fix.

## 5. Open questions

- ~~**Q1.**~~ **Moot.** It was designed to separate C2 from C1; the trace did
  that directly and ruled C2 out.
- **Q2.** Does it reproduce on CEF 148? Determines whether this is another 152
  regression or a long-standing bug only noticed now.
- **Q3. ANSWERED — no, and severity rose accordingly.** The character is
  discarded when the view is rebuilt, so it never reaches disk. This was silent
  data loss for every first edit.
- **Q4.** Does it reproduce in a pane that was never re-rendered — e.g. open a
  file, click away to another app, click straight back into the text area?
