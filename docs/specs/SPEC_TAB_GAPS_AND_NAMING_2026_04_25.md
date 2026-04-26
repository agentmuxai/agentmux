# Spec: Constant Tab Gaps + Plain-Language Default Names

**Date:** 2026-04-25
**Status:** Draft, ready to implement
**Owner:** AgentA
**Touches:** `frontend/app/tab/tab.scss` (or wherever the
             tab-bar layout rules live),
             tab-creation code paths that assign default
             names — likely `frontend/app/store/global.ts`
             and any "new window" / "new tab" RPC code in
             `agentmux-srv/src/` and `agentmux-cef/src/`.

---

## 1. Problem

Two unrelated issues in the tab strip; bundled because they
ship together cheaply.

### 1.1 Tab gaps drift with text width and window resize

When a tab is renamed (or the window is resized), the gap
*between* tabs becomes inconsistent — one tab might have 4 px
to its right, the next 12 px. The strip looks irregular.

The root cause is almost always one of:

- `flex: 1` on each tab distributes remainder space as
  variable per-tab margin.
- `justify-content: space-between` / `space-around` —
  remainder space is computed per-cell.
- A `gap` rule that's right in principle but combined with a
  per-tab `margin-right` that survives the last tab.
- Transitions on `width` / `flex-basis` that make adjacent
  tabs animate at different rates while text changes.

The user's stated requirement: **the gap between any two
adjacent tabs must be exactly the same constant, always —
regardless of tab text width or pane width.**

### 1.2 Default tab names use technical / abbreviated forms

- A brand-new tab in this window: `Untitled1`, `Untitled2`, …
- A tab in a newly-opened window (via "Open another window"):
  `T1`, `T2`, …

Both feel like internal-implementation artefacts. `Untitled`
sounds like a Word-95 placeholder; `T1` feels like a
low-bandwidth abbreviation for "tab number one." The user
wants both unified to plain `tab1`, `tab2`, …

## 2. Goals

- **G1.** Adjacent tabs have a fixed, theme-token-driven gap
  that **never changes** with tab text width, total tab
  count, or window width. Eyeballable: the gap between tab 1
  and tab 2 must equal the gap between tab N-1 and tab N for
  any N, at any window width that fits the tabs.
- **G2.** New-tab default name is `tab1`, `tab2`, … no
  matter whether the tab was created in this window or a
  freshly-opened one.
- **G3.** Existing tabs the user has explicitly named keep
  their names (no migration that overwrites custom labels).
- **G4.** Stylelint stays green; all spacing tokens come from
  `theme.scss`'s `--space-*` family.

## 3. Non-goals

- **No change to tab dimensions.** The width / padding /
  font-size of each tab stays the same; only the *between-tab*
  spacing is normalised.
- **No change to how tabs scroll / overflow.** If too many
  tabs to fit, the existing horizontal-scroll or overflow
  behaviour is preserved.
- **No change to drag-reorder, close button, dirty-state
  indicator, etc.** Pure layout + label.
- **No tab-numbering renumber on close.** Closing tab 2 of
  three leaves the remaining tabs as `tab1` and `tab3` (just
  like before with `Untitled1`/`Untitled3`). New tabs get the
  next free integer that doesn't collide.

---

## 4. Design — Constant gaps (G1)

### 4.1 Pick one source of spacing

The bar is a single `display: flex; flex-direction: row;`
container. Spacing comes **only** from a flex `gap`:

```scss
.tab-bar {
    display: flex;
    flex-direction: row;
    align-items: stretch;
    gap: var(--space-1);   // single source of truth
    // …no margin / padding-right on individual .tab
    // children. No `space-between` / `space-around`.
}

.tab {
    flex: 0 0 auto;        // intrinsic content width, no flex grow/shrink
    // No margin-right. No margin-left. No flex: 1.
}
```

Rationale: `gap` on a flex container is computed as a fixed
distance between every adjacent pair, independent of the
total free space. `flex: 0 0 auto` ensures each tab takes
exactly its content width, so no per-tab remainder
distribution can drift.

### 4.2 Audit + remove conflicting rules

While migrating, grep `frontend/app/tab/` for any of:
- `flex: 1` on tab children
- `justify-content: space-between` / `space-around` on the
  bar
- per-tab `margin-right` / `margin-left`
- `width: auto` on tabs combined with `margin: auto`

Remove or replace each with the §4.1 pattern.

### 4.3 Width transitions

If tabs animate `width` on rename, keep the animation but
ensure it runs on the **tab itself** (not on neighbours via
flex remainder). The `gap` value should not be inside any
animated property.

### 4.4 Last-tab right-edge

`gap` does not add space after the last tab — that's correct
behaviour. If we currently rely on a trailing space (e.g.
to avoid the close-X bumping the right pane), use
`padding-right` on `.tab-bar`, not `margin-right` on the
last `.tab`.

---

## 5. Design — Default naming (G2)

### 5.1 Naming function

A single helper used by every tab-creation code path:

```ts
function defaultTabName(existing: string[]): string {
    // Find the lowest positive integer N not already used as
    // "tabN" in `existing`. Returns "tab1" / "tab2" / …
    const taken = new Set<number>();
    for (const n of existing) {
        const m = /^tab(\d+)$/.exec(n);
        if (m) taken.add(Number(m[1]));
    }
    let i = 1;
    while (taken.has(i)) i++;
    return `tab${i}`;
}
```

Stable, predictable, doesn't renumber on close.

### 5.2 Call sites

The exploration pass should report exact file:line. Likely
candidates to migrate:

| Today | Becomes |
|---|---|
| Tab created in current window: `"Untitled" + n` | `defaultTabName(currentTabNames)` |
| Tab created in new window: `"T" + n` | `defaultTabName(newWindowTabNames)` |
| Any spec / test fixture asserting `"Untitled1"` / `"T1"` | Update fixtures |

### 5.3 Backwards compatibility

- Existing tabs the user has named (anything other than the
  exact patterns `Untitled\d+` or `T\d+`) — never touched.
- Existing tabs still named `Untitled1` / `T1` from old
  sessions — *not* auto-renamed on app start. They remain
  what the user / app saved them as. The change applies only
  to tabs created from this build forward.
- Settings / config files: no migration. The tab names live
  in the per-window state, not config.

---

## 6. Implementation steps

1. **CSS pass.** Apply §4 to the tab-bar SCSS. Resolve
   token: pick a single `--space-*` value (probably
   `--space-1` or `--space-1-5`) for the gap and document
   the choice in the SCSS comment.
2. **Naming helper.** Add `defaultTabName()` (likely in
   `frontend/app/tab/utils.ts` or alongside the tab store).
3. **Call-site migration.** Replace each of the two patterns
   (`Untitled` + n, `T` + n) with the helper.
4. **Fixture / test updates.** Run `tsc --noEmit` + the test
   suite; fix any expected-string assertion that references
   `Untitled1` / `T1`.
5. **Visual smoke.**
   - Open AgentMux. Create 4 tabs. Measure gap with browser
     dev-tools — same px between every adjacent pair.
   - Rename tab 2 to a much longer label. Gap stays
     constant.
   - Resize the window from 600px → 1800px. Gap stays
     constant; tabs remain content-sized.
   - Close tab 2. Tabs 1 and 3 stay named `tab1` and `tab3`.
   - Open a 5th tab. It becomes `tab2` (lowest free).

## 7. Risks

| Risk | Mitigation |
|---|---|
| Removing a `flex: 1` breaks an existing layout dependency (e.g. tabs that *should* fill remaining width) | Verify by inspection — the design explicitly wants content-width tabs. If a separate "spacer" element existed, replace it with `margin-left: auto` on the right-hand sibling group, not `flex: 1` on tabs. |
| Some call site is in Rust (not just TS) — e.g. `agentmux-cef` creates a default tab on window open | Provide the same naming function on the Rust side, or rename via a frontend-side post-spawn hook so naming logic stays in one language. |
| `Untitled\d+` regex collision with user-named tabs that happen to match | Use exact-match `^Untitled\d+$` / `^T\d+$` only when *generating* the next name. Never *rename* existing tabs based on the regex. |
| Removing trailing right-padding leaves close-X bumping window edge | Spec calls out `padding-right` on `.tab-bar` rather than per-tab margin (§4.4). |

## 8. Validation

- ✅ `task build:frontend` succeeds
- ✅ `tsc --noEmit` clean
- ✅ `npm run lint:scss` green
- ✅ Manual smoke per §6.5 (4 checks)
- ✅ Open a second AgentMux window — its first tab is
  `tab1`, not `T1`.

## 9. Cross-references

- `frontend/app/tab/` — target dir
- `SPEC_DESIGN_SYSTEM_2026_04_23.md §5.2` — `--space-*`
  spacing scale (use a token, no raw px)
- `SPEC_LAUNCH_MODAL_PLAIN_LANGUAGE_2026_04_24.md` —
  precedent for replacing technical-feeling default labels
  ("Host" / "Container" → "On this computer" / "In a safe
  sandbox"). Same spirit applies to "Untitled" / "T".
