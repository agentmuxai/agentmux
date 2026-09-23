# Report — Terminal "Scroll sensitivity" setting requires a pane reload to take effect

**Date:** 2026-09-22
**Status:** implemented in #3509 — see §8 for what shipped (differs
slightly from §6's proposed fix)
**Area:** `frontend/app/view/settings/sections/terminal-section.tsx` (setting
UI), `frontend/app/view/term/termwrap.ts` (xterm.js construction),
`frontend/app/view/term/term.tsx` +
`frontend/app/view/agent/components/AgentShellSubblock.tsx` (the two live
terminal surfaces)
**Related:**
`docs/specs/SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31.md` (the
spec that added the setting, PR #2876) — §2 and §4.2 of that spec name this
exact limitation and defer it; this report is the investigation for
actually closing it.

---

## 1. Reported symptom

> check the scroll sensitivity setting in settings pane. doesnt appear to
> be working. we need it to update scrolling on the fly (no pane reload)

## 2. Root cause

**This is not a bug in the sense of broken plumbing — it is a documented,
deliberate scope cut that was never followed up.** The setting works; it
just only takes effect for panes constructed *after* the change, because
the value is read exactly once, at `Terminal` construction time, and never
re-applied to already-open panes.

`termwrap.ts`'s constructor:

```ts
// termwrap.ts:148-158
const scrollSensitivitySetting = getSettingsKeyAtom("term:scrollsensitivity")();
const scrollSensitivity =
    typeof scrollSensitivitySetting === "number" && scrollSensitivitySetting > 0
        ? scrollSensitivitySetting
        : 1;
this.terminal = new Terminal({
    ...options,
    cursorBlink: false,
    scrollOnUserInput: false,
    smoothScrollDuration: 0,
    scrollSensitivity,
});
```

`getSettingsKeyAtom("term:scrollsensitivity")()` is called as a **plain
function call**, not inside a `createEffect` — so this line runs once, at
construction, and nothing in the file ever assigns
`this.terminal.options.scrollSensitivity` again after that. Confirmed by
grepping the whole file: `scrollSensitivity` appears at exactly these two
sites (the read and the constructor argument) and nowhere else.

The original spec said as much, explicitly:

> Since this is an `ITerminalOptions` (not `ITerminalInitOnlyOptions`)
> field, xterm.js supports updating it live via
> `terminal.options.scrollSensitivity = ...` without recreating the
> terminal — but plumbing a live-update path for every open pane is out of
> scope for this pass (see §4). The setting takes effect for panes
> opened/reloaded after the change, consistent with how
> `term:fontfamily`/`term:theme` already behave (checked at construction
> time only).
> — `SPEC_TERMINAL_SCROLL_SENSITIVITY_SETTING_2026_08_31.md` §2

So the deferral was intentional and documented at the time. It was never
picked back up, and from the user's side it just looks broken — "I changed
the setting and nothing happened" is a reasonable read of "you have to
close and reopen every terminal pane."

## 3. Confirming `scrollSensitivity` really is live-settable

Checked directly against the vendored xterm.js typings rather than assumed:

```
$ grep -n scrollSensitivity node_modules/@xterm/xterm/typings/xterm.d.ts
269:    scrollSensitivity?: number;
```

Line 269 sits inside `interface ITerminalOptions` (opens at line 26), not
`interface ITerminalInitOnlyOptions` (opens at line 328) — exactly the
distinction the original spec's own §2 note drew, confirmed against the
actual shipped types rather than taken on faith.

## 4. The established pattern this setting should have followed

`term:fontsize` is the sibling setting that **does** already update live,
and it's a direct template for the fix. Two things happen for it:

1. `termViewModel.ts:261-274` derives a reactive `fontSizeAtom` (a
   `createMemo` reading `getSettingsKeyAtom("term:fontsize")()`, folded
   together with a per-block zoom factor).
2. Each of the two components that actually own a `TermWrap` instance
   subscribes to that atom with its own `createEffect` and pushes the new
   value directly into the live terminal:

   **`term.tsx:245-252`:**
   ```ts
   // Update font size in-place when zoom changes
   createEffect(() => {
       const fs = termFontSize();
       const termWrap = model.termRef.current;
       if (termWrap?.terminal && termWrap.loaded) {
           termWrap.terminal.options.fontSize = fs;
           termWrap.handleResize();
       }
   });
   ```

   **`AgentShellSubblock.tsx:311-330`** — the same shape, with an extra
   comment explaining a prior bug (reagentx P2 on #2522) about reading the
   `loaded` signal unconditionally so the effect actually subscribes to it
   on its first run even before `termWrap` exists:
   ```ts
   createEffect(() => {
       const fs = termFontSize();
       const loaded = wrapLoaded();
       if (termWrap?.terminal && loaded) {
           termWrap.terminal.options.fontSize = fs;
           termWrap.handleResize();
       }
   });
   ```

`term:scrollsensitivity` has no equivalent effect in either file. The
settings-*write* side is identical between the two settings — both go
through the same `set(key, value)` helper
(`settings-controls.tsx:11-13`, `void
RpcApi.SetConfigCommand(TabRpcClient, {[key]: value})`) and land in the
same reactive `settingsAtom`, which is exactly what makes the fontSize
comparison a valid proof that the transport isn't the problem: the
write path works today; only the terminal-side read-and-apply is missing
for this one setting.

## 5. Scope: two independent `TermWrap` owners, both affected

```
$ grep -rln 'new TermWrap(' frontend/
frontend/app/view/term/term.tsx
frontend/app/view/agent/components/AgentShellSubblock.tsx
```

Both are affected identically — a plain Terminal pane (`term.tsx`) and an
agent pane's inline shell sub-block (`AgentShellSubblock.tsx`). Neither has
a scroll-sensitivity live-update effect; both already have the fontSize one
to mirror.

`AgentShellSubblock.tsx` does not currently import `getSettingsKeyAtom` at
all (`import { atoms, getSettingsPrefixAtom, staticTabId, MOS } from
"@/app/store/global";` — line 18) since it derives its fontSize atom from
`termViewModel.ts` rather than reading the raw setting directly; a fix
needs to either add that import or (more consistently with the fontSize
pattern) add a small reactive `scrollSensitivityAtom` alongside
`fontSizeAtom` in `termViewModel.ts` and have both consumers read that
instead of touching the raw settings atom directly in two places.

## 6. Proposed fix

Mirror the fontSize pattern exactly — no new mechanism needed, this is a
known-working shape already proven twice in this codebase:

1. In `termViewModel.ts`, add a `scrollSensitivityAtom: () => number`
   alongside `fontSizeAtom` (same `useBlockAtom`-memoized shape), reading
   `getSettingsKeyAtom("term:scrollsensitivity")()` with the same
   `typeof … === "number" && … > 0 ? … : 1` fallback `termwrap.ts` already
   uses, so both call sites agree on one clamping rule instead of each
   re-deriving it.
2. In `term.tsx`, add a `createEffect` beside the existing font-size one
   (`term.tsx:245-252`) that sets
   `termWrap.terminal.options.scrollSensitivity = model.scrollSensitivityAtom()`
   under the same `termWrap?.terminal && termWrap.loaded` guard. No
   `handleResize()` call needed — unlike font size, sensitivity doesn't
   change cell geometry.
3. Do the same in `AgentShellSubblock.tsx` beside its own font-size effect
   (`AgentShellSubblock.tsx:317-330`), reading the unconditional `wrapLoaded()`
   signal first for the same subscription-ordering reason that effect's own
   comment already documents.
4. `termwrap.ts`'s constructor keeps its current read-at-construction
   logic unchanged (newly-opened panes still need a correct initial value
   before any effect has run) — this is additive, not a replacement.

## 7. Out of scope for this report

- Writing the fix itself — this report is the investigation only, per the
  request that prompted it ("write report to file").
- Any change to `term:fontfamily` or `term:theme`, the other two settings
  the original spec cited as sharing this same construction-time-only
  limitation. Worth a follow-up audit, but not asked for here and not
  confirmed to bother users the way scroll sensitivity apparently does
  (font family/theme changes are rarer, and arguably more tolerable to
  require a reopen for, than a per-scroll-gesture feel setting).
## 8. What shipped

Implemented as §6 proposed, with one deliberate deviation and one addition:

- **Deviation:** instead of duplicating the `typeof … === "number" && … > 0
  ? … : 1` clamp inline in the new atom (as §6 literally suggested),
  extracted it into a new shared module,
  `frontend/app/view/term/termscrollsensitivity.ts`
  (`resolveTermScrollSensitivity`), mirroring the existing
  `termscrollback.ts` — the sibling module this codebase already uses for
  exactly this "every xterm surface must resolve a setting the same way"
  problem (its own header comment states the rationale, almost word for
  word applicable here). `termwrap.ts`'s constructor was refactored to call
  it too, rather than keeping its inline copy — three independent copies
  of one clamp is the precise drift risk `termscrollback.ts` exists to
  prevent.
- **Addition, as a side effect of the above:** the shared resolver also
  clamps to the schema's documented `0.1`–`10` range
  (`schema/settings.json`), which `termwrap.ts`'s original inline logic
  did not — it only checked `> 0` with no upper bound. A value above `10`
  was previously only reachable by hand-editing `settings.json` past what
  the UI's own number input accepts; it now clamps to `10` instead of
  passing through uncapped. This tightens behavior to match the already-
  published contract rather than loosening or changing it for any value
  reachable through the settings UI.
- `AgentShellSubblock.tsx`'s effect reads the resolver off the settings
  bag it already subscribes to (`getSettingsPrefixAtom("term")`, already
  in use there for `term:scrollback`) rather than adding a new
  `getSettingsKeyAtom` import — one fewer subscription, same reactivity.
- 6 unit tests for the resolver (`termscrollsensitivity.test.ts`), full
  frontend suite (1980 tests across `frontend/app/view/term` +
  `frontend/app/view/agent`) and `npx tsc --noEmit -p
  tsconfig.citypecheck.json` (the exact CI config) both clean.

**Not live-verified against a running instance this pass** — no dev CEF
host with hot-reload was available at implementation time (only a stale
packaged build from before this change, and an orphaned leftover `vite`
process of uncertain origin that seemed unsafe to interfere with
unprompted). The two new effects are structurally identical to the
already-proven-live `fontSize` effects immediately beside them in both
files (`term.tsx:247-254`, `AgentShellSubblock.tsx:318-333`), which is
why this was judged an acceptable pattern-match rather than blocking on a
fresh dev build — but it is a real gap relative to how the sibling
pane-color work in this repo's recent history was verified, and is worth
closing with a `task dev` session before merge if one becomes available.
