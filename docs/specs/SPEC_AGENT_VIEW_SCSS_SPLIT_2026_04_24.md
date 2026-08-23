# Spec: agent-view.scss Decomposition

**Date:** 2026-04-24
**Status:** implemented — verified 2026-08-23: `agent-view.scss` is down to 387 lines (from the 4055 this spec was written against), almost entirely `@use` statements pulling in 28 files under `styles/` whose names match this spec's target list exactly (`_picker.scss`, `_card-info-popover.scss`, etc.). The remaining ~350 lines of actual rules are root-level pane-wrapper layout (`.agent-pane-stack`), not unmigrated component styles — consistent with this spec's own §4 caveat that some rules stay at the top level. No single "did the split" PR found; this looks like it happened incrementally as normal feature PRs extracted their own component's styles over time, guided by this spec's pattern rather than executed as one dedicated migration. (An earlier pass of the docs-cleanup audit that flagged this spec as stale mischaracterized it as "half-done" based on a shallower check — corrected here after actually reading the file's current content.)
**Owner:** AgentA
**Implements:** [SPEC_DESIGN_SYSTEM_2026_04_23](./SPEC_DESIGN_SYSTEM_2026_04_23.md) Phase 5
**Touches:** `frontend/app/view/agent/agent-view.scss` (4055 lines)

---

## 1. Why this is a separate spec

The design-system spec calls for splitting `agent-view.scss` into per-subcomponent files (`_picker.scss`, `_card.scss`, …). The original spec gave a target file list but no migration mechanics. After Phase 3 + 4 cleanup, the file is now token-clean but still 4055 lines — every edit has to scroll past unrelated rules, and `git blame` rewrites itself with each tweak. This spec specifies **how** to split safely without introducing visual regressions or breaking CSS specificity.

## 2. Current state

```
frontend/app/view/agent/agent-view.scss             4055 lines
├── lines    1–3578: nested inside .agent-view { ... }   3578 lines
│   ├── document view + node wrapper                     ~120
│   ├── activity log panel                               ~80
│   ├── status line                                      ~120
│   ├── auth URL panel + paste UI                       ~150
│   ├── retry bar                                        ~30
│   ├── empty state                                      ~50
│   ├── focused-overlay (definition rename / settings)   ~150
│   ├── picker + card + card-settings panel             ~600  ← biggest
│   ├── nodejs notice                                    ~40
│   ├── picker empty state                               ~60
│   ├── control bar                                      ~10
│   ├── composer region (footer)                         ~120
│   ├── spinner + process badge                          ~70
│   ├── slash picker / autocomplete / help              ~600
│   ├── tool overlay + content                          ~600
│   ├── conversation document nodes (markdown / code)   ~750
│   └── miscellany                                       ~100
└── lines 3580–4054: top-level (outside .agent-view)     475 lines
    ├── identity panel                                  ~165
    ├── launch modal body                                ~125
    ├── import preview modal body                       ~70
    ├── delete-error message                             ~10
    └── card info popover                                ~75
```

Top-level rules already live outside `.agent-view` because they're rendered through portals (modal-v2, Popover) — they wouldn't match if scoped under `.agent-view`. That set is already correctly factored.

The nested 3578 lines are the real target.

## 3. Goals

- **G1.** Reduce single-file size from 4055 → at most ~400 lines per file. Median target: ~250 lines.
- **G2.** **Zero CSS regression.** Visual output must be byte-identical (same rules, same order, same specificity, same media queries).
- **G3.** Each split file maps 1:1 with a TSX component or component cluster (e.g. `_picker.scss` ↔ `AgentPicker.tsx`).
- **G4.** Files co-locate next to consumers where reasonable (`frontend/app/view/agent/components/_card.scss` next to `AgentCard.tsx`), or live in a sibling `styles/` folder when shared by several components.
- **G5.** Build time and bundle size unchanged within ±2%. SCSS is compiled away; the runtime CSS is identical bytes.
- **G6.** Stylelint stays green throughout (no new ignored files).
- **G7.** `git blame` survives — use `git mv -k` patterns where possible, but for splits we accept the blame rewrite (filed under "necessary cost").

## 4. Non-goals

- Renaming any selector. Pure file reorganisation.
- Refactoring nesting depth. If the original was `.agent-view .agent-picker .agent-card .agent-card-icon`, the split file keeps the same nesting (just moves it).
- Re-tokenising any value. Phase 3 + 4 already did that.
- Splitting the **top-level** sections (already factored).
- Touching `.tsx` imports. Each TSX file imports `agent-view.scss` from a single point; that import stays.

---

## 5. Target file structure

```
frontend/app/view/agent/
├── agent-view.scss              ← becomes a 60-line aggregator
└── styles/                      ← NEW directory
    ├── _picker.scss             ← .agent-picker, .agent-picker-list, .agent-picker-empty (~70 lines)
    ├── _card.scss               ← .agent-card, .agent-card-* (~230 lines)
    ├── _card-settings.scss      ← .agent-card-settings-panel + tabs (~100 lines)
    ├── _document.scss           ← .agent-document, .hover-strip-host, .agent-document-node-wrapper (~120 lines)
    ├── _document-nodes.scss     ← markdown / code / list rendering (~750 lines)  ← the big one
    ├── _activity-log.scss       ← .agent-activity-log (~80 lines)
    ├── _status-line.scss        ← .agent-status-line + spinner-dot + process-badge (~190 lines)
    ├── _auth.scss               ← .agent-auth-url-* + .agent-auth-paste-* (~140 lines)
    ├── _retry-empty.scss        ← .agent-retry-bar + .agent-empty + .agent-connect-btn (~80 lines)
    ├── _focused-overlay.scss    ← .agent-focused-* (~110 lines)
    ├── _nodejs-notice.scss      ← .agent-nodejs-notice (~40 lines)
    ├── _composer.scss           ← .agent-composer-region + .agent-control-bar (~130 lines)
    ├── _slash.scss              ← .slash-picker / .slash-autocomplete / .slash-help (~600 lines)
    └── _tool-overlay.scss       ← tool block + portal content (~600 lines)
```

The new `agent-view.scss` becomes:

```scss
// Copyright …

@use "styles/picker";
@use "styles/card";
@use "styles/card-settings";
@use "styles/document";
@use "styles/document-nodes";
@use "styles/activity-log";
@use "styles/status-line";
@use "styles/auth";
@use "styles/retry-empty";
@use "styles/focused-overlay";
@use "styles/nodejs-notice";
@use "styles/composer";
@use "styles/slash";
@use "styles/tool-overlay";

// Top-level rules for portal-rendered content stay here for now —
// they're already factored from the .agent-view block below and
// don't fit under any "view-internal" partial.
@use "styles/identity-panel";       // ← extracted from current 3614–3779
@use "styles/launch-modal-body";    // ← 3781–3905
@use "styles/import-modal-body";    // ← 3907–3978
@use "styles/picker-delete-error";  // ← 3980–3987
@use "styles/card-info-popover";    // ← 3989–4054
```

Total expected file count after split: **20 partials + 1 aggregator** (vs 1 file today).

## 6. Migration mechanics

### 6.1 The `.agent-view` wrapper problem

Every selector in lines 1–3578 lives inside `.agent-view { ... }`. When I split `_card.scss`, do I:

- **Option A:** wrap each partial in its own `.agent-view { ... }` block?
  ```scss
  // _card.scss
  .agent-view {
      .agent-card { … }
      .agent-card-icon { … }
  }
  ```
  Sass will compile this. CSS output: each partial contributes its own `.agent-view .agent-card { ... }` rule — same selector specificity as the merged file. ✅ Cascade preserved.

- **Option B:** strip the wrapper, expect the consumer to wrap on import?
  ```scss
  // _card.scss
  .agent-card { … }
  // agent-view.scss
  .agent-view {
      @use "styles/card";
  }
  ```
  Sass `@use` is **not** nestable inside selectors. Won't compile. ❌

- **Option C:** use SCSS `@mixin` per partial, then `@include` from `agent-view.scss`?
  ```scss
  // _card.scss
  @mixin styles {
      .agent-card { … }
      .agent-card-icon { … }
  }
  // agent-view.scss
  .agent-view {
      @include picker.styles;
      @include card.styles;
  }
  ```
  Works, but ugly + every file needs a `@mixin styles` boilerplate. ❌

**Decision:** Option A — each partial wraps in its own `.agent-view { ... }` block. Sass deduplicates the selector chain in the compiled output (it doesn't actually emit multiple `.agent-view { ... }` parent declarations; it emits the descendant rules with the full chain). The browser sees identical CSS to today.

### 6.2 Atomic vs incremental split

- **Atomic:** one big PR moves all rules in one commit.
  - Pro: cascade is provably preserved (just `git mv`-style moves).
  - Pro: no transient state where some rules live in two places.
  - Con: huge PR — hard to review.
  - Con: any merge conflict during review forces a full re-split.

- **Incremental:** one PR per partial (~14 PRs total).
  - Pro: each PR is small + reviewable.
  - Pro: visual diff caught early per partial.
  - Con: cascade order between rules-in-old-file and rules-in-new-file becomes important. SCSS `@use` happens at the top of the aggregator; the rules from the partial appear in compiled CSS at the position of the `@use`. As long as the `@use` is placed where the original rules were, cascade order is preserved.
  - Con: ~14 round-trips through review.

**Decision:** **Atomic single PR.** Diff is a giant move, but git can do better than expected with `--find-copies-harder` / `-M -B` flags during review. The cascade-preservation argument is strong: every other strategy adds risk for marginal review benefit.

### 6.3 Cascade preservation strategy

The compiled CSS must have rules in the **same order** as today. Sass emits rules in source order, depth-first. To guarantee identical output:

1. The new `agent-view.scss` `@use` declarations are listed **in the order the corresponding sections appear in the current file**.
2. Each partial's contents are copied **verbatim** from the source file (same order within the partial).
3. We diff the compiled CSS before and after. Identical bytes = identical cascade. The Vite build produces a single CSS chunk; we compare the chunk's bytes.

Concrete validation:

```bash
# Pre-split
task build:frontend
sha256sum dist/assets/index-*.css > /tmp/before.sha

# After split
task build:frontend
sha256sum dist/assets/index-*.css > /tmp/after.sha

diff /tmp/before.sha /tmp/after.sha  # MUST be empty
```

If the bytes differ, we abort the merge until the diff is reconciled.

### 6.4 The `@use`-in-`.agent-view`-block problem (revisited)

Re-checking Option C — wrapping `@use` inside a selector. Sass actually supports `@include` (mixin call) inside selectors, but not `@use` (which is a top-level module-import statement). For our wrapper to work, every partial must already be wrapped in `.agent-view { … }` itself (Option A).

If a future refactor wants to drop the `.agent-view` wrapper entirely (e.g. when the design system promotes BEM-style flat selectors), each partial can simply unwrap independently. The split spec doesn't fight that future direction.

### 6.5 What about the top-level (lines 3580–4054) section?

Already top-level, no `.agent-view` wrapper needed. Each partial is a straight extraction:

```scss
// styles/_identity-panel.scss
.agent-identity-panel { … }
.agent-identity-provider-label { … }
…
```

`agent-view.scss` `@use`s them just like the nested ones. Sass will emit them at top level (no parent wrapper).

---

## 7. Risks & mitigations

| Risk | Mitigation |
|------|-----------|
| Compiled CSS bytes change → visual regression | Pre/post SHA-256 diff on the chunk file (§6.3). Abort if not identical. |
| Sass `@use` namespace collision (two partials defining a same-named mixin) | Audit current file: zero mixins. Risk doesn't exist. |
| Source-map regressions | Vite generates source maps from compiled CSS back to SCSS. Maps will point at the new partials, not `agent-view.scss`. That's correct behaviour, not a regression. |
| `git blame` rewrite | Accepted cost. Add a note in `.git-blame-ignore-revs` so contributors can use `git config blame.ignoreRevsFile .git-blame-ignore-revs` to see through the split commit. |
| Partial file accidentally not imported | Visual regression on first reload. Mitigated by the byte-diff check (any missing rule = different bytes). |
| Build time impact | Sass compiles all partials in parallel; Vite combines. Expected delta: ±50ms. Measured before/after. |
| Reviewer fatigue on a 4000-line PR | Use `--find-copies-harder -B 5%` in the PR's git diff URL and link to per-section sub-diffs in the PR body. |

---

## 8. Implementation steps

Single PR, executed as one commit.

1. **Pre-flight**
   - Verify lint clean: `npm run lint:scss`.
   - Capture baseline CSS hash:
     ```bash
     task build:frontend
     sha256sum dist/assets/index-*.css > /tmp/before.sha
     ```
   - Note: the dist file name has a content hash — record both filename and SHA so post-split comparison is unambiguous.

2. **Create `frontend/app/view/agent/styles/` directory.**

3. **Extract partials in section order** (top of file → bottom). For each partial:
   - Identify the line range in current `agent-view.scss`.
   - Copy lines verbatim into `styles/_<name>.scss`.
   - **For nested partials:** wrap the extracted content in `.agent-view { ... }` (one outer block per partial).
   - **For top-level partials:** no wrapper.

4. **Rewrite `agent-view.scss`** to a 30-line aggregator that `@use`s each partial in source order.

5. **Build + compare:**
   ```bash
   task build:frontend
   sha256sum dist/assets/index-*.css > /tmp/after.sha
   diff /tmp/before.sha /tmp/after.sha
   ```
   - Empty diff → safe to land.
   - Non-empty diff → bisect: identify which partial changed bytes; usually a missing rule, an extra `}`, or a wrap-block ordering bug. Fix and rebuild.

6. **Lint check:** `npm run lint:scss` must stay green.

7. **Type check:** `npx tsc --noEmit` must pass (no TS deps on SCSS file paths, but verify).

8. **Type check + lint pass → bump + open PR.**

9. **PR body** includes:
   - Pre/post SHA-256 of the compiled CSS chunk (proof of zero regression).
   - Per-partial line counts vs target.
   - The `.git-blame-ignore-revs` entry for the PR's commit hash (to be added in a follow-up after merge).

## 9. Open questions

1. **Should we use `@forward` instead of `@use`?** `@forward` re-exports a partial's namespace; `@use` imports it. For our case (partials are pure CSS, no mixins/functions), the choice is cosmetic. `@use` is the modern recommendation; sticking with it.
2. **Should the partials use `_name.scss` (Sass partial convention) or `name.scss`?** Convention says underscore prefix for files only ever consumed via `@use`. We follow convention.
3. **Should the new `styles/` folder live next to the components (`components/styles/`) or as a sibling?** Sibling is simpler — components/ stays focused on TSX. Decision: sibling.
4. **What about future component refactors?** If `AgentCard` later moves to its own component folder, the matching `_card.scss` should follow. The `styles/` directory is a stepping-stone, not a permanent fixture.
5. **Should we also move the `// === ToC` style comments** at the top of the file into per-partial `// purpose` headers? Yes — each partial's first 5 lines should be a docstring explaining what it styles.

## 10. Validation plan

- ✅ `npm run lint:scss` returns 0
- ✅ `npx tsc --noEmit` returns 0
- ✅ `task build:frontend` completes
- ✅ SHA-256 of compiled CSS chunk matches pre-split (the load-bearing check)
- ✅ Manual smoke: open the agent pane, verify cards render, click into a definition → modal works, message arrives → status line + activity log behave normally, toolblocks expand/collapse
- ✅ Hot-reload still works in `task dev` after edits to a partial
- Optional: take a screenshot of the agent pane before and after; visually `cmp` (would catch sub-pixel rendering differences if the byte-diff didn't already)

## 11. Cross-references

- [SPEC_DESIGN_SYSTEM_2026_04_23.md §5.8](./SPEC_DESIGN_SYSTEM_2026_04_23.md#58-mega-file-decomposition) — the parent spec entry this implements.
- `.git-blame-ignore-revs` — to be created post-merge with the PR's commit hash.
- The 8 design-system PRs (#526–#536) that brought `agent-view.scss` to its current token-clean state — the cleanup that made this split tractable.
