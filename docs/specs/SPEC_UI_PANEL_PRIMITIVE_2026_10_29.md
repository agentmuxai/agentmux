# SPEC: one `Panel` primitive (with Popover and Prompt) for every inline panel, prompt card and form

(Part 2, added 2026-10-29 at the operator's word "those tags also need to be part of the greater framework": **§9, a `Tag` primitive (chips, ribbons, counts, filter chips) built on `Pill`**. Korp's #10964 then only chooses which tags a provider card gets and their wording.)

**Status:** active. Approved by the operator 2026-10-29 ("recommendations look good, lets build it", relayed by the platform lead); reviewed by Agent1 (UI kit seat). Task #10927; the build is #10928. Build order: Tag T1 (§9) first, then the Panel primitive, then the migrations (§4). T1 is built in this PR; the rest is not.
Owner: Camper-2. Written 2026-10-29 against origin/main `20bb25c0d` plus #5839.
Builds on `SPEC_UI_LINE_STYLE_COMPONENT_SYSTEM_2026_10_05.md` (§5.7 patterns, §7 enforcement) and the motion primitives (#5781).

## 1. The request, and the answer to the operator's questions

The operator, 2026-10-29 (quoted): "build all the panels off the same template, so they all look alike, make them DRY, currently they appear ad-hoc. We also have what appears to be an entirely different panel style on new agent .. I thought by now we had consistent workings in this area, how conformed is this to the project styles system? and how good are the styles in place now?"

**How conformed, how good (measured on main, not guessed).**

- **Tokens, colour, type, motion, z-index: good and enforced.** `check-ui-primitives.mjs` holds 0 solid fills, 0 off-token radii, 0 undefined variables, 0 colour literals in styles, and ratchets raw durations, font sizes, z-indexes and hand-written focus rings downwards. Controls (`Button`, `TextInput`, `Field`, `SegmentedControl`, `Tabs`, `Card`, `Pill`) are one family.
- **The box a control sits in: not a system at all.** There is no `Panel`, `Popover` or `Prompt` in `element/ui/`. `AnchoredPopover` (portal, position, outside-click and Esc, pane-overlay cut) is the one shared piece, and it deliberately draws nothing: "the body carries every visual style". So each surface draws its own box. Counted: **30 drop-shadow declarations in 27 SCSS files outside the kit; 24 of them hand-written, 20 distinct values** (`0 4px 12px rgba(0,0,0,.25)` three times, `.4` twice, `0 4px 10px color-mix(...)` twice, `0 8px 24px`, `0 8px 28px`, `0 8px 32px`, `0 2px 6px` ...) against 6 that use a shadow token. Surface colour is three different tokens for the same idea (`--modal-bg-color` in 18 rules, `--main-bg-color` for `.modal-panel` and `.instance-panel`, `--panel-bg-color` for the card detail) plus accent-tinted `color-mix` variants. Borders are `--border-color`, `--accent-color`, accent-50%-mixed, `--attention-color` with a 3 px left edge, `0.5px solid var(--tint-4)`, `--modal-border-color`. Padding is `var(--space-2) 10px`, `var(--space-3)`, `var(--space-2) var(--space-3)`, `var(--space-3) var(--space-4)`, `16px`, `4px 0`. Widths are 200, 300, 320, 360, 380, 420, 440, 460 px, written as literals. Three mixins draw a frame (`popover-frame`, `menu-frame`, `approval-window`) with 1, 3 and 3 users.
- **Why New agent looks "entirely different".** It is not a popover: it is the P1 "details below the card" pattern. `.ui-card-detail` (card.scss) is an inline, full-width box with an **accent border, `--panel-bg-color`, a caret, a CSS reveal and no shadow**, and `.agent-create-panel` inside it has no box of its own. Every prompt next to it (`.agent-fork-prompt`, `.agent-question-panel`, `.agent-decision-panel`) is a different recipe. They are all "a titled box with a body and an actions row"; they differ only by accident.

**The fix is one template with a small number of declared variants**, plus a ratchet so a new one cannot be hand-rolled.

## 2. Inventory: what becomes `Panel`, what stays

Read from the SCSS (file:line is the class's rule). "Fate" is the recommendation.

| # | Family | Where | Kind today | Fate |
|---|---|---|---|---|
| 1 | `.agent-fork-prompt` (already-open, no-pane, fork/duplicate naming, rename) | `_recent-sessions.scss:586` | floating, anchored | **Popover + Prompt**, tone accent |
| 2 | `.agent-row-menu` | `_recent-sessions.scss:513` | floating menu | **Menu** (§6): surface tokens only |
| 3 | `.agent-create-panel` in `.ui-card-detail` (New agent form) | `AgentCreatePanel.scss:88`, `card.scss:37` | inline, docked under a card | **Panel docked** (caret) |
| 4 | `.identity-account-panel` | `identity/styles/_detail.scss:123` | inline, docked (accent border, no top) | **Panel docked** |
| 5 | `.memory-adoption-panel`, `.memory-claims-panel` | `native-memory-manager.scss:577,659` | inline boxes | **Panel inline** |
| 6 | `.agent-question-panel` | `AgentQuestionPanel.scss:9` | inline, attention edge, `--shadow-lg` | **Prompt** inline, tone attention |
| 7 | `.agent-decision-panel` | `_decision-panel.scss:9` | inline, attention tint + edge, own button weights | **Prompt** inline, tone attention |
| 8 | `.agent-auth-panel-card` | `_activity-log.scss:15` | inline, accent border, `--shadow-lg` | **Prompt** inline, tone accent |
| 9 | `.agent-seat-panel`, `.agent-env-panel` (bodies of the Seat and Environment modals) | `AgentSeatPanel.scss`, `AgentEnvPanel.scss` | modal body, own notices | stay inside `Modal`; adopt the form fields and `line.notice` already used; no box of their own |
| 10 | `.in-app-login-*` | `InAppLoginPanel.scss` | inline form, own URL input | **Panel inline** + `TextInput` |
| 11 | `.files-conflict` | `files.scss:567` | inline dialog in a pane (`--modal-border-color`) | **Prompt** inline |
| 12 | `.accounts-chooser` | `accounts-gallery.scss:54` | floating, `0 8px 28px` literal | **Popover** |
| 13 | `.status-bar-popover` + `.cpu-cores-popover`, `.disk-volumes-popover`, `.history-usage-popover` | `StatusBar.scss:269`, `_cpu-cores-popover.scss`, `_disk-volumes-popover.scss`, `_history-usage.scss` | floating readouts (`popover-frame` mixin) | **Popover**, density compact; the readout row/bar mixins stay as body content |
| 14 | `.instance-panel`, `.status-bar-qr-panel` | `_instance-panel.scss:9`, `StatusBar.scss:364` | floating, `--main-bg-color` | **Popover** |
| 15 | `.tab-context-panel` | `tab.scss:270` | floating (`menu-frame` mixin) | **Popover** (it is a form, not a menu) |
| 16 | `.search-container` | `element/search.scss:5` | floating search box, accent border | **Panel floating**, compact |
| 17 | `.notification-bubble` | `notificationitem.scss:139` | toast, `0 8px 32px` literal, close + pin | **Panel floating** (toast variant, later PR) |
| 18 | `.approval-window` mixin (credential, SSH, memory-adoption approval) | `element/_approval-window.scss` | whole top-level window chrome | stays (a window, not a panel); takes the surface and padding tokens |
| 19 | `.modal-panel`, `.confirm-dialog` | `modal.scss:87`, `confirm-dialog.scss:15` | modal; one uses `--main-bg-color`, the other `--modal-bg-color` | **stay `Modal`/`ConfirmModal`**; they share the surface and shadow tokens so the two agree |
| 20 | `.ctx-menu`, `.menu`, `.status-bar-tip-balloon` (tooltip) | `context-menu.scss:12`, `StatusBarTip.scss:15` | menu / tooltip | **stay** menu and tooltip; surface and shadow tokens only (their own spec: `SPEC_UNIFIED_MENU_SYSTEM`) |
| 21 | `.agent-card` | `_picker.scss:272` | a tile | **`ui/Card`**, not Panel |
| 22 | typeahead modal, `.agent-tool-panel`, `.swarm-section-panel`, `.editor-error-panel`, `.ui-tabbed-pane-panel` | various | modal / transcript block / layout section / empty state / tab body | **stay** (not a floating or prompt box) |

**Counts.** Becomes Panel, Popover or Prompt: families 1, 3 to 8, 10 to 17 (16 families, 22 class names). Takes only the shared tokens: 2, 9, 18 to 20. Moves to another primitive: 21. Stays: 22. (The task text says "about 23"; this is the same set, counted by what each one *is*.)

## 3. The primitive

One file family in `frontend/app/element/ui/`: `Panel.tsx`, `_panel.scss` (a mixin, like `line.notice`, so SCSS-only surfaces can adopt it before they are rewritten), exports in `index.ts`.

```tsx
<Panel
  kind="floating" | "inline" | "docked"   // see below
  tone="neutral" | "accent" | "attention" | "error"
  density="comfortable" | "compact"
  width="auto" | "sm" | "md" | "lg"        // tokens, never px at a call site
  title?: JSX.Element                       // header row, left
  onClose?: () => void                      // Panel (inline/docked): renders the top-right close button (IconButton, xmark, aria-label Close)
  actions?: JSX.Element                     // the actions row, right-aligned, [secondary] [primary]
  caret?: boolean                           // docked only: a caret points at the card
  caretLeft?: string                        // docked only: where it points (the --ui-card-caret-left value)
>
  body
</Panel>
```

- **`floating`**: lifted off the page (shadow, `--panel-bg-floating`). Placement, `z-index`, portal and dismissal belong to the host, `Popover`.
- **`inline`**: in the flow, no shadow, the same border and padding. For prompts in a stream (question, decision, login).
- **`docked`**: inline, attached to a card or row (accent border, a caret, the reveal). This is today's `.ui-card-detail`, so the card grid renders its detail through `Panel kind="docked"` and `.ui-card-detail` goes away.
- **`Popover`** = `AnchoredPopover` + `Panel kind="floating"`: takes the anchor and placement, returns focus to the opener on close, focuses the first control on open (`focusOnOpen`, already in `util/focusutil`), and keeps the pane-overlay cut. Callers stop passing `class` to style the box. **One dismissal handler:** `onDismiss` drives outside mousedown, Esc **and** the close button (a `Popover` has no `onClose`); `dismissible={!busy}` refuses all three while a primary action is committing (a prop, not a convention). **`escScope`**: a `Popover` defaults to pane-scoped Esc (`eventBelongsToPaneOf`), because `AnchoredPopover`'s window-wide Esc with `stopPropagation` eats an Escape meant for another pane; `AnchoredPopover` itself gains the `escScope` option with its default unchanged, so `MyAgentsList` can drop its own workaround later.
- **`Prompt`** = a `Panel` whose body is a message and whose `actions` are the answer buttons: `<Prompt kind=... message=... actions=...>`. It takes `kind` and passes it through; it never infers floating-or-inline from where it is rendered. It exists so "one sentence and two buttons" is one component, with the close button in the corner as in #5839.
- **Tone** reuses the line kit's custom properties (`--ui-tone`, `--ui-line`, `--ui-bg` from `line.tone`): the line colour and a faint tint, nothing else. `attention` keeps its 3 px left edge, since the question and decision panels use it to say "an agent is waiting"; no other tone has an edge.
- **Density** only changes padding and gaps (`--space-3` comfortable, `--space-2` compact), as `Button`'s density does.
- **A11y.** `role="group"` with `aria-labelledby` the title when there is one; `role="dialog"` only for a `Popover` that traps focus (none in v1: they return focus, they do not trap); the close button has an accessible name; Esc closes a floating panel, and is refused while a primary action is busy (the P1 rule).

### 3.1 Tokens

Add to `theme.scss` (the only place values live), all derived from existing tokens so every theme follows:

```
--panel-bg-floating: var(--modal-bg-color);
--panel-bg-inline:   var(--panel-bg-color);
--panel-line:        var(--border-color);
--panel-shadow:      var(--shadow-md);          // floating only; modal keeps --shadow-modal
--panel-w-sm: 240px;  --panel-w-md: 360px;  --panel-w-lg: 480px;
--panel-pad:         var(--space-3);  --panel-pad-compact: var(--space-2);
```

The 20 distinct literal shadows collapse to `--panel-shadow` (menus and tooltips use `--shadow-sm`/`--shadow-md`, modals `--shadow-modal`). `--main-bg-color` for a floating box goes away; `.modal-panel` and `.confirm-dialog` agree on one surface. Widths 200 to 460 map to sm/md/lg (the status-bar readouts are `sm`, the fork prompt and chooser `md`, the toast and the files-conflict `md`/`lg`).

### 3.2 Inputs and focus

- The `ui-input` ring and border treatment stays inside `element/ui/`, so the ratchet's rule 9 does not count it.
- Fold `.agent-fork-input`, `.in-app-login-url-input`/`-text` and the decision panel's feedback input into `TextInput` (`ui-input`, already density-aware and `Field`-aware), and delete the three bespoke rule sets. Today they show focus three ways: `.agent-fork-input` changes the border and fill, the login URL input draws a 1 px inset outline, and `ui-input` takes an accent border plus a 1 px accent ring (`ui.scss:311`, `:focus`).
- **Two focus treatments, each used by one kind of thing, and none inside a panel's own SCSS.** A text field is `ui-input`'s accent border and ring; every other focusable thing (buttons, cards, the close button) is `@include mixins.focus-ring`. A panel never draws a ring; the ratchet's rule 9 already counts hand-written ones (4 today).

### 3.3 Motion

Through the motion policy, never `prefers-reduced-motion` directly (`mixins.when-motion-reduced(...)` in SCSS, `motionAllowed` and the primitives in TS). A `floating` or `docked` panel plays `enter(el, ...)` (Agent1's primitives, category `transition`) when it mounts, through `useMotion()` from `app/motion/solid.ts` so the enter is cancelled on unmount. That replaces `ui-card-detail-reveal` (a CSS keyframe with its own reduced-motion branch) and any per-surface fade.

- **Docked reveal without a visible change.** Today's reveal is opacity plus `translateY(-4px)` (from above); `enter`'s `slideY` starts +8 px (from below). Agent1 adds an `offset` option to `enter` (a small PR of theirs, before the docked step; PR 1 does not need it). Open: ask Agent1 whether they want it before PR 1.
- **Housekeeping when `ui-card-detail-reveal` goes:** run `node scripts/check-motion-gates.mjs --update` in the same PR (its keyframes entry in `scripts/check-motion-gates.baseline` must drop, or the check fails). Panel SCSS adds **no `@keyframes` and no literal milliseconds** (the motion guard).
- **Exit: v1 has enter only**, exit after the migration, with Agent1. Then: `exit` leaves the node at its end state (fill forwards) until the caller removes it, so Agent1 makes `exit` itself set `pointer-events: none` and `inert`; `Popover` gets a `present` prop and an `exiting` local state and stays mounted until `exit(...).finished` (`AnchoredPopover` is documented "mount only while open").
- The close button, the attention edge and the tint never animate.

## 4. Migration order

Each step is one PR with `task dev` before/after screenshots of every surface it touches (the operator's ask). Narko cannot build `task dev` until #10731 gives it a fast profile; until then the screenshots are taken by an agent on charlie or Area54 (Opaz or Posa), or owed in the PR.

1. **Primitive**: `Panel`, `Popover`, `Prompt`, `_panel.scss`, tokens, tests (every kind, tone, density; close button; Esc and outside click; focus return; reduced motion), and the ratchet rule (§5) at its baseline. No consumers. No visible change.
2. **Prompts, first (the operator's complaint)**: the fork/already-open/no-pane/rename prompts and the naming step (family 1, building on #5839), `.agent-fork-input` into `TextInput`; the row menu takes the surface tokens.
3. **New agent form and its siblings** (lands alone; a screenshot for each `CardGrid` consumer: Connectors, MCP, Remotes, Toolchain, Accounts; grep first for users of `.ui-card-detail` outside `CardGrid` and keep it as a deprecated alias for one PR if any exist): `.ui-card-detail` becomes `Panel kind="docked"` (Connectors, MCP, Remotes, Toolchain and Accounts all render through `CardGrid`, so their detail panels change look too: say so in the PR), `.agent-create-panel`, `.identity-account-panel`, the memory panels (families 3 to 5). This is the "entirely different panel style" fixed.
4. **Agent attention prompts**: question, decision, auth card, files-conflict (families 6 to 8, 11), with the decision panel's own button weights replaced by `Button` tones.
5. **Floating readouts**: status bar popovers, instance panel, QR panel, accounts chooser, tab context panel (families 12 to 15); the `popover-frame` and `menu-frame` mixins are deleted when their last user moves.
6. **Login form, search box, toast** (families 10, 16, 17), then the leftover tokens-only surfaces (menus, tooltip, modals) take `--panel-shadow` and the one surface token.

Order within a PR: the primitive's visual change lands with the surface it is for, not ahead of it. Each PR lowers the ratchet's baseline.

## 5. The ratchet (stops new hand-rolled panels)

Extend `scripts/check-ui-primitives.mjs` with a rule **12. panels**, per file, SCSS outside `element/ui/`, compared with the base branch like the other eleven (no checked-in baseline). A count that goes up fails; one that goes down is reported. Three counts, each measurable today:

- **(a) hand-written drop shadows**: a `box-shadow` that is not `inset`, a ring or glow (`0 0 0 Npx`, `0 0 Npx`) and does not read a `--shadow-*`, `--panel-shadow` or `--modal-box-shadow` token. **24 today** in 27 files' worth of 30 declarations.
- **(b) hand-rolled surfaces**: a rule that sets `background: var(--modal-bg-color | --panel-bg-color | --main-bg-color)` together with a `border` and a `padding`, outside `element/ui/` and the theme files. Today's `--modal-bg-color` backgrounds alone are 18 rules; this is the fuzzy one, so it counts a rule only when all three (surface, border, padding) are present, and the PR that adds the check states the exact number.
- **(c) legacy frame mixins**: `@include` of `popover-frame`, `menu-frame` or `approval-window`'s box (1, 3 and 3 today).

The message names the primitive to use. The same check already walks SCSS with nesting resolved (`stripComments`, the rule walker used for rules 2 and 3), so (a) and (b) are small additions. A `// panel: fixed` marker (the escape the colour rule uses) covers a real exception, such as a canvas overlay.

## 6. Decisions

Recommendation first; each can change after Agent1's review.

1. **Where it lives.** `element/ui/` with the rest of the kit, `Panel.tsx` and `_panel.scss`. Recommended. (Alternative: `element/`, beside `anchored-popover`; rejected, the kit is where `Card`/`Button` are and where the ratchet looks.)
2. **Three kinds, not one.** `floating`, `inline`, `docked`. Recommended: the box is the same, the three differ only in shadow, border edge and caret, and a prompt that moves from a popover to an inline card should not change look.
3. **Menus are not Panels.** Recommended. Menus stay the `.menu` family with their own spec; they take the surface and shadow tokens so they look alike, and `Panel` is not asked to do roving focus.
4. **Modals stay `Modal`/`ConfirmModal`.** Recommended. They share the surface (`--modal-bg-color`, not `--main-bg-color`) and `--shadow-modal` tokens; a modal is a dialog with a backdrop, not a panel.
5. **One close affordance**: a corner close button on every dismissible panel, as #5839 does for the fork prompt, plus Esc and outside click for floating ones. Recommended.
6. **Tone by line, not fill**, as the kit already says: accent and error are a 1 px line and a faint tint; only `attention` has an edge. Recommended.
7. **Exit animations after the migration**, not in PR 1 (§3.3). Recommended; asks Agent1.
8. **Toast as a later variant.** Recommended: pin and close need their own behaviour.
9. **Who builds PR 1.** Recommended: Camper-2, with Agent1 reviewing and landing the `enter` `offset` option; Agent1 may take PR 1 instead (settle with them).
10. **Ratchet counts (a) to (c)**, not a single heuristic for "looks like a panel". Recommended: each is precise and cannot be argued with.

## 7. Risks

- The `docked` change restyles five unrelated screens at once (everything on `CardGrid`). The PR for step 3 must carry screenshots of each, and lands separately from step 2.
- `Popover` changes the focus behaviour of floating panels that today do not return focus; test with the keyboard, not only the mouse.
- `AnchoredPopover`'s Esc handler is window-wide and calls `stopPropagation`; the My Agents list avoids it on purpose for its menu (pane-scoped Esc). `Popover` should take an `escScope` or reuse `eventBelongsToPaneOf` so an Escape meant for another pane is not eaten. Raise with Agent1.
- Nothing here was run in a live window; sizes and the visual outcome are read from the SCSS.

## 8. Tests

`ui.test.tsx`-style unit tests per variant; a render test per migrated surface that its class names and test ids survive; the ratchet's own test file (`check-ui-primitives.test.mjs`) gains cases for rule 12 (a literal shadow counted, a token shadow not, a `// panel: fixed` line not, a mixin include counted).

## 9. Part 2: a `Tag` primitive (chips, ribbons, counts, filter chips)

The operator, 2026-10-29 (via AgentY): "i think those tags also need to be part of the greater framework". "Those tags" are the provider-card tags Korp designs in #10964: Free (blue), Local, Click to install, upgrade, container/native, and so on. Same reasoning as the panels: they will be hand-rolled again unless the kit has one.

### 9.1 What exists

`Pill` (`element/ui/Pill.tsx`, `pill.scss`): one component, five tones (`neutral`, `ok`, `warn`, `error`, `accent`), an optional icon, a native `title` tooltip, 36 users. Its wash and line come from theme tokens (`--success-bg`, `--success-line` ...), its text stays `--main-text-color`, and `pill-colors.test.ts` measures contrast in every theme. That is the right base: **a new tag tone or kind must pass the same test**. It has no sizes, no corner placement, no ordering, no overflow, no toggle.

### 9.2 What is hand-rolled today (migration targets)

About 30 badge, chip, tag and ribbon rules outside the kit, read from the SCSS:

| Kind | Rules (file:line) | Fate |
|---|---|---|
| **Chip** (a property: a status, a scope, a name) | `.instance-panel-window-badge` (`_instance-panel.scss:135`), `.maintenance-badge` (`_maintenance-section.scss:11`), `.agent-primitive-modal-list-item-badge` (`AgentPrimitiveModal.scss:44`), `.agent-question-panel-q-chip` (`AgentQuestionPanel.scss:141`), `.agent-ambient-narration-tag` (`AmbientNarrationBlock.scss:17`), `.runtime-badge` (`RuntimeBadge.scss:4`), `.agent-shell-agentlock-badge` (`_composer-strip.scss:699`), `.agent-tool-result-pill` (`_document-nodes.scss:780`, `_responsive.scss:35`), `.agent-jekt-tier-badge` and `-delivery-badge` (`_document-nodes.scss:1075`), `.agent-context-delivery-item-chip` (`:1197`), `.agent-card-upgrade-badge` (`_picker.scss:350`), `.agent-seat-badge` (`_recent-sessions.scss:346`), `.agent-no-pane-tag` (`:439`), `.bundle-view-global-badge` (`bundle-view.scss:76`), `.bundle-primitive-section-item-badge` (`BundlePrimitiveSection.scss:65`), `.bundle-import-collision-badge` (`BundleImportModal.scss:120`), `.identity-agent-chip` (`_detail.scss:96`), `.identity-provider-badge` (`_provider-badge.scss:10`), `.kanban-held-chip` (`kanban-held.scss:6`), `.kanban-agent-chip` (`kanban.scss:492`), `.memory-file-card-badge` (`native-memory-manager.scss:231`), `.native-memory-history-item-badge` (`memory-history.scss:101`) | **`Tag kind="chip"`** (22 rules) |
| **Ribbon** (what clicking the card will do, on its corner) | `.agent-card-install-ribbon` (`_picker.scss:321`) | **`Tag kind="ribbon"`** |
| **Count** (a number on a corner) | `.kanban-needs-badge` (`kanban-needs-badge.scss:5`) | **`Tag kind="count"`** |
| **Dot** | `.agent-active-badge` (`_recent-sessions.scss:413`) | `StatusDot` (already in the kit) |
| **Filter chips** (narrow a grid) | the filter toggles in `AgentPickerFilterBar`, the memory manager's agent filter (`native-memory-manager.scss` `.memory-agent-filter-toggle-*`), the Kanban filters | **`FilterChip`** |
| **Stay** (not tags) | `.pane-size-badge`, `.block-stats-badge`, `.image-viewer-zoom-badge` (readout HUDs over content), `.drone-chip` (a graph node), `.muxbus-login-chip` (a button), `.browser-profile-badge`, `.browser-loading-badge` (an avatar and a spinner), markdown `.am-badge` (agent-written rich output) | untouched |

**How far they differ.** Font sizes: `--text-3xs`, `--text-2xs`, `--text-xs`, `0.7em`, `0.75em`. Radii: 0, `--radius-sm`, `--radius-full`, `50%`. Paddings: `1px 5px`, `1px 6px`, `0 5px`, `0 4px`, `2px 8px`, `1px var(--space-1)`, `3px 10px`. **Four solid fills** (accent on the window badge and the global badge, warning on the needs counter, success on the install ribbon with a `#fff` literal and a stylelint disable), against the kit's rule that nothing fills solid. Tooltips are the browser's `title` or none.

### 9.3 The primitive

In `element/ui/`: `Tag.tsx` (`Tag`, `TagList`, `FilterChip`, `FilterChips`) and a `tag.scss` on the same base as `pill.scss` (`Pill` is kept for its 36 users and becomes `Tag kind="chip"` internally, so its API does not change).

```tsx
<Tag
  kind="chip" | "ribbon" | "count"
  tone="neutral" | "ok" | "warn" | "error" | "accent"
  size="sm" | "xs"                // --text-xs / --text-2xs; no other sizes
  icon?: string                   // as Pill
  group?: TagGroup                // from the typed const TAG_GROUPS (below), never a string a call site invents
  detail?: JSX.Element            // the tooltip slot
>label</Tag>

<TagList tags={...} max={3} />    // orders, truncates, renders "+2"
<FilterChips value=... onChange=...>   <FilterChip id count tone>label</FilterChip> ... </FilterChips>   // multi-select toggles
```

- **Kinds.** `chip`: a property, inline, round, quiet (today's Pill). `ribbon`: the label of the action a click on the card will do ("Click to install"): square, anchored to a card corner (`corner="bottom-right"`), not interactive itself (as today: `pointer-events: none`; the card is the button). `count`: a number at a corner, for "needs you" counters. A tag is never a button except `FilterChip`.
- **Tones from theme tokens, nothing else.** The five `Pill` tones. **"Free (blue)" is `accent`**: the default theme's `--accent-color` is `rgb(65,159,224)`, a blue, and it follows the theme everywhere else, so no new token is needed (the existing `--info-color` is purple and used by Kanban stages; do not reuse it for Free). Suggested mapping for Korp's set, for #10964 to confirm: Free `accent`, Local `neutral` with a `house` icon, Click to install `ok` ribbon with a `download` icon, Upgrade `warn` with `circle-up`, Container `neutral` with `box`, Native `neutral` with `desktop`. The wording and which providers get which is Korp's.
- **No solid tone fills, but opaque.** The kit rule (SPEC_UI_LINE_STYLE §9.3) applies: a ribbon is a tone line, a wash and a caps label, with hover raising the line like a button. One catch (Agent1): a ribbon or a count sits **on** the card's corner, over text or art, so a translucent wash would show what is under it. Their wash is therefore computed against the card's own surface, an opaque `color-mix(in srgb, var(--tone) N%, var(--card-bg))`, never against `transparent`. A chip in a row keeps `Pill`'s translucent wash. Today's install ribbon is a solid green with white text; the line version trades a little shout for consistency, and it works because the whole card is the call to action. **Decided (operator, 2026-10-29, via AgentY: "yes, do that"): the install ribbon goes line-style (outlined).** The contrast rule in §9.4 is the other half of that answer.
- **Fixed ordering.** `group` ranks tags in one constant order, never per call site: `action` (the ribbon, always its own corner), `attention` (errors and anything that blocks: tone `error`, and `warn` when it blocks), `plan` (Free), `where` (Local, Container, Native), `fact` (everything else, including a quiet note such as Upgrade). `TagList` sorts by group, then by the order given. A card therefore reads the same everywhere. The group is a typed const (`TAG_GROUPS.plan`) or comes from a named preset (`<FreeTag/>`, `<UpgradeTag/>`), so a call site cannot invent one; otherwise the order is only as consistent as its callers. Because `attention` ranks right after `action`, `+N` always cuts `fact` and `where` tags first and an error last, with a test on exactly that. **The ribbon is outside the count:** `max` applies to chips only (the ribbon has its own corner and is never folded into "+N"). So the worst provider card (Click to install ribbon, Upgrade, Free, Local) shows the ribbon plus all three chips at `max=3`, in the order Free, Local, Upgrade; on a narrow card (`max=2`) Upgrade is the one that collapses into "+1", because Upgrade is a quiet `fact` note on a card that works (as `_picker.scss` already says), not an `attention` tag. (Agreed with Korp.)
- **Tooltip slot.** `detail` renders in the shared `Tooltip` (the one `IconButton` uses). It is a JSX body, not one line: it wraps at a token width (`--panel-w-sm`, 240 px) and is meant for up to about four short lines of text with inline emphasis (Free's limits, catches and "verified 2026-10-29"). It is **not interactive**: no links or buttons, because a tooltip closes when the pointer or focus leaves the tag. A "Sign up" link belongs in the card (its docked panel, or an action on the card), not in the tag's tooltip; a tag is never a link or a button (only `FilterChip` is a toggle). a tag without `detail` is not focusable and keeps `Pill`'s native `title` when one is given (`Pill`'s `title` prop must keep working). A tag with `detail` takes `tabindex="0"`, and three things are checked, not assumed: the `Tooltip` opens on **focus** as well as hover; `aria-describedby` points at something that exists while the tooltip is closed (portaled tooltip content is not in the DOM until it opens), so the tag carries a visually hidden text node with that id; and a **tap** opens it on touch, which has no hover.
- **Overflow.** `TagList max` (default 3) shows the first N in order and a `+2` chip whose tooltip lists the rest. v1 is count-based; a width-aware version (a `ResizeObserver` fitting as many as fit) can follow without changing the API.
- **Filter chip.** `FilterChip` is a tag that is a toggle: a `<button aria-pressed>`, quiet at rest, `accent` line and wash when selected, an optional count. `count` is a number prop shown after the label ("Free 7"), updated live as the other filters change; an "Installed" chip is a `FilterChip` with no tone and no count. `FilterChips` groups them in a `role="group"`, **multi-select only**: one-of-N is `SegmentedControl`'s job and would need radio semantics, so there is no `single` mode in v1. The split stays crisp: exclusive and short means `SegmentedControl`, narrowing a grid means `FilterChip`.
- **Motion.** None of its own; a tag appearing in a list uses the list's `enter`. Hover and press are the kit's `control-transition` mixin.

### 9.4 Contrast: every tone, on every bundled theme, enforced by a test

The operator, 2026-10-29 (via AgentY): "as-is on a bunch of the dark themes (but not all) it is too hard to read". **The rule**, for every Tag tone (`neutral`, `ok`, `warn`, `error`, `accent`) on every bundled theme (the 12 in `frontend/app/themes/` plus the default):

1. **Text** on the tag's fill: WCAG AA, **4.5:1**. The fill is the opaque line-style fill of §9.3 (the tone at its wash percentage laid over the surface the tag sits on); the text is `--main-text-color` (`--secondary-text-color` for `neutral`), as `Pill` does today.
2. **Outline** against the surface the tag sits on: **3:1** (WCAG 1.4.11). The outline is the **full tone colour**, not the translucent `--*-line` wash.
3. The surfaces are the ones a tag really sits on: the theme's `--main-bg-color`, a card (`--tint-1` over it) and a popover/panel (`--modal-bg-color`).

**Measured on main today** (a throwaway probe over the theme files, resolving tokens the way `pill-colors.test.ts` does; `main-bg-color` as the surface):

- **The install ribbon's text fails on 11 of the 13** (white on the solid `--success-color`; bar 4.5): catppuccin 1.49, dracula 1.37, high-contrast 1.54, monokai 1.55, tokyo-night 1.83, nord 2.04, gruvbox 3.10, solarized-light 3.20, catppuccin-latte 3.34, default 3.53, midnight 3.53. Only `light` (5.08) and `gruvbox-light` (4.86) pass. That is the operator's "a bunch of the dark themes": white text on the light greens those themes use. The line-style ribbon removes the cause (the text becomes `--main-text-color` on a 14% wash).
- **The upgrade badge's text passes everywhere** (`--warning-text-color` on its wash: lowest 4.70, `light`).
- **Every `Pill` outline fails the 3:1 outline rule on every theme**: the translucent `--success-line`, `--warning-line`, `--error-line`, `--accent-line` come out at 1.2 to 2.2:1 against the surface. They are decorative on a pill (the words carry the status), but the rule asks for a real edge, and the full tone colour gives one.
- **The full tone colour as an outline fails 3:1 in 5 places today**: catppuccin-latte `success` 2.96 and `warning` 2.31, gruvbox `error` 2.69, solarized-light `success` 2.97 and `warning` 2.98 (nord `error` 3.05 and default `error` 4.12 pass). Everything else is at 3.3 or more.
- **Text on a 14% wash of the tone passes on all 13** (lowest 5.9, solarized-light `error`), so line-style fills are safe.

**Fix the token, not the component.** The five failing edges are fixed in their themes. Recolouring `--success-color` and friends would also change terminal and status colours elsewhere, so the kit gets four edge tokens in `theme.scss`, `--success-edge`, `--warning-edge`, `--error-edge`, `--accent-edge`, each defaulting to its tone colour, and the five failing themes override just their edge to the nearest value that reaches 3:1 (darker on the light themes). A component never overrides contrast itself; `Tag` reads only these tokens.

**Enforcement.** A new `tag-contrast.test.ts` in `element/ui/`, next to `pill-colors.test.ts` and built on the same resolver (moved into a shared test helper, `theme-resolve.ts`, so there is one): it lists the themes directory (as that test does, asserting at least 12) and for each theme x tone x surface checks rule 1 and rule 2 above, so a new theme, or a token change in an old one, cannot regress it. It also checks the hover state (the raised line) and that the ribbon and count fills are opaque. `pill-colors.test.ts` stays and is extended to cover ribbon and count (Agent1). Tag CSS may not contain a colour literal (`#fff` in today's ribbon is exactly the kind of line the colour rule and this test catch).

### 9.5 Ratchet rule 13: tags

In `check-ui-primitives.mjs`, per file, vs the base branch: **hand-rolled tags** = SCSS rules outside `element/ui/` whose class name ends in `badge`, `chip`, `tag`, `pill` or `ribbon` and that declare a `padding`, a `font-size` and a `border-radius` or `background` (a box of their own). 26 rules in 21 files today, the exact count when the check was written (T1 left them all counted: each `// tag: fixed` marker with its reason goes in with the migration PR that looks at that file). The message names `Tag`. Same escape as the colour rule.

### 9.6 Migration

Independent of the Panel steps except that both use the same tokens, so it can run in parallel:

- **T1: the primitive** (`Tag`, `TagList`, `FilterChip(s)`, tests: `tag-contrast.test.ts` (§9.4) and `pill-colors.test.ts` kept and extended to measure ribbon and count in every theme, the edge-token fixes (seven tokens in five themes, and the neutral outline at 80%, which the measurement showed 60% could not hold on the light themes), the `+N` cut order, the focus and tap tooltip, ratchet rule 13 at its baseline). No visible change; `Pill` users are unchanged. **Recommend this lands before Korp's #10964 build**, so that build uses it from the start.
- **T2: the provider cards**, in #10964's own build: install ribbon, upgrade badge, Free, Local, container/native on `agent-card`.
- **T3: status chips** in the agent transcript (jekt tier and delivery, tool result, context delivery, ambient narration, runtime, no-pane, seat).
- **T4: list chips** (bundle, identity, Kanban, memory, window badge, maintenance).
- **T5: filter chips** (picker filter bar, memory filters, Kanban) and the Kanban needs counter.

Each PR carries before/after screenshots and lowers rule 13's baseline.

### 9.7 Decisions (recommendation first)

1. **Extend `Pill` into `Tag`** rather than a second component: one base, `Pill` stays as an alias. Recommended.
2. **Ribbon in line style**, not a solid fill. **Decided by the operator** (2026-10-29); §9.4 adds the contrast rule.
3. **"Free" is `accent`** (blue in the default theme). Recommended; no new token.
4. **Fixed group order** in the kit, not per call site. Recommended.
5. **`detail` through the shared Tooltip**, focusable only when present. Recommended.
6. **Count-based overflow in v1; no `single` mode on `FilterChips`; group from a typed const or a preset.** Recommended (Agent1).
7. **Tag PR T1 before #10964's build**, and before or beside Panel PR 1 (they do not touch the same files). Recommended; Agent1 reviews both.
