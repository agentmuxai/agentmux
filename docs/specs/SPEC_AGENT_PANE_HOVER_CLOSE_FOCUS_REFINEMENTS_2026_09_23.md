# SPEC: Agent pane refinements — expanded tool-call hover text, close-last-tab returns to My Agents, composer autofocus on launch

**Date:** 2026-09-23
**Status:** implemented (branch `clamk/agent-pane-hover-close-focus`) —
§6 records where the implementation deliberately differs from the design
below.
**Repo state:** `agentmuxai/agentmux` main @ `cdee2e2` (design);
rebased onto main @ `934b335` — see §6 "Rebase onto #3519"
**Related:**
`docs/reports/REPORT_TOOL_CALL_PEEK_SUPPRESSED_WHEN_EXPANDED_2026_09_04.md` (#2972 — the decision §1 reverses),
`docs/specs/SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`,
`docs/specs/SPEC_TOOL_HOVER_CONSOLIDATION_2026_05_28.md`,
`docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (line 262 — "closing the last/only tab closes the pane", which §2 amends for agent panes),
`docs/specs/SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`,
`docs/specs/SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md` (already requires focus to land on the agent input — §3 is what finally makes that true).

Three independent agent-pane UX refinements. Each has its own section,
root cause, design, and tests; they can ship as one PR or three.

---

## 1. Hovering an expanded tool call should show the full, word-wrapped tool call

### 1.1 Symptom

Hovering a **collapsed** tool call pops the peek overlay (pinned to the
row's right edge, tracks the mouse vertically) with the time, the token
estimate, and the full tool call (command / file path / pattern / query /
URL) word-wrapped. Hovering an **expanded** tool call shows the same
overlay with only the time and token lines. The header summary is
ellipsis-truncated in both states, so once a row is expanded there is no
way to read the full call.

### 1.2 Root cause — one JSX condition

`frontend/app/view/agent/components/ToolBlock.tsx:498`:

```tsx
<Show when={cmdText() && !expanded()}>
    <div class="agent-node-peek-tooltip-body">{cmdText()}</div>
</Show>
```

- `cmdText()` (`ToolBlock.tsx:315-317`) is `extractToolDetail(tool, params)`
  (`stream-parser.ts:117-147`).
- `expanded()` (`ToolBlock.tsx:224`) is `props.pinned || autoExpanded() || userHolding()`.
  It is true for pinned rows, and also while a tool is running, pending
  approval, or held open after it completes.
- The wrapping comes from `.agent-node-peek-tooltip-body`
  (`styles/_document-nodes.scss:2144-2151`: `white-space: pre-wrap;
  overflow-wrap: break-word`). This rule is not gated on state, so the
  body wraps correctly the moment it renders.
- The header truncation (`_document-nodes.scss:223-233`, `259-272`) is not
  gated on `.collapsed` either. Expanding a row never un-truncates it.

PR #2972 dropped `!expanded()` from the overlay as a whole, so time and
tokens now show on expanded rows. It deliberately kept the gate on the
command line, reasoning that the expanded panel already shows it "in
context". That is only partly true:

- `ToolBlockOverlay` shows only a status label.
- Bash shows its command only in the finished-result view
  (`BashOutputViewer.tsx:73`), not while output streams
  (`ToolOverlayLog.tsx:128-133`).
- Other tools show a pattern or description at best, and never the full
  set of parameters.

Either way, the product decision is now explicit: hover behaves the same
in both states.

### 1.3 Change

1. `ToolBlock.tsx:498` → `<Show when={cmdText()}>`.
2. No other logic change. `hasAnyPeekContent()` (`ToolBlock.tsx:352-354`)
   already counts `cmdText()`, and the overlay's `show=` is already
   ungated.
3. Rewrite the three comments that describe the suppression as intended:
   the file header (`ToolBlock.tsx:19-28`), `310-314`, and `481-490`.
   Point them at this spec.
4. Add a status note to `SPEC_TRANSCRIPT_NODE_HOVER_PEEK_2026_08_03.md`
   and `REPORT_TOOL_CALL_PEEK_SUPPRESSED_WHEN_EXPANDED_2026_09_04.md`
   saying their "command stays suppressed when expanded" rule is
   superseded by this spec's §1.

### 1.4 UX consideration

On an expanded row the overlay floats over the panel body, which may be
streaming output the user is reading. That is already true for the
time/token overlay since #2972. The command body can be tall, but the
overlay is `overflow-y: auto` (`_document-nodes.scss:2156-2179`) and only
shows while the mouse is on the row. No mitigation is proposed. If it
proves noisy, cap `.agent-node-peek-tooltip-body` height on
`.agent-tool-block.expanded` rows rather than bringing the gate back.

### 1.5 Tests (`ToolBlock.test.tsx`, `describe("command tooltip")`)

Flip the three tests that assert the suppression:

- `263-279` "expanded (pinned) … but not the redundant command line" →
  assert `.agent-node-peek-tooltip-body` **is** present with the full
  `cmdText`.
- `344-377` "keeps a running tool's command line suppressed through
  completion" → assert the body is present while running, while held
  open, and after completion.
- `379-395` "hides only the command line … the instant an
  already-hovered tool gets pinned open" → assert the body **stays**
  after `setPinned(true)`.

Add one test with a long, space-free command (for example a 400-character
path) on a pinned row. It asserts the body renders the full untruncated
string. jsdom can't test layout, so wrapping is covered by the unchanged
SCSS rule, not by a test.

---

## 2. Closing an agent pane's tab should return to My Agents

### 2.1 Symptom

In-pane tabs are the pills in the pane's own tab strip, one per block in
the layout leaf's `blockStack`. They are not the workspace tab bar.
Closing the last or only agent tab in a pane closes the whole pane. The
user expected to land back on the agent start page (My Agents), ready to
pick another agent.

### 2.2 Current behavior

The agent pane overrides the shared `PaneChrome` close handler through
`buildAgentPaneChromeModel` in `frontend/app/view/agent/agent-view.tsx`:

- `handleTabClose` is at `439-443`.
- It is wired as `onClose` at `471-474`.

It calls `closeBlockInStack` (`frontend/layout/lib/layoutStack.ts:162-202`),
which behaves as follows:

| Case | Today |
|---|---|
| Stack has one member (the last tab) | `closeNode` → leaf removed from the layout → `DeleteBlock`. The pane disappears. |
| Stack has more than one member | Block dropped from the stack. If it was active, the right-hand neighbor becomes active. `onNodeDelete` → `DeleteBlock`. |

`DeleteBlock` → `sagas/delete_block.rs:137` →
`blockcontroller::delete_controller`. Block deletion is what stops the
agent's CLI.

The picker is shown whenever the block's `meta.agentId` is empty:
`AgentBlockContent`, `agent-view.tsx:181-303`, with the "lost agentId"
branch at `240-246`. A fresh `{view: "agent"}` block therefore opens on
My Agents. That is exactly what the "+" → Agent tab already does
(`addWidgetAsPaneTab`, `layoutStack.ts:78-114`).

### 2.3 Rejected approach: `AgentViewModel.backToPicker()`

`agent-model.ts:246-266` already clears the agent meta keys, and the
picker reappears when it does. It has one caller, the context-exceeded
"New session" path at `agent-view.tsx:1910`. Reusing it for tab close is
tempting, but it is the wrong tool:

- **It does not stop the CLI.** The SetMeta handler
  (`websocket.rs:880-935`) only writes meta. Today the controller dies
  because the block is deleted. Keeping the block would leave a live
  process behind a picker.
- **It leaves stale meta behind:** `cmd:cwd`, `term:ctx-tokens`,
  `agentInstanceId`, and so on. It also doesn't clear
  `agent:historyTabFor`, so a History tab would stay a History tab.
- It keeps the same block id. Anything keyed on that id carries over:
  composer drafts, snapshot persistence, and fork/instance links.

### 2.4 Design: replace the closing tab with a fresh picker tab, then close it normally

In `handleTabClose`, when the tab being closed is the **last member of
its owning leaf** and is a **loaded agent tab** (`meta.view === "agent"`
and `meta.agentId` set, which includes History tabs):

1. `await addWidgetAsPaneTab(layoutModel, node.id, { meta: { view: "agent" } })`.
   This creates a blank agent block and pushes it as the active member.
   The stack now has two members.
2. `await closeBlockInStack(layoutModel, node.id, targetBlockId)`. This
   now takes the more-than-one branch: it removes the old block from the
   stack and deletes it through the normal saga, stopping its controller,
   removing its shell sub-block, and so on. Because the old block is no
   longer active, the new picker block stays active.

Every other case keeps today's behavior. That covers a non-last tab, a
non-agent tab in an agent-started pane, and a picker tab (no `agentId`),
where closing it still closes the pane.

Implementation notes:

- Put this in a small, testable helper, for example
  `frontend/app/view/agent/close-agent-tab.ts` →
  `closeAgentTab(layoutModel, blockId)`, and call it from
  `handleTabClose`. `closeBlockInStack` itself must stay generic: every
  pane type shares it, and `quick-fork.ts:153` relies on its exact
  semantics.
- The helper needs the leaf's stack length. `effectiveStack` in
  `layoutStack.ts:48` is module-private; export it, or add a thin
  exported `stackSize(node.data)`. Read meta with
  `getBlockMetaKeyAtom(blockId, "agentId")` / `"view"`.
- **Failure fallback:** if `pane.open` rejects, fall through to plain
  `closeBlockInStack` (today's behavior) rather than leaving the tab
  un-closable.
- **Double-click guard:** step 1 awaits an RPC. Keep a module-level
  `Set<blockId>` of in-flight closes and ignore a repeat close of the
  same id. Without it, a fast double-click adds two picker tabs.
- **Order matters:** add first, then close. Closing first would hit the
  one-member branch and destroy the pane before the replacement exists.
  `addWidgetAsPaneTab` already deletes an orphaned new block if the pane
  vanishes mid-RPC (`layoutStack.ts:104-108`).
- **Focus:** after the swap, give the new picker block focus with
  `refocusNode(newBlockId)` so keyboard pane navigation keeps working.
  The picker filter stays un-autofocused per
  `AgentPicker.test.tsx:628-633` (Q2). This only keeps the pane
  focused; it does not focus the filter input. *(Not implemented; see §6.)*

### 2.5 Out of scope, deliberately unchanged

- **Header × (`blockframe.tsx` ~250) and Cmd:W (`keymodel-nav.ts:34-46`)**
  still close the whole pane. They are "close pane" affordances, not
  "close tab". Cmd:W closing only the active tab is tracked separately by
  `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md:490`. When that
  lands, it should route through the same `closeAgentTab` helper.
- **Panes that started as a terminal:** `pane-leaf-chrome.tsx:360-366`
  fixes the chrome model to the pane's first view. An agent tab in a
  terminal-started pane closes through the default handler, so this rule
  doesn't apply there. Follow-up: have `PaneChrome` ask the closing
  tab's own view model for `onClose`.
- **Close confirmation:** the "processes still running" prompt
  (`useAgentCloseConfirm.ts:44-65`) is not raised by a pill ×, today or
  after this change. That gap already exists and is noted here, not
  fixed here.

### 2.6 Tests

New `close-agent-tab.test.ts` with a mocked `LayoutModel` and RPC,
following the `layoutStack.test.ts:276-430` fixtures:

- Last tab, agent loaded → `pane.open` called with `{view: "agent"}`; the
  stack ends as `[newId]`, active is `newId`; `DeleteBlock(oldId)` is
  called; `closeNode` is **not** called.
- Last tab, History tab (`agentId` + `agent:historyTabFor`) → same as
  above.
- Last tab, picker tab (no `agentId`) → `closeNode` called; no
  `pane.open`.
- Non-last agent tab → today's behavior: neighbor activated, block
  deleted, no `pane.open`.
- `pane.open` rejects → falls back to `closeNode`.
- Two rapid closes of the same id → `pane.open` called once.

---

## 3. When an agent loads, the composer should take keyboard focus

### 3.1 Symptom

Clicking an agent in My Agents loads it in the pane, but typing goes
nowhere. The user has to click the input box first.

### 3.2 Root cause

Nothing in the agent pane ever moves focus to the composer, and the
element that did have focus is removed:

1. Each My Agents row is a real `<button>`
   (`MyAgentsList.tsx:997-1003`), so clicking it focuses the button.
   `focusin` → `block.tsx:167-190 handleChildFocus` marks the pane as
   layout-focused.
2. `handleRowClick` → `AgentPicker.handleReattach`
   (`AgentPicker.tsx:390-438`) → `await model.launchAgentDefinition(...)`
   (`agent-model.ts:393+`). This is a chain of RPCs; it writes
   `meta.agentId` via `SetMetaCommand` at `771-774`.
3. `AgentBlockContent` mounts `AgentPresentationView` and the
   `AgentFooter` `<textarea class="agent-input">`
   (`AgentFooter.tsx:1103-1128`) in the same block. There is no block
   remount and no layout focus event. The textarea is never disabled; it
   is usable the moment it mounts.
4. The picker fades out and unmounts after `PICKER_FADE_OUT_MS = 200`
   (`agent-view.tsx:137`, `229-232`). That removes the focused button, so
   `document.activeElement` falls back to `<body>`.
5. Nothing refocuses:
   - The textarea has no `autofocus` and no focus on mount. Its existing
     `.focus()` calls (`AgentFooter.tsx:613`, `795`) all follow a user
     action.
   - `AgentViewModel.giveFocus()` is hard-coded to `return false`
     (`agent-model.ts:924-926`). Every generic focus path (`setFocusTarget`,
     `refocusNode`, `focusManager`, `openOrFocusPaneByView`) therefore
     lands on the block's hidden dummy input, never the composer.

Point 5's `giveFocus` stub also breaks keyboard pane navigation into an
agent pane, the fork-picker "Switch to existing" jump
(`AgentPicker.tsx:489`), and window re-activation, which
`SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md` requires to focus
the agent input. The terminal gets this right:
`termViewModel.ts:498-511` and `term.tsx:213-215`.

### 3.3 Design — two parts

**Part A — implement `giveFocus()` for real.** (The ref below was
superseded by #3519's `focusTargetRef` at rebase — see §6.)

- `AgentViewModel` gets `composerFocusRef: { current: (() => boolean) | null }`.
  This mirrors the existing `voiceTargetRef` pattern.
- `AgentFooter`'s `onMount` sets it to a function that focuses
  `textareaRef` with `preventScroll: true`, puts the caret at the end, and
  returns `true`. `onCleanup` clears it (as in `AgentFooter.tsx:733-779`).
- `giveFocus()` returns `this.composerFocusRef.current?.() ?? false`.
- **Selection guard:** `giveFocus` returns `false` without focusing if
  `window.getSelection()` holds a non-collapsed range inside this block.
  `handleBlockClick` → `setFocusTarget` fires on the click that ends a
  drag-select of transcript text. Moving focus into the textarea there
  would wipe the user's selection. This guard is new behavior surface,
  which is why Part A needs its own test.
- **Don't steal from panels that own focus:** if `document.activeElement`
  is already an input, textarea, or `[autofocus]` element inside this
  block other than the composer, return `true` without moving focus. That
  covers `InAppLoginPanel.tsx:48` and `AgentDecisionPanel` (`autofocus`,
  L396).

**Part B — a one-shot "focus the composer once it mounts" request from the launch path.**

- `AgentViewModel.pendingComposerFocus: boolean`.
- Set it to `true` in `launchAgentDefinition` right before the
  `SetMetaCommand` that writes `agentId` (`agent-model.ts:~771`). Set it
  only on the success path: the early `return false` exits (unknown
  provider or missing Node.js, `406-417`) leave the picker up, with
  nothing to focus.
- `AgentFooter`'s `onMount` consumes it (read, then clear) and calls the
  Part A focus function if both hold:
  - The flag was set.
  - `focusedBlockId() === model.blockId`, meaning the user hasn't clicked
    another pane during the launch round-trips. That holds because the
    picker button inside this block still has focus at that moment.
- This fires as the footer mounts, before the picker's 200 ms unmount, so
  focus moves directly from the button to the textarea. `<body>` is
  never in between.
- It is a one-shot flag, not "focus on every mount." `AgentFooter`
  remounts on every pane tab switch (see the `composerDrafts` comment,
  `AgentFooter.tsx:37-51`) and on app start/layout restore for every
  agent pane. Unconditional mount-focus would make every restored agent
  pane fight for focus.

`launchAgentDefinition` is the common path for every launch source:
My Agents reattach, fork, template launch (`AgentPicker.tsx:308/617/685`),
and the launch modal. Setting the flag there covers all of them without
touching each caller.

### 3.4 Side effects to verify during implementation

- `AgentRuntimeDropup.tsx:208-259` closes itself on `focusin` into the
  textarea. It should see programmatic focus the same way.
- Focusing inside the block triggers `handleChildFocus` → the
  `main_window_focus` IPC, which reclaims Win32 focus from a browser pane
  that last held it. That is desirable; confirm keystrokes land on
  Windows when a Browser pane was previously active.
- Resume (`continueSessionId`) with history replay: the transcript
  loading overlay (`agent-view.tsx:823-855`) must not block programmatic
  focus. It doesn't; it is an absolutely positioned overlay, not
  `inert`.

### 3.5 Tests

- `agent-model.test.ts`:
  - `giveFocus()` returns `false` with no handle and delegates when a
    handle is registered.
  - `launchAgentDefinition` sets `pendingComposerFocus` on success and
    not on the early-return paths.
- `AgentFooter.test.tsx`:
  - Mounting with `pendingComposerFocus = true` and the block focused →
    `document.activeElement` is `.agent-input`, and the flag is cleared.
  - With the flag false → no focus change.
  - With the flag true but a different block focused → no focus change.
  - A remount after the flag is consumed → no focus change.
  - With a non-collapsed selection in the block, `giveFocus()` → returns
    `false`, and the selection is preserved.
- Manual check in `task dev`:
  - My Agents → click an agent → type immediately; the text lands in the
    composer.
  - Repeat with a template launch and a fork.
  - Then press Alt+Tab away and back; the composer is re-focused, which
    confirms `SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE`.

---

## 4. Interaction between §2 and §3

After §2's swap, the new picker tab is focused (§2.4). Picking an agent
in it then flows through §3 exactly like a fresh pane. The picker
button is focused, so `focusedBlockId()` matches and the composer takes
focus. No extra wiring is needed.

## 5. Decisions and open questions

**Decided (user, 2026-09-23):**

1. **Closing a non-last agent tab** closes that tab and activates the
   neighbor, exactly as today. It does not return to My Agents.
2. **§1's full tool call shows on hover while the tool is still running**,
   not only once it has finished or been pinned. This is the same
   behavior in every expanded state.

**Open — recommendation in bold:**

3. **Closing a fork pill that lives in a *different* pane.** These are the
   "extra tabs" (`agent-view.tsx:390-393`): forks of this conversation
   that are open as their own top-level pane, shown here as shortcut
   pills. Clicking one jumps focus to that pane. Its × closes the fork
   there: `handleTabClose` resolves the owning node, which is the other
   pane. When that fork is the other pane's only tab, §2.4 as written
   would turn that pane into My Agents. The user would get an empty picker
   pane elsewhere on screen as a side effect of acting in this one.
   **Scope §2.4 to tabs owned by the pane the × was clicked in.** A
   cross-pane fork pill keeps today's semantics: the fork's block is
   closed, and if it was its pane's only tab, that pane closes. In
   `closeAgentTab`, apply the replace-with-picker path only when the
   owning node is the calling pane's own `nodeModel.nodeId`. Add a test
   for a cross-pane fork pill that is the last tab of its pane →
   `closeNode` on that pane, no `pane.open`.

---

## 6. Implementation notes (deviations from the design above)

**Rebase onto #3519 (`SPEC_PANE_SELECT_AUTOFOCUS_2026_09_22.md`).** While
this branch was open, main landed #3519, which independently built §3 Part A:
`AgentViewModel.focusTargetRef` (the live textarea), `giveFocus()` focusing
it, and `focusManager.claimFocusOnMount` on every footer mount of the active
tab's selected pane. That already covers a click in My Agents. The rebase
keeps #3519's structure and drops this branch's `composerFocusRef`; what
remains of §3 on top of it:
- `giveFocus()` routes the textarea through `focusComposer`, so the guards
  in §3.3 still apply (no focus over a text selection in the pane or a
  focused login/decision panel; caret to the end of a restored draft).
- The one-shot launch request (Part B) still exists, because
  `claimFocusOnMount` deliberately does nothing while a modal is open —
  launching from the launch modal would otherwise never focus. When a
  request is pending, the footer runs `focusComposerWhenReady` *instead of*
  `claimFocusOnMount`; every other mount uses `claimFocusOnMount`.

**§2 — the close confirmation is asked before the swap.** Also since the
design, #3422 made `closeBlockInStack` ask `beforeNodeDelete` (the
busy-agent confirmation) for a single tab too. Asking at that point — after
the picker tab was added — would leave a stray My Agents tab whenever the
user cancels. So `closeAgentTab` asks first and closes with
`closeBlockInStack(..., { confirmed: true })`; `closeBlockInStack` and
`closeNode` gained that optional flag (skip asking again), default off.

**§2 — the pane's own `LayoutModel`.** `closeAgentTab` takes the
`layoutModel` from its caller (`nodeModel.layoutModel`) rather than
`getLayoutModelForStaticTab()`, matching #3392 /
`SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md`.

**§2 — no explicit focus call after the swap.** `refocusNode(newBlockId)`
runs before the new block's component has mounted and registered, so it
could only reach the dummy-focus fallback, which doesn't exist yet
either. Today's non-last-tab close doesn't move focus either. Left as is;
the picker filter stays un-autofocused (Q2) regardless.

**§2 — where it lives.**
- The code is `frontend/app/view/agent/close-agent-tab.ts`
  (`closeAgentTab({ layoutModel, ownNodeId, blockId })`), called from
  `buildAgentPaneChromeModel`'s `handleTabClose`.
- The replacement tab uses the `defwidget@agent` blockdef from widget
  config, the same one "+" → Agent uses, so it is indistinguishable from
  a freshly added tab.
- The swap holds the leaf reveal gate, like `openOrFocusHistoryTab`.
- `effectiveStack` did not need exporting: the helper reads
  `node.data.blockStack` directly, as `open-history-tab.ts` does.

**§3 Part B — per-block request registry, not a model field.**
- The code is `frontend/app/view/agent/composer-focus.ts`.
- `launchAgentDefinition` may target a *different* block than its own
  model's (`quickForkAgent` passes `targetBlockId`), and that block has
  its own `AgentViewModel`. So the request is keyed by block id in a
  module-level map (`requestComposerFocus` /
  `takeComposerFocusRequest` / `cancelComposerFocusRequest`) rather than
  stored as a boolean on `this`.
- A request expires after 10 s, so a request no mount ever consumed (for
  example a relaunch into an already-mounted footer) can't steal focus on
  some unrelated later mount.

**§3 Part B — the launch modal needed a wait, not a single attempt.**
Launching from the launch modal (rather than a direct click in My Agents)
keeps the modal open until `launchAgentDefinition` fully resolves
(`modal-dispatch.tsx:78-86`). While it is open, the pane-scope region
lock marks the pane content `inert` (`modal-region-lock.ts`), so a
`focus()` at footer mount silently does nothing. On close, `modal.tsx`
restores focus to the element that opened it: the picker card, which is
about to be removed.

So `focusComposerWhenReady` retries every 50 ms for up to 5 s, and on
each check it:
- **focuses** when focus is on `<body>`, the dummy input, or a non-text
  element of this pane (the picker button), and the composer isn't under
  `[inert]`;
- **waits** while focus is inside a `.modal-root`, or the composer is
  inert;
- **stops** once focus is in the composer, in another pane, or in any
  other text entry (a login/decision panel, or an input elsewhere).

A picker click resolves on the first check, before the picker unmounts.

**§3 — `focusedBlockId() === blockId` gate replaced.** For quick-fork,
the new block isn't the one holding DOM focus, so the check became "focus
is not in a *different* block" (the stop rule above).

**§3.5 — `agent-model.test.ts` not written.** `AgentViewModel` has no
test harness: nothing in the suite constructs one. The model's part is
three lines:
- `giveFocus()` passes `focusTargetRef` (#3519) through `focusComposer`;
- `requestComposerFocus(blockId)` runs before the agentId `SetMetaCommand`;
- `cancelComposerFocusRequest(blockId)` runs in the launch `catch`.

The behavior is covered through `composer-focus.test.ts` (16 cases) and
`AgentFooter.test.tsx` → "AgentFooter keyboard focus" (4 cases).

**Local test runs on Windows.** Every suite under
`frontend/app/view/agent/` fails to *load* on Windows with
`ERR_INVALID_ARG_VALUE … 'file:///@solid-refresh'`, a known
pre-existing issue (`TRACKING_AGENT_AVAILABILITY_AND_BACKGROUNDING_2026_09_17.md`).
Local runs for this change used an uncommitted config that re-registers
`vite-plugin-solid` with `hot: false`. With it, the full frontend suite
passes (after the rebase: 4782 tests, 3 skipped), and
`tsc --noEmit -p tsconfig.citypecheck.json` is clean. Not verified in a
running app yet.
