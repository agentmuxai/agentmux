# SPEC: switch account from the composer's sign-in chip, cap the height of jekt messages, and outline Swarm rows on hover

**Author:** lark
**Date:** 2026-09-26
**Status:** active. Part B implemented in #3861, Part C in #3862. Part A is in progress; §5 Q1 is answered from the
backend code but still wants one live check.
**Related:** `SPEC_ACCOUNT_EMAIL_IN_ARMORY_2026_09_23.md` (where the chip's email comes from),
`SPEC_AGENT_LOGIN_FLOW_TIGHTENING_2026_09_04.md` (the failure row's "Bind account" action, which this
reuses), `SPEC_ARMORY_BIND_TO_AGENT_CONTEXT_MENU_2026_08_09.md` (the live-apply pair a bind performs),
`SPEC_TOOL_PREVIEW_HEIGHT_THIRD_AND_FOLLOW_LATEST_2026_09_25.md` (the height Part B copies),
`SPEC_TOOL_PREVIEW_SCROLL_CHAINING_2026_07_03.md` (why Part B needs no wheel hand-off code),
`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §3.3 (the jekt bubble),
`SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md` §2.2 (the hover tint Part C replaces).

All line numbers are against `main` at `5555d521a`.

---

## 1. Request (user, 2026-09-26)

1. **Account switch.** The agent pane's composer shows the logged-in provider account's email. Make it a
   hyperlink: clicking it lets you select another account from the **same provider**.
2. **Jekt height.** Jekt messages get a maximum height, similar to the tool-call previews (the recent
   work in the tool-preview height spec). A message taller than that scrolls inside its box.
3. **Swarm hover.** The Swarm view got a hover colour. Refine it: not a full block of colour, only the
   border. Hovering an entry draws the same border around it that selection does. The only difference
   is the shade: use the colour an unselected pane's border uses.

---

## 2. Part A: click the account chip to switch accounts

### A.1 Today

The chip is the `auth` slot in `AgentComposerStrip.tsx` (~lines 780–818). It is a plain, non-interactive
`<span class="agent-composer-strip-auth">`: a coloured dot plus the label. The label is
`shortenEmail(props.authEmail)`, or "Logged in" when the account recorded no email, or "Not logged in".
The slot is registered with `interactive: false` (~line 793).

`agent-view.tsx` already has everything needed to list and apply an alternative account:

| Piece | Where | What it does |
|---|---|---|
| `accountCache` | ~line 1987 | Live, app-wide account list (`subscribeAccountChanges`). |
| `linkedAccountId` | ~line 1998 | This agent's currently linked account for its provider. Refreshed on `agentidentities:changed`. |
| `authEmail` | ~line 2026 | Email of `linkedAccountId`; feeds the chip. Updates by itself after a rebind. |
| `bindCandidates` | ~line 2028 | `computeAccountBindCandidates(provider, accounts, linkedAccountId)`: same provider (alias-resolved), OAuth-class, `status === "valid"`, current account excluded, newest first. |
| `onBindAccount(e)` | ~line 2034 | One candidate: binds at once. Two or more: opens a `ContextMenuModel` picker at the click. |
| `status.bindExistingAccount(acct)` | `useAgentControllerStatus.ts` ~line 1362 | Links the account, refreshes the block's `cmd:env` config-dir override, and force-restarts the controller (`--resume`). Failures surface as `authNotice`. |

Today these are wired only to the failure row's "Bind account" action (`useAgentFailure.ts`), so a healthy
pane has no way to switch account. The "same provider" requirement in the request is already what
`computeAccountBindCandidates` does; nothing new is needed to find the accounts.

### A.2 Change

Make the chip a link when there is somewhere to switch to. Reuse the existing candidate list and bind
path; add no new backend calls.

**Interaction**

- The chip renders as a `<button type="button">` styled as a link, not an `<a>`: it performs an action, it
  does not navigate. It carries `aria-haspopup="menu"`.
- It is a link only when **all** hold: `authStatus === "authenticated"`, at least one candidate exists, and
  the pane is not busy (A.3). Otherwise it renders exactly as today, as a `<span>`.
- Applies to both labels of the authenticated state: the email, and "Logged in" (account with no email).
  "Not logged in" is unchanged; the login CTA and the failure row's own "Bind account" cover that state.
- Click opens a `ContextMenuModel` menu anchored at the click, **always**, even for exactly one candidate.
  This differs from `onBindAccount`, which binds a lone candidate immediately. That is right for the
  failure row, where the agent is already broken. Here the agent works, and a switch restarts it (A.3), so
  the user must pick a named destination.
- Menu rows are `Switch to <accountLabel(acct)>` (the email, else the account name, the same
  `accountLabel` the failure row uses). Selecting one calls `status.bindExistingAccount(acct)`.
- After the bind, `agentidentities:changed` refreshes `linkedAccountId`, and `authEmail` and the candidate
  list recompute. The chip shows the new email with no extra code. The old account now appears in the menu.
- A failed bind shows the existing `authNotice` banner. No new error UI.

**Visual**

- Keep the dot and its colour (`--ok` green while authenticated). Underline the label with a dotted
  underline at rest, solid on hover and keyboard focus, and use `cursor: pointer`. No chevron: the strip
  is width-constrained and the slot is measured (`computeComposerRows`).
- Button reset: `background: none; border: 0; padding: 0; font: inherit; color: inherit`, plus a visible
  `:focus-visible` outline. Put it in `_composer-strip.scss` beside `.agent-composer-strip-auth`.
- `title`: "Signed in as <email>. Click to switch account." (`title` already carries the full, unshortened
  email today; keep that.)

**Wiring** (follow `useAgentFailure`'s `bindCandidates` + `onBindAccount(e)` convention so the strip stays
presentational)

- `AgentComposerStrip` gains two optional props:
  `switchAccountCandidates?: { id: string; name: string }[]` and `onSwitchAccount?: (e: MouseEvent) => void`.
- `agent-view.tsx` passes `bindCandidates().map(a => ({ id: a.id, name: accountLabel(a) }))` and a new
  `onSwitchAccount`. To avoid a second copy of the menu code, extract the "build the picker and show it"
  half of `onBindAccount` into one helper, called by `onBindAccount` (which keeps its one-candidate
  shortcut) and by `onSwitchAccount` (which never shortcuts).
- `interactive: false` becomes `interactive: true` on the `auth` slot. It is now clickable, and the strip
  orders interactive slots to the outer edge (`orderKeysForEdgePriority`).

### A.3 Behaviour to get right

1. **A switch restarts the agent.** `bindAccountToAgent` ends in
   `ControllerResyncCommand{ forcerestart: true }`, because a running CLI never re-reads its env. A turn
   in flight would be killed. So the link is **inactive while the pane is busy**: use the same `paneBusy()`
   the strip already receives as `loading`, and also `compacting`. While busy the chip is the plain span,
   and its tooltip says why ("Switching accounts restarts the agent. Wait for the current turn to finish.").
   This is a guard, not a dialog; the user picks a named account, which is the confirmation.
2. **Expired accounts are not offered.** `computeAccountBindCandidates` is `valid`-only on purpose: after
   any bind, `recheckAuthAfterBind` trusts `CheckCliAuthCommand`, which can report an expired account as
   authenticated. Offering a known-expired account risks the pane declaring itself healthy when the next
   turn will fail (module doc comment, reagentx P1 on that change). Keep the rule. See open question Q2.
3. **No candidates, no link.** With one account only, the chip is exactly today's chip. Nothing suggests an
   affordance that does nothing.
4. **Layout invariants.** The strip pins `auth` left and `ctx` right (`pinnedPairs`, Rev 9,
   `SPEC_COMPOSER_STRIP_AUTH_COMPACT_SIDE_STABILITY_2026_09_16.md`). Making `auth` interactive changes only
   its position among the *left* slots (interactive first). The runtime dropup is already interactive and
   first, so the expected order is unchanged. Confirm with the existing strip tests; do not assume.
5. **The provider session restarts.** The new account's config dir cannot see the old CLI session, so a
   switch starts a fresh CLI session that carries AgentMux's own record of the conversation, and the
   transcript shows a session-outcome row saying so (Q1 in §5). Expected, and already implemented in the
   backend; the menu does not need to warn about it.
6. **Multiple panes, one agent.** A bind is per agent definition. Another open pane of the same agent picks
   the change up through the same `agentidentities:changed` event that the auto-unblock path already uses.

### A.4 Tests

- `AgentComposerStrip.test.tsx`: authenticated + candidates + not busy renders a button that calls
  `onSwitchAccount` with the click event; no candidates renders the span; busy renders the span with the
  explanatory title; "Logged in" (no email) is also a button; unauthenticated is unchanged.
- Strip layout: the existing ordering and `pinnedPairs` tests still pass with `auth` `interactive: true`;
  add one that pins the resulting left-zone order.
- `agent-view` / helper: `onSwitchAccount` with exactly one candidate still opens a menu, whereas
  `onBindAccount` with one candidate still binds directly. Menu rows are `Switch to <label>`, and choosing
  one calls `bindExistingAccount` with that account.
- Live check with the CDP harness (`scripts/ui-screenshots/`): switch on a real pane, watch the restart,
  confirm the chip changes to the new email and the agent answers the next message (this is Q1).

---

## 3. Part B: cap the height of a jekt message

### B.1 Today

`JektBubble.tsx` renders an expanded jekt as `.agent-jekt-content`, holding
`<pre class="agent-jekt-body">` (the message), a metadata line, and a `<details class="agent-jekt-raw">`
whose `<pre>` repeats the whole raw payload (~lines 104–130). Styles are in `_document-nodes.scss`
(~lines 1013–1051). None of these has a height limit, so a long jekt, such as a pasted review or a log,
pushes the rest of the conversation off-screen. Collapsed bubbles are one line and are not affected.

The tool-preview cap is `max-height: calc(50vh / 3)` on `.agent-tool-panel` (~line 378 of the same file):
about 13 lines on a 1400 px window, relative to window height, and shorter output renders at its natural
height.

### B.2 Change

Cap the jekt body at the **same value as tool previews**, and scroll inside it.

```scss
// _document-nodes.scss — .agent-jekt-bubble .agent-jekt-content
.agent-jekt-body {
    max-height: calc(50vh / 3);   // same as .agent-tool-panel
    overflow-y: auto;
}
.agent-jekt-raw pre {
    max-height: calc(50vh / 3);
    overflow-y: auto;
}
```

- **Share the value.** Declare one SCSS variable near the top of `_document-nodes.scss`
  (`$transcript-preview-max-height: calc(50vh / 3)`) and use it for both the tool panel and the jekt rules,
  so the two stay "similar" when someone changes one. The persistent-shell log keeps `50vh` (unchanged from
  the tool-preview spec §2.A.3).
- **Cap the body, not the whole bubble.** The metadata line (sender, MSGID, trust, time) stays visible
  under a long message. Without this, the trust fields the human is meant to read are the first thing
  scrolled away.
- **Cap the raw payload too.** It contains the whole message again plus the marker, so it would
  reintroduce the same problem one click later.
- A short jekt renders at its natural height. There is no minimum height; on an 800 px window the cap is
  about 7 lines, the same trade-off the tool-preview spec accepted.
- `box-sizing: border-box` is global (`reset.scss`), so `max-height` bounds the padded box. The body keeps
  its `pre-wrap`, so there is no horizontal scrollbar for normal text.
- The body starts scrolled to the top. A jekt is a finished message, not a stream, so no follow-latest
  behaviour is needed (unlike the tool preview's Part B).

### B.3 Scroll chaining needs no code

`ToolOverlayLog.tsx` hands wheel events to the outer pane by hand (~lines 158–181) **only because** that
box sets `overscroll-behavior: contain`, which blocks the browser's native chaining. The jekt body sets no
`overscroll-behavior`, so the default (`auto`) applies: at the top or bottom of the box, the wheel passes
to `.agent-document` natively. **Do not** copy the tool box's JS hand-off, and do not add
`overscroll-behavior: contain`.

Verify this live (the chaining spec found that behaviour differs from what the CSS suggests until it was
checked in the running app): wheel a capped jekt to its end, and confirm the pane keeps scrolling.

### B.4 Virtualizer estimate

`estimateJektMessage` and the `jekt_message` cases in `estimateNodeForState`
(`virtualization/renderers.ts` ~lines 112, 180, 242) estimate an expanded jekt from its text length, up to
`TEXT_MAX_ESTIMATE_PX`. With the cap that over-estimates a long jekt, which shifts scroll position until
the row is measured. Clamp the estimate to `JEKT_EXPANDED_MAX_ESTIMATE_PX` = 290 px: about 230 px for the body (the cap on a
1400 px window) plus about 60 px for the summary and metadata lines. A short jekt keeps its natural
estimate. It only has to be close; the measured height replaces it.

### B.5 Scope

- **In:** the expanded jekt bubble's body and raw-payload `<pre>`, incoming and outgoing, at every pane
  width.
- **Out:** collapsed bubbles; the hover peek; the jekt marker text or tier logic (this is presentation
  only, no security behaviour changes); other message blocks (agent and user messages).

### B.6 Tests

- `JektBubble.test.tsx`: the expanded body element and the raw `<pre>` have the capping class. jsdom does
  not compute `vh`, so assert on the class and, if the cap is also set inline anywhere, on that. Do not
  assert pixel values.
- `renderers` tests: an expanded jekt with a very long message estimates at the clamped height plus
  chrome, not at `TEXT_MAX_ESTIMATE_PX`; a short one is unchanged.
- Live check: send a jekt of several hundred lines to a pane; confirm the body scrolls, the metadata line
  stays visible, and the wheel chains to the pane at both ends.

---

## 4. Part C: Swarm rows outline on hover instead of filling

### C.1 Today

`SPEC_SWARM_ROW_AGENT_COLOR_AND_SELECT_TO_FOCUS_2026_09_25.md` §2.2 (PR #3781) tints the whole hovered
agent card with a lightened copy of the agent's colour. In `swarm-view.scss`, `.swarm-agent-card`:

```scss
&:hover  { background: var(--swarm-agent-hover-bg, var(--highlight-bg)); }   // full block of colour
&--active {
    background: var(--highlight-bg);
    box-shadow: inset 0 0 0 1px var(--swarm-agent-active-border, var(--accent-color, #5b8dd9));
    border-radius: 0;
}
```

`AgentRow` in `swarm-view.tsx` sets both custom properties inline: `--swarm-agent-active-border` from
`computeBlockActiveBorderColor(blockMeta)` and `--swarm-agent-hover-bg` from `lightenAgentColor(...)` of
the same value. `lightenAgentColor` has no other caller.

### C.2 Change

Hover draws the **same 1 px inset border as selection**, in the colour an **unselected pane's border**
shows. No background fill on hover.

- **Which colour.** An unselected pane's border is `computeFocusRingBorderColor(false, blockMeta)`
  (`blockframe.tsx`): an explicit `frame:hue` wins (`hueToBorder`), else `frame:bordercolor`. For an agent
  that is `dimAgentColor(...)` of its full-strength colour, seeded at launch (`agent-model.ts`). This is the
  exact counterpart of the selected row's `computeBlockActiveBorderColor`, so hover and selection differ
  only in shade, and a custom "Pane Color" choice is honoured in both.
- **CSS.**
  ```scss
  &:hover:not(&--active) {
      box-shadow: inset 0 0 0 1px var(--swarm-agent-hover-border, var(--border-color));
      border-radius: 0;   // same as --active, so the outline does not change shape on selection
  }
  ```
  `:not(&--active)` is required. `.swarm-agent-card:hover` (0,2,0) outranks `.swarm-agent-card--active`
  (0,1,0), so without it hovering the selected row would swap its full-strength border for the dim one.
  The selected card keeps its border, its `--highlight-bg` fill, and its colour when hovered.
- **Wiring.** `--swarm-agent-hover-bg` and the `hoverTint` memo go away. `AgentRow` sets
  `--swarm-agent-hover-border` from `computeFocusRingBorderColor(false, blockMeta())`. Both colours are
  derived in one small pure helper, `swarm-row-colors.ts`, so they can be unit-tested without a store.
- **No colour.** A row whose block has no colour (or no `blockId`) gets `var(--border-color)`, the theme's
  ordinary border, so hover still shows an outline. It used to fall back to `--highlight-bg`.
- **Dead code.** `lightenAgentColor` and its test are deleted; nothing else uses it.

### C.3 Scope

- **In:** the agent card (`.swarm-agent-card`: the name row plus its activity summary, enclosed together).
- **Out:** sub-rows (subagents, shells, workflows, drones) and their hover styles, the toolbar buttons, and
  the select-to-focus behaviour. Only the agent card's hover changes.

### C.4 Reading of the request

"Lighted shade colour used for unselected pane borders" is read as *the colour an unselected pane's border
already uses* (the dimmed one), matching "the unlighted variation of the border" in the same message. If
the intent was instead the *lightened* colour the hover tint used, it is a one-line change to
`swarm-row-colors.ts` (call `lightenAgentColor` and keep it).

### C.5 Tests

- `swarm-row-colors.test.ts` (pure): a block with `frame:activebordercolor` and `frame:bordercolor` yields
  the full colour for selection and the dim one for hover; `frame:hue` wins in both; no colour gives
  `undefined` for both.
- Stylesheet contract, in the style of `tool-panel-height.test.ts`: the hover rule sets no `background`,
  draws the inset 1 px `box-shadow` from `--swarm-agent-hover-border`, excludes `--active`, and the
  `--swarm-agent-hover-bg` variable no longer appears anywhere.
- Live check: hover an unselected row (dim outline, no fill), hover the selected row (unchanged), and select
  a row (outline brightens, shape unchanged).

---

## 5. Open questions

**Q1. Does `--resume` survive the switch? (Part A.) Answered from the code: no, and the backend already
handles it. One live check remains.** A bind force-restarts the controller with `--resume`, and Claude's
session transcripts live inside the account's config directory
(`REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md`). After a switch the CLI points at a different
account's directory and reports `No conversation found with session ID`. `persistent/spawn.rs` (~line 464)
handles exactly this case: it clears the unreachable session id, marks the resume as failed
(`mark_resume_failed`, which puts a session-outcome notice in the transcript), and starts a fresh CLI
session. The new session receives AgentMux's own record of the conversation with its first message
(`carry_continuation`, `SPEC_DURABLE_CONVERSATION_MEMORY_2026_09_23.md` §4.4). So the pane does not go
blank and nothing is silent, but the provider-side session is a new one and the user sees a "fresh start"
row after switching. Part A therefore needs no session-handling code. It does need the live check (A.4,
last bullet) to confirm the notice appears and the next message is answered with the conversation intact.

**Q2. Should expired accounts appear, marked?** This spec keeps them out, following the existing
`valid`-only rule and its reason (A.3.2). If the user would rather see them, the price is the
false-healthy risk in `recheckAuthAfterBind`, which would have to be fixed first.

## 6. Non-goals

- Adding an account from this menu ("Add account…" / new login). The Armory and the login flow own that.
- Non-OAuth (API-key / service) accounts. `computeAccountBindCandidates` excludes them and that stays.
- Any change to jekt trust, tier, or marker handling.
- Any change to which colour a pane border uses, or to sub-row hover styles in the Swarm view.
- A user-adjustable jekt height, or an expand-to-full button. The cap matches tool previews and scrolls.

## 7. Rollout

Three independent PRs (they touch different files and none depends on another): Parts B and C first, since
they are small and have no open questions; Part A after Q1 is answered. No flags, no migration, no backend change.
