# SPEC — Agent Pane Follow-ups (post-consolidation)

**Date:** 2026-04-13
**Status:** Draft
**Owner:** AgentA
**Branches / PRs:** TBD

Four small UX fixes surfaced during real use of the consolidated agent pane (after 0.33.130). Each is a standalone PR so they can land independently.

---

## 1. Scroll-to-bottom on send: user's own message is hidden

### Observation

User types into the composer and presses Enter. Their message is appended to the document as a `user_message` node, but the viewport doesn't move — so the message the user just sent is often off-screen at the bottom, and they only see it once the agent starts streaming a response (which triggers the next auto-scroll).

### Root cause

`AgentFooter` currently only calls `onTyping` on `input` events, which RAF-debounces a scroll-to-bottom via `AgentDocumentView.jumpToBottom`. On Enter, it calls `onSendMessage(message)` but does **not** fire `onTyping`, so there's no explicit scroll trigger after send.

`AgentDocumentView`'s auto-scroll effect *should* fire when the document length changes (the new `user_message` node), but it's gated on `autoScroll`, which is set to `false` any time the user manually scrolls away from the bottom. Between the typing jump-to-bottom (which forces `autoScroll = true`) and the actual send, a `scroll` event can re-run `handleScroll` and flip `autoScroll` off if the composer grew and briefly moved the scroll position.

Either way, by the time the `user_message` node is committed to the DOM, auto-scroll may be off, so the effect's scroll-to-bottom is skipped. Result: the message is hidden below the fold.

### Fix

Explicitly scroll to bottom **after** `sendMessage` has mutated the document. The append is synchronous inside `useAgentCommands.sendMessage`, but the DOM node for the new `user_message` is created in the next render tick. So the correct sequence is:

1. Append the `user_message` node (already happens in `useAgentCommands.sendMessage`).
2. Fire the RPC to the backend (unchanged).
3. **On the next animation frame**, call `scrollToBottomFn?.()` — which is the same `jumpToBottom` that `onTyping` already uses (unconditional, forces `autoScroll = true` again).

### Implementation

- **`useAgentCommands.sendMessage`** takes a new optional `onSent?: () => void` callback and calls it after the document mutation, wrapped in a `requestAnimationFrame`.
- **`agent-view.tsx`** passes `() => scrollToBottomFn?.()` as the `onSent` callback when constructing the hook.
- `AgentFooter` is unchanged — it just calls `onSendMessage` and clears the textarea as before.

Alternative considered: fire `onSent` synchronously inside `sendMessage` before the RPC. Rejected because the DOM node for the new `user_message` isn't mounted yet in the same tick — the scroll math would land above the new message. A single RAF defers to after the SolidJS render pass.

### Risk

Very low. One extra RAF per send, scrolls only if `scrollToBottomFn` is captured (which it always is for the presentation view).

---

## 2. Remove stale "+ New agent in Forge" footer button

### Observation

The agent picker has a disabled placeholder button at the bottom:

```
[ + New agent in Forge ]
```

It's inside `.agent-picker-footer` in `AgentPicker.tsx` and has been disabled forever. The empty-state fallback (when no agents are configured) has the same dead button. With the consolidation work, creating a new agent is now done via the `+ New agent` tile that sits *in the card list* and opens the inline settings panel in create mode. The footer button is pure noise.

### Fix

- Delete the `.agent-picker-footer` wrapper from `AgentPicker.tsx` (both in the success path and the empty-state fallback).
- Delete the corresponding `.agent-picker-forge-btn` SCSS rules from `agent-view.scss` if they're not referenced elsewhere.
- The empty-state UI still works — the existing `+ New agent` tile (PR #363) handles the create path from any state.

### Risk

None. Dead UI.

---

## 3. Auto-trigger login flow when the agent isn't authenticated

### Observation

First time a user opens an agent pane and the CLI reports "not logged in," the current flow:

1. `startLaunchFlow` runs the auth check.
2. CLI returns `auth_failed`.
3. `useAgentControllerStatus` sets `canRetry(true)`.
4. The UI shows a **Retry Login** button.
5. User has to notice the button, click it.

That's two clicks (open the pane, click Retry) to reach the thing the user obviously wants (logging in so the agent works). The retry button itself doesn't actually trigger the OAuth browser flow — it re-runs the `startLaunchFlow`, which re-runs the auth check, which fails again, and so on.

### Desired behavior

On first open, if the auth check fails, automatically run the provider's `/login` command (the same thing the `/login` slash command invokes from `useAgentCommands.runLoginCommand`). That command opens the OAuth browser, captures the returned URL into `setAuthUrl`, and transitions the pane into the `loginWaiting` state that's already rendered by the existing UI.

### Fix

Inside `runLaunchFlow`'s auth-check phase (launch-flow.ts), when the check reports unauthenticated, instead of returning `"auth_failed"` and letting the caller decide, run the provider's login command directly and then poll the check-auth command with the existing 2s cadence / 5-minute timeout loop. This is **already** what the flow does for some providers via the `login` step — verify it actually runs, and if not, wire it in.

Specifically:
- **`launch-flow.ts`**, Phase 2 (auth check): if the check-auth exit code / stdout indicates unauthenticated AND the provider has an `authLoginCommand` defined, automatically invoke it via `getApi().runCliLogin(cliPath, prov.authLoginCommand, authEnv)` — same call the manual `/login` slash command uses.
- If the login command emits an OAuth URL, push it into `setAuthUrl` so the existing auth UI banner shows it.
- Poll check-auth every 2s until it reports authenticated, cancelled, or 5 min elapses.
- On success, continue to Phase 3 (controller registration).
- On failure/timeout, fall back to the existing `auth_failed` path so the user still sees the Retry Login button — the manual path remains as a backstop.

### Constraint

**Don't auto-trigger during resume.** If the block already has `agentReady = true` from a prior session (i.e. the pane was reopened after a successful launch), skip the auto-login. Only run on first launch per pane. The `agentReady` signal in `useAgentControllerStatus` is the right gate — `startLaunchFlow` already early-returns if `flowRunning`; add a similar check for `agentReady` to skip the flow entirely when the pane is coming back from hydration.

Actually simpler: the flow already handles "already authenticated" correctly — the check-auth command returns success on a cached credential, Phase 2 short-circuits, Phase 3 runs, done. No extra gate needed. The auto-login triggers only when the check actually says "no cached credential".

### Risk

Medium. The OAuth browser flow has historically been finicky — see `docs/AGENT_AUTH_STATE_MACHINES.md` and PR #357. Auto-triggering it means one bug there becomes one wrong auto-click for every user. Mitigations:
- **Keep the manual Retry Login button as a backstop** for when auto-login fails or times out.
- **Log each phase transition** to the launch-logs sink, so users can see what happened.
- **Only auto-login for providers that have `authLoginCommand` defined** — providers without it fall back to the old flow.

### Implementation notes

- `runLaunchFlow` already accepts a `setAuthUrl` callback and a `setLoginWaiting` callback. All the plumbing is in place — this is about *when* the login kicks off, not *how*.
- The existing `cancelLogin` path must still work during the auto-login phase so the user can bail out.

---

## 4. Tool blocks "running" state takes two lines — should be one

### Observation

Tool blocks have a collapsed-by-default state that the user explicitly asked to be strictly one line (see PR #346 and the existing feedback memory in `~/.claude/projects/C--Systems/memory/feedback_agent_pane_tool_display.md`). When a tool is in the `running` status, it currently takes two lines — icon + name + status on the first line, and the tool parameter summary on the second. At rest (status `success` / `failed`), hover expansion collapses it correctly.

### Root cause

`ToolBlock.tsx` has a branch that renders `running` tools with an always-expanded parameter row so the user can see what the tool is actively doing. This predates the one-line rule. The parameter row is what breaks the height invariant.

### Fix

Collapse the running-state representation to a single line. Options:

1. **Append a spinner + truncated params to the summary line.** The summary already has the tool icon and name; add a spinner glyph and the first ~40 chars of the param text, ellipsized. Hover expansion still reveals the full params + any output.
2. **Replace the param row with a subtle progress indicator.** A left-edge pulse or bottom border that animates while `running`, no extra height.

Go with option 1 — it keeps the information density the user expects (you can see WHAT is running) while honoring the one-line rule.

### Implementation

- **`ToolBlock.tsx`**: in the `running` branch, remove the separate param row. Extend the summary text with a short truncation of the tool's `params` object — roughly what the current `nodeSearchText` helper produces for a `tool` node, clamped to ~40 chars.
- Add a spinner glyph (`⏳` is already used as `STATUS_ICONS.running` in `types.ts`) to the left of the status — it's already there, just make sure it's on the summary line not a separate row.
- **Hover expansion** still shows the full param object + in-flight output, so nothing is lost.
- **Verify the CSS** — `.agent-tool-block` may have a min-height rule that assumes a two-line layout for running tools; relax to one line.

### Risk

Low. Localised change in one component. The existing hover-to-expand + click-to-pin UX is untouched.

---

---

## 5. Errors collapse to one line by default

### Observation

Error content in the pane currently sprawls across many lines:

- **Launch log errors** (tag `[error]` emitted via `log("error", ...)` into the terminal-style log area) render full stack traces or long error messages, each on a wrapped line in `.agent-status-log` / `.agent-status-line--error`.
- **Failed tool blocks** show the error payload in an expanded body (`.exit-error` / `.has-error` in the SCSS) by default, not collapsed.
- **Agent-reported errors** inside text blocks wrap naturally into as many lines as the message takes.

This violates the same one-line rule tool blocks already follow (PR #346 / the user's feedback memory). The pane feels noisy when anything goes wrong.

### Fix

Every error should render as a **single line by default**, with the same hover-expand / click-to-pin affordances that tool blocks use. Specifically:

**Launch log errors** (`LogLine` with `level === "error"`):

- Collapse the error text to a single line, truncated to the pane width with ellipsis.
- Add a 1-line `.agent-status-line--error--collapsed` variant that sets `white-space: nowrap; overflow: hidden; text-overflow: ellipsis; max-width: 100%`.
- On hover, reveal the full text — either by switching `white-space: pre-wrap` or by rendering a floating tooltip with the full message.
- Click to pin open (same pattern as tool blocks).

**Failed tool blocks:**

- Already collapse by default per the existing one-line rule — verify and fix if status `failed` currently bypasses the collapsed-by-default path.
- The error payload is shown inside the expanded body on hover; the collapsed summary line already includes the ✗ status icon and tool name.
- If `ToolBlock.tsx` has special-case always-expanded logic for `failed` (mirroring the `running` bug in item #4 above), remove it.

**Agent-reported errors inside text blocks:**

- Out of scope for this pass — agent output is markdown and wrapping is intentional. Only in-pane status/log/tool errors get the one-line treatment.

### Implementation

- **`AgentDocumentView.tsx`** — the `.agent-status-line` rendering loop: for `level === "error"` lines, apply a collapsed variant class and wire hover/click handlers that toggle an "expanded" set (local signal in the component, keyed by line index).
- **`agent-view.scss`** — add the `.agent-status-line--error--collapsed` rule (`white-space: nowrap; overflow: hidden; text-overflow: ellipsis`) and an `--expanded` variant that unsets the clamp.
- **`ToolBlock.tsx`** — audit the `failed` branch. If it renders the error body always-expanded, route it through the same collapsed-by-default path used by `success` (parallel fix to item #4 for `running`).
- **Persistence** — error expansion state is session-local, not stored in block meta. Same as tool pin state in the current design.

### Alternative considered

Show errors as a 1-line summary with a ⚠ icon and a **count** when multiple errors arrive in a burst (e.g. "3 errors — hover to view"). Rejected — adds aggregation UX that doesn't exist today and the user explicitly asked for "same 1-line scheme as the others", not a new scheme.

### Risk

Low. Localised to three files. The only semantic question is whether click-to-pin is needed for error log lines or if hover-to-expand is enough — start with hover-only and add pin if users ask for it.

---

## 6. Tool hover-expand is broken — shows only the hover highlight, no expanded body

### Observation

User hovers over a (collapsed) tool block and the hover style fires (border/background changes) but the expanded content never appears. Expanded content should show the full params + result inline or as an overlay.

This is a regression relative to PR #346, which explicitly implemented hover-to-expand + click-to-pin semantics in `ToolBlock.tsx`. The docblock at `ToolBlock.tsx:5-29` still describes the intended behavior.

### Root cause (hypotheses, to verify before implementing)

`ToolBlock.tsx:73-99` implements the hover expansion like this:

```ts
const [hovered, setHovered] = createSignal(false);
const forceExpanded = () => props.node.status === "failed" || props.node.status === "running";
const expanded = () => props.pinned || hovered() || forceExpanded();
const overlayMode = () => (hovered() || props.pinned) && !forceExpanded();
```

And `<Show when={expanded()}>` wraps the body. So the content IS computed. Likely failure modes:

1. **`setHovered` never fires.** The `onMouseEnter` / `onMouseLeave` handlers were dropped during PR #348 ("tool hover overlay + scroll-on-type") which reorganized the overlay rendering. Grep for `setHovered` — if it's not called anywhere, the signal stays `false` and `hovered()` never toggles.
2. **Overlay positioning is wrong so the expanded body mounts off-screen.** PR #348 introduced absolute-positioned overlay mode (`overlayMode()`). If `position.top` / `position.left` are computed from `blockRect` but the measurements fire before the first render, the overlay lands at `(0, 0)` or negative coordinates and is clipped by `overflow: hidden` on an ancestor.
3. **`<Show>` is evaluated inside a `classList` context, not a JSX child.** Unlikely — line 234 shows `<Show when={expanded()}>` is a JSX child.
4. **Pointer-events gap**: the overlay wrapper has `pointer-events: none` and the expanded content has nothing to receive the hover, so as soon as the mouse moves into the gap between the collapsed row and the overlay, `onMouseLeave` fires, the state flips back to collapsed, and you never see the body.

My guess is **#1 or #4**. Need to read the current `ToolBlock.tsx` fully and trace the event handlers.

### Fix

1. **Verify `setHovered` is wired to `onMouseEnter` / `onMouseLeave` on the outer wrapper.** If missing, add them. If they fire on a child element that the mouse can leave en route to the overlay, hoist them to the wrapper that encompasses both the collapsed row and the expanded area.
2. **Ensure the expanded overlay is hover-sticky.** The simplest hover-sticky pattern: the wrapper has `onMouseLeave` with a small (50–100 ms) delay, or the collapsed row and the overlay share a single wrapper that scopes the hover.
3. **Log or temporarily render `hovered()` as a debug border color** during development to confirm the signal is toggling.
4. If #4 is the cause, give the overlay `pointer-events: auto` so moving the mouse into the overlay keeps hover active.

### Why this is separate from #4 (tool running one-line)

Item #4 is about the *running* branch rendering two lines at rest. Item #6 is about hover expansion being completely broken for ALL tool states that should support it (`success`, in particular — the at-rest default). They touch the same file but are independent bugs; fix #6 first because it blocks the hover UX entirely.

### Risk

Low-to-medium. The symptom is a regression in established behavior, so there's a known-good state to aim for. Risk is misdiagnosis — always fire up the pane and verify the signal transitions before shipping.

### Out of scope

- Redesigning the hover → pin UX — keep the current contract.
- Switching to a click-only expansion model — the user explicitly wants hover-to-expand.

---

---

## 7. Move the runtime options dropdown below the composer hint

### Observation

The `AgentControlBar` (permission mode · model · effort) currently sits **above** the document view, between the header and the message list:

```
┌ AgentPresentationHeader ─────────────────────┐
│ ⚡ claude-sonnet                         [✕] │
├──────────────────────────────────────────────┤
│ [AgentControlBar — Permission │ Model │ ...] │  ← now here
├──────────────────────────────────────────────┤
│  (document view — messages + tool blocks)   │
├──────────────────────────────────────────────┤
│  composer textarea                           │
│  Enter to send • Shift+Enter for newline     │
└──────────────────────────────────────────────┘
```

It should live **below** the composer hint line instead:

```
┌ AgentPresentationHeader ─────────────────────┐
│ ⚡ claude-sonnet                         [✕] │
├──────────────────────────────────────────────┤
│  (document view — messages + tool blocks)   │
├──────────────────────────────────────────────┤
│  composer textarea                           │
│  Enter to send • Shift+Enter for newline     │
│  [AgentControlBar — Permission │ Model │ ...] │  ← moves here
└──────────────────────────────────────────────┘
```

Rationale: the controls are **per-turn settings** (permission mode, model, effort) — always available but rarely touched. Putting them above the document eats vertical space from the messages (the thing the user actually looks at). Placing them below the composer keeps them one glance away but out of the primary reading area.

### Fix

`agent-view.tsx` composes the layout. Move the `<AgentControlBar>` element out of its current slot (after the header) and drop it into the composer region — structurally a sibling of `<AgentFooter>`, rendered immediately after it.

Simplest implementation: wrap `<AgentFooter>` and `<AgentControlBar>` together in a `.agent-composer-region` container so they're visually one unit at the bottom.

```tsx
// agent-view.tsx — approximate target
<AgentPresentationHeader ... />
<Show when={bookmarks.visible()}>
  <BookmarksPanel ... />
</Show>
<AgentSearchBar ... />
<Show when={!digest.dismissed()}>
  <SessionDigestBanner ... />
</Show>
<AgentDocumentView ... />
<Show when={status.loginWaiting()}>...</Show>
<Show when={status.canRetry()}>...</Show>

<div class="agent-composer-region">
  <AgentFooter ... />
  <AgentControlBar ... />  {/* now here, not up top */}
</div>
```

### SCSS adjustments

- `.agent-control-bar` currently has top-of-pane styling (border-bottom, maybe top margin). Flip to a top-border on the bottom variant so it reads as a sub-footer rather than a sub-header.
- The collapsible-body transitions upward into the composer area — verify `max-height` / `overflow` still behave correctly when the bar is anchored to the bottom.
- The header row with the `[ ⚙ Permission │ Model │ Effort ]` chevron-toggle pattern stays the same; only the vertical position changes.

### Risk

Low. The control bar is self-contained — it reads block meta via RPC, has its own collapsed/expanded state, doesn't reach into document view. Moving its mount point is a JSX reorder + a SCSS border flip.

One thing to verify: when `AgentControlBar` was placed above the document, expanding its body pushed the document down (flex-column natural behavior). When it's below the composer, expanding should push the composer UP (or the control body expands downward from the top-border). Both are fine; just pick one and make sure the max-height animation doesn't feel wrong.

### Out of scope

- Redesigning the controls (permission mode enum, model list, effort levels). Unchanged.
- Adding new controls. Unchanged.
- Making the control bar dockable (user can choose top/bottom). Overkill.

---

---

## 8. Remove the in-pane `AgentPresentationHeader`; use the pane frame's title instead

### Observation

Inside the agent pane we render an `AgentPresentationHeader` strip at the very top:

```
┌─────────────────────────────────────┐
│ [agent] claude-sonnet          [✕]  │  ← AgentPresentationHeader
├─────────────────────────────────────┤
│ pane header (block frame)           │  ← existing block chrome
├─────────────────────────────────────┤
```

But the block frame already has its own title area that shows the view name + icon. The in-pane header duplicates that information, eating vertical space.

### Current state

- `AgentViewModel` in `agent-model.ts` sets `viewName = () => "Agent"` and `viewIcon = () => "sparkles"` — both static. The block frame therefore displays "Agent" / sparkles regardless of which forge agent is launched.
- `AgentPresentationHeader` (44 lines, `components/AgentPresentationHeader.tsx`) renders the icon + name from `block.meta.agentName` / `agentIcon` and the `✕ Back to agents` button.
- Pane title doesn't update as the user picks an agent because `viewName` never reads the meta.

### Fix

**Drive the pane's title from the launched agent's name/icon**, and delete the in-pane header entirely.

1. **Make `viewName` / `viewIcon` reactive** to `block.meta.agentName` / `agentIcon`:
   ```ts
   // agent-model.ts
   this.viewName = () => {
       const meta = this.blockAtom()?.meta;
       return meta?.["agentName"] ?? "Agent";
   };
   this.viewIcon = () => {
       const meta = this.blockAtom()?.meta;
       return meta?.["agentIcon"] ?? "sparkles";
   };
   ```
   The `blockAtom` is already a `SignalAtom<Block>` — SolidJS will re-evaluate these accessors when the meta changes after launch.
2. **Delete `<AgentPresentationHeader>` from `agent-view.tsx`.**
3. **Delete `components/AgentPresentationHeader.tsx`** — no other callers.
4. **Delete the `.agent-pres-header` / `.agent-pres-icon` / `.agent-pres-name` / `.agent-pres-back` SCSS rules** from `agent-view.scss`.

### The `✕ Back to agents` button

The header was the only entry point for "exit this agent session and return to the picker." We lose that UI when we delete the header. Options:

1. **Block frame header icons.** `ViewModel` exposes a `viewText: () => string | HeaderElem[]` that can include interactive elements. Add a small back-arrow icon there so the pane frame shows it next to the title.
2. **Context menu on the pane header** — right-click → "Back to agents." Less discoverable; reject.
3. **Keyboard shortcut only** (e.g. `Ctrl+Shift+B`). Too hidden.
4. **Command palette action** — "Agent: Back to picker." Good as a secondary path but not as the only one.

Go with **option 1** (block frame header icon) as primary + **option 4** as a secondary affordance.

For option 1, the exact implementation depends on `HeaderElem`'s existing vocabulary — check `frontend/types/custom.d.ts` for whether it supports click-able icons, or if we need a new variant. If it doesn't, fall back to placing a small back button inside the control bar row (from item #7) as a stopgap.

### Alternative considered

**Keep the header but shrink it to a narrow breadcrumb row.** Rejected — the user's explicit guidance is "we'll be using the pane's title for that." Two sources of truth for the agent identity is the exact thing they're calling out.

### Implementation notes

- The `handleBack` action (currently in `useAgentCommands.back()` and called from the header's `onBack` prop) stays exactly the same — it still clears `agentId` + related meta keys so the view falls back to the picker via `AgentViewWrapper`. Just the caller changes.
- After removal, `agent-view.tsx`'s JSX is simpler by ~6 lines.
- Verify the pane frame rerenders its title when `meta.agentName` changes — the existing WOS atom subscription should handle it automatically. Test by launching an agent and watching the frame title flip.

### Risk

Low. Delete operation + one accessor wire-up. The one thing to verify is that the back action remains reachable via SOME UI surface (header icon OR command palette) before this PR ships — losing the `✕` without a replacement would trap the user in the agent view.

### Out of scope

- Redesigning the block frame chrome.
- Adding per-agent colors to the pane title.
- Making the pane title editable in place.

---

## 9. Estimated cost

| # | Feature | File(s) | Est. time | Review rounds |
|---:|---|---|---:|---:|
| 1 | Send scroll-to-bottom | `useAgentCommands.ts`, `agent-view.tsx` | 15 min | 0–1 |
| 2 | Remove footer button | `AgentPicker.tsx`, `agent-view.scss` | 5 min | 0 |
| 3 | Auto-login | `launch-flow.ts` | 30 min | 1–2 |
| 4 | Tool `running` one-line | `ToolBlock.tsx`, `agent-view.scss` | 15 min | 0–1 |
| 5 | Errors one-line | `AgentDocumentView.tsx`, `ToolBlock.tsx`, `agent-view.scss` | 20 min | 0–1 |
| 6 | Fix tool hover-expand regression | `ToolBlock.tsx` | 30 min | 0–1 |
| 7 | Move control bar to bottom | `agent-view.tsx`, `agent-view.scss` | 15 min | 0–1 |
| 8 | Remove in-pane header; use frame title | `agent-model.ts`, `agent-view.tsx`, `agent-view.scss` (delete) | 20 min | 0–1 |
| 9 | Esc = clear / stop agent | `AgentFooter.tsx`, `useAgentCommands.ts`, `agent-view.tsx` | 15 min | 0–1 |

Total: ~165 min of coding, 1–8 review rounds. Ship as separate PRs; **bundle #4 + #6** (both ToolBlock), and keep the rest standalone.

---

## 9. Esc key in the composer: clear if text, stop the agent if empty

### Observation

The composer currently only listens for Enter (send) and Shift-Enter (newline). There's no way to cancel a message you've started typing without selecting-all and deleting, and no way to stop an agent mid-turn without finding the right UI affordance elsewhere.

### Desired behavior

Two Esc semantics, chosen by the current textarea state:

1. **If the textarea has text:** Esc clears it. Cursor stays in the composer, ready for the next message.
2. **If the textarea is empty (or whitespace only):** Esc sends a **stop signal** to the running agent process — same as Ctrl+C would in a terminal. Claude Code supports this via a SIGINT to the CLI process.

The two behaviors are mutually exclusive per keypress, so one handler is enough.

### Fix

**`AgentFooter.tsx`** — extend `handleKeyDown`:

```ts
const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        handleSend();
        return;
    }
    if (e.key === "Escape") {
        e.preventDefault();
        if (textareaRef && textareaRef.value.trim().length > 0) {
            textareaRef.value = "";
            // Fire onTyping so the composer resets its auto-grow
            props.onTyping?.();
        } else {
            props.onStopAgent?.();
        }
    }
};
```

**`AgentFooter` props** — add `onStopAgent?: () => void`. If unset, Esc on empty is a no-op (defensive).

**`agent-view.tsx`** — wire `onStopAgent` to a new `commands.stopAgent()` (item below) so the composer doesn't reach into the model directly.

**`useAgentCommands.ts`** — add a `stopAgent()` method that sends a SIGINT to the CLI process via the existing `RpcApi.ControllerInputCommand(TabRpcClient, { blockid, signame: "SIGINT" })` path. This is already documented in the memory file for terminal I/O and used by the terminal view, so the RPC exists.

### Hint line

Update the composer hint to reflect the new binding:

```
Enter to send • Shift+Enter for newline • Esc to clear / stop
```

### Risk

Low. Esc is an obvious UX improvement. The only subtle concern: the composer is inside a pane and Esc might conflict with something at the window level (e.g. closing a modal). `e.preventDefault()` only blocks default, not propagation — if we need to stop propagation too, add `e.stopPropagation()`. Check if any ancestor listens for Esc.

### Out of scope

- Graceful stop (`/stop` slash command, soft shutdown, etc.). Esc sends SIGINT — equivalent to Ctrl+C. If the agent ignores SIGINT on certain platforms, that's a separate fix.
- Esc cancelling the OAuth login flow — already handled by the existing `cancelLogin` button.
- Visual "stopping..." indicator — out of scope; the existing subprocess-done log line covers it.

---

## 10. Ordering

**#6 first** (hover is broken; blocks all tool-expansion UX — restore it before touching anything else in ToolBlock) → **#2** (dead code, zero risk) → **#8** (remove in-pane header, atomic deletion) → **#7** (control bar move, layout-only) → **#1** (send scroll, trivial) → **#9** (Esc key handling) → **#4 + #5 bundled** (one-line rule for running tools + errors) → **#3 last** (auto-login, biggest surface area).

Reasoning: #6 is a regression gating other tool UX, fix it before anything else in that file. #7 is a pure layout reorder with no behavior change, ship it early so the test builds reflect the new layout. #3 is riskiest and ships last.

## 10. Success criteria

After all nine:

- Pressing Enter in the composer scrolls the document so the user's own message is visible, every time.
- The picker has no `+ New agent in Forge` button at the bottom. The `+ New agent` tile in the list is the only way to create one.
- Opening an agent pane for the first time (no cached credentials) opens the OAuth browser flow automatically. The user completes login in the browser; the agent pane transitions to the running state without a manual click.
- Tool blocks in `running` state are exactly one line tall, matching the existing success/failed rule. Hover still expands to show params + output.
- Error log lines and failed tool blocks render as a single line at rest. Hover reveals the full error text. The pane is calm when something goes wrong, not sprawling.
- Hovering a collapsed tool block expands it to show the full params + result (or routes through overlay mode per PR #348). Click to pin still works.
- The runtime control bar (permission mode · model · effort) sits **below** the composer hint line, not above the document view. Expanding it reveals the controls without covering the messages.
- The in-pane `AgentPresentationHeader` is gone. The block frame's title shows the launched agent's name + icon (reactive to `block.meta.agentName` / `agentIcon`). A "back to picker" action is reachable from at least one surface (block header icon or command palette).
- Pressing Esc in the composer with text clears the textarea. Pressing Esc with an empty composer sends a SIGINT to the agent CLI process. The hint line reflects both bindings.

## 11. Out of scope

- Redesigning the auth state machine — just auto-trigger the existing flow.
- Adding a progress indicator to running tools beyond the existing spinner — option 2 above is rejected.
- Touching the slash-command dispatcher (`/login`, `/clear`) — unchanged.
- Error aggregation / count badges — rejected above; keep it simple.
- Wrapping agent-generated text output (markdown) — agent text stays as-is.
- Any backend change — these are all frontend/UX fixes.
