# Report: Agent Pane — New Rows Arrive with a Jerk

**Status:** analysis — owner asked for a quick animation instead of an instant jump; recommendation in §5, not yet implemented
**Date:** 2026-10-01
**Verified against:** `8f6415525` (main)

---

## 1. The complaint

When a new row lands in the agent pane (a tool call, a reply), it appears
instantly and the conversation jerks. Fast is right; the motion is not. Asked:
is an animation set now, and what do other apps do?

## 2. What the pane does today

**There is an enter animation, but it is small.**
`frontend/app/view/agent/styles/_document.scss` (the
`.agent-document-streaming-buffer[data-animate]` rule): a new row fades from
opacity 0 to 1 and rises 4px over **120 ms**
(`cubic-bezier(0.4, 0, 0.2, 1)`), via `@starting-style`. Only rows of the turn
in flight, and not the user's own message (#4123). It is not disabled for the
owner: `window:reducedmotion` is unset, and the rule has no
`prefers-reduced-motion` gate at all (the spec,
`SPEC_AGENT_PANE_MESSAGE_ENTER_ANIMATION_2026_05_30.md` §6, says it should
snap under reduced motion).

**Why it still reads as a jerk:**

1. **Everything else jumps instantly.** The new row takes its full height in
   one frame. While pinned, the content ResizeObserver in
   `AgentDocumentVirtualList.tsx` calls `scrollToTrueBottom()`, which does
   `scrollTo({ top: MAX, behavior: "auto" })`: the whole conversation moves
   up by the row's height before the next paint. The fade covers only the
   new row; the eye follows the big movement.
2. **The fade is barely visible.** 120 ms is about seven frames, and a 4px
   rise is below what most people notice. The original spec targeted the tool
   panel's feel — geometry *and* opacity (§1) — but only opacity and the 4px
   shipped.
3. **Rows that grow after mounting also snap.** A tool row mounts as a
   one-line running row, then expands when its result lands; Edit/Write
   previews mount open. Only the result pill fades
   (`agent-tool-summary-fade-in`).

## 3. What other apps do

Source-level findings where the code is open; docs or reports otherwise.

| App | New row | Following the bottom | Evidence |
|---|---|---|---|
| **VS Code chat** | No row animation. An experimental per-block fade/rise (`translateY(4px)`, 600 ms default, staggered, off under reduced motion) | **Instant** — `revealLastItem()` → `setScrollTop()` with no animation; `workbench.list.smoothScrolling` (125 ms easeOutCubic) applies to wheel/keys only | `chat/browser/widget/chatListWidget.ts` `_withPersistedAutoScroll`; `chatIncrementalRendering.css`; `base/browser/ui/list/listView.ts` |
| **Codex CLI** | None (terminal) | Native terminal scrollback | `codex-rs/tui/src/insert_history.rs`; `streaming/chunking.rs` paces output one line per ~8 ms tick, with a catch-up mode |
| **Claude Code CLI** (fullscreen) | None documented | Auto-follow, pauses on scroll-up, "N new messages" jump button | code.claude.com/docs/en/fullscreen |
| **Claude Desktop / claude.ai / Cowork** | Unknown (closed) | Stick-to-bottom; public reports are about it breaking, not about motion | anthropics/claude-code#53382, #11578 |
| **Zed agent panel** | None | Pinned at layout time (`FollowMode::Tail`), no scroll write | `crates/agent_ui/src/conversation_view.rs`; `crates/gpui/src/elements/list.rs` |
| **use-stick-to-bottom** (StackBlitz / bolt.new) | — | **Animated**: on content growth, a per-frame spring on `scrollTop` (damping 0.7, stiffness 0.05, mass 1.25; 350 ms retain window); new growth mid-animation joins the same moving target | `src/useStickToBottom.ts` |
| **Vercel AI Elements** | — | use-stick-to-bottom with `resize="smooth"` by default | `packages/elements/src/conversation.tsx` |
| **assistant-ui** | — | Instant on resize (`scrollTo` "instant"); optional "anchor the user's turn at the top" mode | `useThreadViewportAutoScroll.ts` |
| **ChatGPT, Cursor** | Unknown | Stick-to-bottom; Cursor reports losing it when blocks expand | user/forum reports only |

**Patterns:**

1. **Most pin instantly.** VS Code, Zed, assistant-ui and the terminals all
   jump. Where they smooth anything, they **pace the content** (VS Code's
   words-per-second progressive render, Codex's line-per-tick), not the
   scroll.
2. **The one widely used library that glides — use-stick-to-bottom — is the
   default in Vercel's chat components.** It is the closest precedent for
   what the owner asked for. Its key choices: animate toward the bottom
   rather than jump; fold new growth into the running animation instead of
   restarting; stop as soon as the user scrolls or selects.
3. **VS Code has exactly our clash** — a 4px rise on new blocks under an
   instant pin — and its code avoids wrapper elements to keep layout shifts
   out of scrolling. Nobody has a better answer for it than gliding the
   follow itself.

## 4. Options

| | Change | Fixes | Risk |
|---|---|---|---|
| A | Longer, clearer row fade: ~180 ms, rise 8px; snap under `prefers-reduced-motion` and the app's reduced-motion setting | Cause 2; the reduced-motion gap | None |
| B1 | **Transform glide.** When a row is appended while pinned, keep the instant `scrollTop` write, then animate the content's `transform` from `translateY(+delta)` to 0 over ~180 ms (Web Animations, `composite: "add"` so overlapping glides sum). Cancel on user scroll, wheel or selection | Cause 1 | Low–moderate: no change to the pin logic or scroll events; the scrollbar is already at its final place during the glide; hit-testing is off by up to `delta` for ~180 ms |
| B2 | **Spring on `scrollTop`** (use-stick-to-bottom's approach), ~350 ms | Cause 1 | Moderate–high here: many programmatic scroll events per append, which the pin/user-scroll handshake (`pendingProgrammaticScroll`, `pinnedGeometry`) is built around one-per-pin; the newest content sits below the fold for the spring's duration |
| C | Animate row height growing in | Causes 1 and 3 | High: per-frame layout, fights the virtual list's measurements |
| D | Glide the growth of existing rows too (a tool result expanding) — B1 applied to any pinned growth over a threshold | Cause 3 | As B1; must skip plain streamed-text growth (one line at a time) |

## 5. Recommendation

**A + B1**, then measure, then decide on D.

- A is free and fixes the reduced-motion gap the spec already required.
- B1 gives the "content glides up" feel use-stick-to-bottom is known for,
  without touching the pin handshake that took several audits to get right
  (`REPORT_AGENT_PANE_SCROLL_PIN_FLICKER_AUDIT_2026_07_30.md`). Folding
  overlapping glides together (`composite: "add"`) is the same "retarget,
  don't restart" rule use-stick-to-bottom follows.
- Guard rails: only when pinned and a row was added; skip when `delta` is
  larger than the viewport (a history load) or in a burst; cancel on user
  input; off under reduced motion.
- If B1 fights the virtual list or the tool overlays (a transformed ancestor
  re-anchors `position: fixed` descendants for the glide's duration), fall
  back to B2.
- Verify over CDP as for the send flash (#4123): frame-by-frame capture and
  layout-shift score, plus a typing-latency check while a pane streams, before
  and after.

## 6. Sources

- VS Code: `src/vs/workbench/contrib/chat/browser/widget/chatListWidget.ts`, `…/chatContentParts/chatIncrementalRendering/media/chatIncrementalRendering.css`, `src/vs/base/browser/ui/list/listView.ts`, `src/vs/base/common/scrollable.ts`
- Codex CLI: `codex-rs/tui/src/insert_history.rs`, `codex-rs/tui/src/streaming/chunking.rs`, `codex-rs/tui/src/app.rs`
- Claude Code: https://code.claude.com/docs/en/fullscreen ; anthropics/claude-code#53382, #11578
- Zed: `crates/agent_ui/src/conversation_view.rs`, `crates/gpui/src/elements/list.rs`
- use-stick-to-bottom: https://github.com/stackblitz-labs/use-stick-to-bottom (`src/useStickToBottom.ts`)
- Vercel AI Elements: https://github.com/vercel/ai-elements (`packages/elements/src/conversation.tsx`)
- assistant-ui: `useThreadViewportAutoScroll.ts`
- Cursor: https://forum.cursor.com/t/agent-stops-scrolling-chat-buffer-automatically-after-terminal-or-code-boxes-appear-in-chat/150287
