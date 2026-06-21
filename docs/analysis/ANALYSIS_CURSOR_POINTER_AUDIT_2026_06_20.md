# ANALYSIS: Cursor hand (`cursor: pointer`) — full codebase audit

**Date:** 2026-06-20  
**Status:** Analysis complete — implementation pending  
**Scope:** All `cursor: pointer` / Tailwind `cursor-pointer` occurrences in `frontend/`  
**Trigger:** Hand cursor appearing on window controls, pane title-bars, hamburger menu, tab bars,
status-bar items, floating-pane widgets, and menus — none of which are hyperlinks.

**Related:** `docs/analysis/ANALYSIS_CURSOR_STYLING_2026_06_15.md` (earlier analysis focused
specifically on scrollbars; the token layer and utility classes it proposed were shipped and
are now in production — this document supersedes it for the broader pointer-cursor policy.)

---

## 0. TL;DR

**Rule:** `cursor: pointer` (the link hand) must appear ONLY on elements that navigate to an
external URL or open a filesystem path outside the app. All buttons, menus, tabs, toggles,
modals, panels, and other interactive UI controls must use the arrow (`cursor: default`).

**Scale:** ~150 occurrences of `cursor: pointer` exist in the frontend. Approximately 2 are
correct; the rest (≥148) are incorrect by this rule.

**Root causes (3 lines control the majority):**
1. `app/element/button.scss:11` — `cursor: pointer` on every `<button>` element globally.
2. `app/theme.scss:308` — `--cursor-interactive: pointer`; components consume this token.
3. Tailwind `cursor-pointer` utility applied directly in TSX class strings on non-link elements.

**Minimum viable fix:** Change two values — `button.scss:11` and `theme.scss:308` — to
`cursor: default`. Then add one new rule: `a[href] { cursor: pointer; }`. This flip
automatically fixes every site that inherited from the button baseline or consumed the token,
with no per-file churn, and leaves the rare correct cases needing no change (they already
have `cursor: pointer` explicitly via other selectors).

---

## 1. The correct rule

| Cursor | Value | Correct on |
|---|---|---|
| Hand / link | `pointer` | `<a href="…">` to external URL; filesystem-path opener elements; terminal URL hover (xterm) |
| Arrow | `default` | Everything else — buttons, menus, tabs, toggles, modals, panels, status-bar items, window controls, hamburger, widgets, pane headers |

The rationale is the W3C / OS HCI convention: the pointer cursor communicates "this is a
hyperlink that navigates away." Using it on buttons teaches users the wrong affordance —
every click target is NOT a link.

---

## 2. Correct occurrences (keep `cursor: pointer`)

These are the only occurrences that satisfy the rule:

| File | Line | Element | Why correct |
|---|---|---|---|
| `app/view/term/xterm.css` | 134–136 | `.xterm-cursor-pointer` | Applied by xterm.js when the terminal detects a hovered URL in output; this IS a hyperlink. |

**Borderline (review before touching):**

| File | Line | Element | Note |
|---|---|---|---|
| `app/view/browser/browser-view.scss` | 39 | Browser pane nav button | This is a `<button>` for browser back/forward — should be `default`. However the embedded Chromium web content sets its own cursor independently; the rule on the host container is mostly irrelevant. Change to `default` for consistency. |
| `app/element/markdown.scss` | 244 | `.toc-item` | Table-of-contents anchor — in-page navigation. Not an external link; use `default`. |
| `app/view/agent/styles/_document-nodes.scss` | 1161 | `.clickable` modifier on tool result rows | If these rows open filesystem paths or external URLs, `pointer` is correct. If they expand/collapse in-app content, change to `default`. Verify at call site. |

---

## 3. Root causes

### 3.1 `app/element/button.scss:11` — global button baseline

```scss
// Current (incorrect):
button, [type="button"], [type="submit"] {
    cursor: pointer;   // ← propagates to ALL <button> in the app
    …
}
```

This single line is the cascade root. Every button — window controls, hamburger, tabs,
modals, status-bar items, menus — inherits the hand cursor from here. Removing or changing
this one declaration fixes the majority of incorrect sites automatically via cascade.

### 3.2 `app/theme.scss:308` — `--cursor-interactive` token

```scss
// Current (incorrect for the new rule):
--cursor-interactive: pointer;     // buttons, links, clickable rows
```

The June-15 analysis introduced this token and the utility class `.u-cursor-interactive`.
Components consume it via `cursor: var(--cursor-interactive)`. Since the token value is
`pointer`, all consumers show the hand. Changing this one value to `default` fixes all
token-consuming sites simultaneously.

### 3.3 Tailwind `cursor-pointer` in TSX files

Several TSX files pass `cursor-pointer` in Tailwind class strings on non-link elements:

- `app/element/emojibutton.tsx:35`
- `app/element/quicktips.tsx:263,276,289,302,315,345`
- `app/element/streamdown.tsx:222`
- `app/suggestion/suggestion.tsx:315`
- `app/view/launcher/launcher.tsx:236`
- `app/window/action-widgets.tsx:121`

These must be removed case-by-case (replacing with nothing, or explicitly `cursor-default`
where the Tailwind reset is needed).

---

## 4. Full classified inventory

### INCORRECT — should be `cursor: default` (148+ sites)

**Global / element layer**

| File | Lines | Element |
|---|---|---|
| `app/element/button.scss` | 11 | All `<button>` elements (cascade root) |
| `app/element/iconbutton.scss` | 3 | Icon buttons |
| `app/element/toggle.scss` | 25, 64 | Toggle controls |
| `app/element/flyoutmenu.scss` | 29 | Flyout menu items |
| `app/element/expandablemenu.scss` | 18 | Expandable menu items |
| `app/element/collapsiblemenu.scss` | 12 | Collapsible menu items |
| `app/element/popover-menu.scss` | 22 | Popover menu items |
| `app/element/modal.scss` | 180, 244 | Modal close/action buttons |
| `app/element/markdown.scss` | 244 | `.toc-item` (in-page TOC) |
| `app/element/emojipalette.scss` | 25 | Emoji picker cells |

**Window / chrome layer**

| File | Lines | Element |
|---|---|---|
| `app/window/hamburger-menu.scss` | 21 | Hamburger (≡) button |
| `app/window/action-widgets.scss` | 142 | Widget more-menu dropdown items |
| `app/window/action-widgets.tsx` | 121 | Widget slot button (Tailwind) |
| `app/tab/tab.scss` | 128, 285, 303 | Tab controls, close button, drag handle |
| `app/block/block.scss` | 158, 588 | Pane block controls |
| `app/block/titlebar.scss` | 45 | Pane title-bar button |

**Status bar**

| File | Lines | Element |
|---|---|---|
| `app/statusbar/StatusBar.scss` | 80, 231, 319, 330, 376 | Status-bar section items |
| `app/statusbar/_cpu-cores-popover.scss` | 17 | CPU cores popover item |
| `app/statusbar/_instance-panel.scss` | 56, 94, 148, 186 | Instance panel items |
| `app/statusbar/_token-usage.scss` | 18, 158 | Token usage items |

**App-level**

| File | Lines | Element |
|---|---|---|
| `app/app.scss` | 206 | Flash-error notification panel |
| `app/init/error-display.ts` | 318, 332 | Recovery button inline styles |
| `app/errors/ErrorBanner.scss` | 63, 108 | Error banner dismiss button |
| `app/components/confirm-dialog.scss` | 50 | Confirm dialog button |
| `app/notification/memory-pressure-banner.scss` | 39 | Banner CTA (uses `var(--cursor-interactive)`) |

**Modals**

| File | Lines | Element |
|---|---|---|
| `app/modals/command-palette.scss` | 87 | Command palette item |
| `app/modals/bundle-manager-modal.scss` | 46, 103 | Bundle manager items |
| `app/modals/toolchain-modal.scss` | 177, 196 | Toolchain modal items |
| `app/modals/typeaheadmodal.scss` | 81 | Typeahead modal item |

**Agent view**

| File | Lines | Element |
|---|---|---|
| `app/view/agent/styles/_action-bar.scss` | 29 | Action bar button |
| `app/view/agent/styles/_activity-log.scss` | 41, 240 | Activity log items / buttons |
| `app/view/agent/styles/_composer-strip.scss` | 25, 119, 151 | Composer strip buttons |
| `app/view/agent/styles/_connection-status.scss` | 45, 113, 154 | Connection status indicators |
| `app/view/agent/styles/_control-bar.scss` | 241, 299, 355 | Control bar buttons |
| `app/view/agent/styles/_decision-panel.scss` | 48, 193, 268 | Decision panel items |
| `app/view/agent/styles/_disconnected-banner.scss` | 54 | Reconnect button |
| `app/view/agent/styles/_document-nodes.scss` | 80, 141, 353, 388, 663, 735, 953, 1022, 1276, 1444 | Collapsible sections, tool expand buttons, subagent-link, show-more button |
| `app/view/agent/styles/_focused-overlay.scss` | 56, 101 | Overlay buttons |
| `app/view/agent/styles/_header-controls.scss` | 82, 147, 151, 170 | Agent pane header buttons |
| `app/view/agent/styles/_identity-panel.scss` | 106, 127 | Identity panel items |
| `app/view/agent/styles/_launch-modal-body.scss` | 98, 165, 267, 293, 320 | Launch modal items |
| `app/view/agent/styles/_pending-footer.scss` | 73 | Pending footer button |
| `app/view/agent/styles/_picker.scss` | 67, 125, 155, 249, 374, 444, 467 | Picker rows |
| `app/view/agent/styles/_recent-sessions.scss` | 84, 247 | Session list items |
| `app/view/agent/styles/_retry-empty.scss` | 24, 54 | Retry / new session buttons |
| `app/view/agent/styles/_search.scss` | 67 | Search result item |
| `app/view/agent/styles/_session-digest.scss` | 43 | Session digest row |
| `app/view/agent/styles/_setup-wizard.scss` | 260, 273, 335 | Setup wizard items |
| `app/view/agent/styles/_shell-node.scss` | 71, 143, 223, 294, 332 | Shell summary collapsibles and activity controls |
| `app/view/agent/styles/_slash.scss` | 41, 106, 175, 219 | Slash-command palette items |
| `app/view/agent/styles/_tool-overlay-portal.scss` | 87 | Tool overlay button |
| `app/view/agent/components/AgentIdentityModal.scss` | 32 | Modal button |
| `app/view/agent/components/AgentNativeMemoryModal.scss` | 101, 146, 257 | Memory modal items |
| `app/view/agent/components/AgentNewBundleModal.scss` | 106, 110 | Bundle modal items |
| `app/view/agent/components/AgentPrereqModal.scss` | 70 | Prereq modal item |
| `app/view/agent/components/AgentQuestionPanel.scss` | 85, 161, 196 | Question panel items |
| `app/view/agent/components/PaneRow.scss` | 29, 78 | Pane rows (uses `var(--cursor-interactive)`) |
| `app/view/agent/fork/ForkBar.scss` | 24 | Fork bar (uses `var(--cursor-interactive)`) |

**Other views**

| File | Lines | Element |
|---|---|---|
| `app/view/accounts/accounts-gallery.scss` | 31, 151, 179 | Account gallery items |
| `app/view/accounts/oauth-connect.scss` | 26 | OAuth connect button |
| `app/view/brain/global-brain.scss` | 175, 195, 225, 263 | Brain panel items |
| `app/view/browser/browser-view.scss` | 39 | Browser nav button (should be `default`) |
| `app/view/bundle-summary.scss` | 41 | Bundle summary item |
| `app/view/drone/drone-view.scss` | 98, 196, 265, 317 | Drone controls |
| `app/view/editor/editor-view.scss` | 107, 139, 281, 343, 531, 615, 662, 696 | Editor buttons and file-tree rows (in-app navigation) |
| `app/view/identity/identity-pane-view.scss` | 31, 49, 201, 224 | Identity pane items |
| `app/view/identity/styles/_accounts.scss` | 51 | Account item |
| `app/view/identity/styles/_detail.scss` | 95 | Detail item |
| `app/view/identity/styles/_empty-states.scss` | 28 | Empty-state CTA |
| `app/view/identity/styles/_form-overlay.scss` | 44 | Form overlay button |
| `app/view/identity/styles/_header.scss` | 36, 58 | Identity pane header buttons |
| `app/view/memory/memory-view.scss` | 32, 50, 190 | Memory view items |
| `app/view/subagent/subagent-view.scss` | 156, 231 | Subagent view items |
| `app/view/swarm/swarm-view.scss` | 84, 126, 211 | Swarm view items |
| `app/view/warden/warden.scss` | 172 | Warden view item |

**TSX files with Tailwind `cursor-pointer`**

| File | Lines | Element |
|---|---|---|
| `app/element/emojibutton.tsx` | 35 | Emoji button |
| `app/element/quicktips.tsx` | 263, 276, 289, 302, 315, 345 | Quick tips items |
| `app/element/streamdown.tsx` | 222 | Stream down button |
| `app/suggestion/suggestion.tsx` | 315 | Suggestion row |
| `app/view/launcher/launcher.tsx` | 236 | Launcher grid item |
| `app/window/action-widgets.tsx` | 121 | Widget slot button |

---

## 5. Recommended fix strategy

### Step 1 — Token flip (highest leverage, zero per-file churn)

Change `theme.scss:308`:
```diff
-    --cursor-interactive: pointer;
+    --cursor-interactive: default;
```

This fixes every site consuming `cursor: var(--cursor-interactive)` automatically:
`PaneRow.scss:29,78`, `ForkBar.scss:24`, `notification/memory-pressure-banner.scss:39`,
and any future consumer of the token.

### Step 2 — Button baseline (second-highest leverage)

Change `button.scss:11`:
```diff
-    cursor: pointer;
+    cursor: default;
```

This fixes the entire cascade — every `<button>` in the app reverts to the arrow
without touching any call site.

### Step 3 — Add the single correct global rule

In `app.scss` (or a new `_links.scss` global partial), add:
```scss
// External links only — the ONLY place the hand cursor is correct.
a[href] {
    cursor: pointer;
}
```

Optionally introduce `--cursor-link: pointer` as a distinct design token so future
file-path openers can use `cursor: var(--cursor-link)` rather than a raw keyword:
```scss
--cursor-link: pointer;    // hyperlinks and external file-path openers only
```

### Step 4 — Remove remaining hardcoded `cursor: pointer` in SCSS

After steps 1–2 flip the cascade, grep for surviving `cursor: pointer` in SCSS and
remove them (they are now doubly-wrong: violating the rule AND overriding the fixed
baseline). The list in §4 above enumerates all sites.

### Step 5 — Fix TSX Tailwind usages

For each `cursor-pointer` class in the TSX files listed in §4, remove it (or replace
with `cursor-default` if the Tailwind reset needs to be explicit).

### Step 6 — Add `init/error-display.ts` inline style fixes

```diff
-"padding:9px 18px;border-radius:7px;border:none;cursor:pointer;…"
+"padding:9px 18px;border-radius:7px;border:none;…"
```

---

## 6. Implementation order

| Priority | Step | Blast radius | Why |
|---|---|---|---|
| P0 | `theme.scss:308` token flip | Auto-fixes 3+ token-consumer sites | Zero per-file churn |
| P0 | `button.scss:11` baseline flip | Auto-fixes majority of SCSS sites via cascade | Zero per-file churn |
| P0 | Add `a[href] { cursor: pointer; }` | Enables the correct rule globally | 1 line |
| P1 | Remove hardcoded `cursor: pointer` from SCSS | Clears remaining overrides | File-by-file |
| P1 | Remove `cursor-pointer` Tailwind from TSX | Clears in-markup overrides | File-by-file |
| P2 | Fix `error-display.ts` inline styles | Clears the two inline override sites | 2 lines |
| P3 | Add CI grep gate | Prevents regression | `scripts/check-cursor.sh` |

P0 is safe to ship in one small commit and immediately resolves the visible symptom on
hamburger, menus, tabs, window controls, pane headers, and status bar. P1–P2 are cleanup
that can follow file-by-file. P3 keeps it from drifting back.

---

## 7. What NOT to change

- `app/view/term/xterm.css:134-136` — `.xterm-cursor-pointer` — **keep `pointer`**; this is
  the only genuinely correct use.
- Drag cursors (`cursor: grab / grabbing`) — not in scope; these are correct.
- Resize cursors (`cursor: ew-resize`, `ns-resize`, etc.) — not in scope; correct.
- Disabled cursors (`cursor: not-allowed`) — not in scope; correct.
- Text-input cursors (`cursor: text`) — not in scope; correct.
- `tailwindsetup.css:84` — the `.cursor-pointer` Tailwind utility definition itself is
  fine; only its misuse on non-link elements is incorrect.
