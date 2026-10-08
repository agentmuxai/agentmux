# Report: jekt messages collapse like tool calls and skid like tool previews, and one shared preview box

**Date:** 2026-10-07
**Author:** agent3 (Agent3@narko), at the operator's request
**Status:** active — built in this report's PR. D1–D3 were taken on the recommendations (operator, 2026-10-07: "take all opportunities to DRY it out"). Beyond §2, the same pass also:
- **renamed the hold** `expandedTools` to `heldOpenNodes`, with its handlers (`onHoldNodeOpen`, `onReleaseNodeHold`, `releaseScrolledOffHolds`), now that it holds jekts as well as tools;
- **shared the arrival check:** `arrivedLive()` (`virtualization/live-arrival.ts`) replaces `ToolBlock`'s private window check, and the jekt uses it;
- **shared the toggle:** the three message rows (agent message, jekt, context delivery) take their open state and click from the shared rule in `DocumentRow`, through one `flipRow()` that the `e` key also uses, instead of each reading a set directly;
- **removed a copy:** `AgentLaunchModal`'s private `formatRelative`, an exact copy of `util/format-time.ts`'s `formatTimeAgo`.

Looked at and left as they are: `JektBubble`'s `formatHeldFor` ("12 min", "3 h 5 min") is a different style from the shared `formatElapsedCompact` ("12m 0s", no hours), so merging them would change what users see.

The operator, 2026-10-07:

1. Jekt messages should collapse just like the other tool calls.
2. An expanded jekt should have the same wheel "skid" as a tool preview, including when it doesn't scroll.
3. This may warrant a common component, so reassess the duplication (DRY).

Both 1 and 2 reverse an earlier decision, deliberately. This report names each one.

## 1. What happens today

### 1.1 Jekts are open by default; tool calls are closed

Every row's open/closed state comes from one rule, `rowDisclosure()` in `frontend/app/view/agent/virtualization/disclosure.ts` (#3906, `SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md`):

| Row | Default | Toggle | State |
|---|---|---|---|
| tool (panel tools: Bash, Read, Edit, …) | **closed**. Open while running or awaiting approval; held open after finishing live on screen, until it scrolls off the top | pin | `pinnedNodes`, `expandedTools` |
| `agent_message`, **`jekt_message`** | **open** ("A message must be visible by default, not opt-in") | collapse | `collapsedNodes` |
| `context_delivery` | closed when it has text to show | pin | `pinnedNodes` |

So a jekt arrives fully expanded: body, metadata line, raw payload. That's what the operator wants changed.

- **Renderer.** `JektBubble.tsx` is built on `CollapsibleMessage.tsx`, the shared shell: chevron, summary line, body when open, time/token peek. `AgentMessageBlock` and `ContextDeliveryCard` use the same shell. `DocumentRow.tsx:251` passes it `collapsedNodes` and `onToggleCollapse`.
- **Stale comment.** `JektBubble.tsx`'s header comment still says "Collapsed by default, like AgentMessageBlock". That stopped being true with #3906.

### 1.2 The skid exists, but jekt boxes that fit skip it

The skid is `frontend/app/view/agent/components/scroll-handoff.ts` (`SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md`). When the wheel reaches a capped box's edge, the next `SKID_NOTCHES` (3) notches are absorbed and a 2px accent line shows on that edge; only then does the pane scroll. It has one option, `skidWhenFits`. A box whose content fits has no scrollbar, and without that option it hands every notch straight to the pane.

That spec's decision 4 (operator, 2026-10-04/05) was: "Tool previews without a scrollbar skid too … Jekt and context-delivery boxes that fit keep handing notches straight on." So a short jekt under the pointer doesn't hold the wheel, while a short tool preview does. That's the gap the operator noticed.

### 1.3 Every capped preview box is hand-rolled

| Box | Cap + `overscroll-behavior: contain` | Hand-off | `skidWhenFits` |
|---|---|---|---|
| Tool preview log (`ToolOverlayLog.tsx`, `.agent-tool-overlay-log`) | `_tool-overlay-portal.scss`, inside `.agent-tool-panel`'s `$transcript-preview-max-height` | `attachScrollHandoff(scrollRef, { skidWhenFits: true })` in `onMount` | yes |
| Jekt body (`JektBubble.tsx`, `.agent-jekt-body`) | `_document-nodes.scss:1245` | local `handoff` ref callback | **no** |
| Jekt raw payload (`.agent-jekt-raw pre`) | `_document-nodes.scss:1276` | the same local `handoff` | **no** |
| Context-delivery body (`ContextDeliveryCard.tsx`) | `_document-nodes.scss:1413` | its own identical local `handoff` | **no** |
| `.agent-tool-agent-result` | `_document-nodes.scss:705`: 200px, `contain` | none | — |

Each box repeats three things that must agree: the height cap, `overscroll-behavior: contain` (which the helper requires), and the `attachScrollHandoff` call.
- `JektBubble` and `ContextDeliveryCard` each carry an identical five-line `handoff` helper.
- Whether a box skids when it fits is decided per call site, which is how the jekt ended up different.
- `.agent-tool-agent-result` is dead CSS: no component renders that class any more.

## 2. Design

### 2.1 Jekts are closed by default (request 1)

- **Disclosure.** In `rowDisclosure()`, `jekt_message` moves from `collapsible` (open; the user collapses) to `pinnable` (closed; the user pins it open), as tool calls and context-delivery cards are. The virtualizer, the estimator and the `e` key all read this rule, so this one case changes the jekt everywhere.
- **Wiring.** In `DocumentRow.tsx`, `JektBubble` reads `pinnedNodes` and toggles with `onTogglePin`, as `ContextDeliveryCard` does. `JektBubble`'s `collapsed` prop becomes `!pinned`. No `CollapsibleMessage` change is needed.
- **Collapsed row.** It keeps what it shows today, so a jekt is still told apart from a typed message at a glance (`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §3.3, goals G1/G2): direction icon, From/To peer, tier badge, delivery badge, chevron, and the time/token peek on hover.
- **Old state.** Ids a user collapsed under the old rule stay in `collapsedNodes`, and the new rule ignores them. Nothing is migrated.
- **Two refinements**, D1 and D2 below.

### 2.2 Jekt boxes skid when they fit (request 2)

The jekt body and raw-payload boxes skid even when their content fits, like a tool preview. Through §2.3 this is the shared box's default, so there's no per-call-site flag to forget. This reverses decision 4 of the skid spec for jekts; D3 asks whether context-delivery boxes follow too.

### 2.3 One preview box (request 3)

**What's worth sharing.** The *capped preview box* is: cap, `contain`, scroll hand-off, skid policy, and the edge-line classes. Four call sites hand-roll it today.

**Proposal:**
- **SCSS:** one mixin, `transcript-preview-box`, in `_document-nodes.scss`: `max-height: $transcript-preview-max-height`, `overflow-y: auto`, `overscroll-behavior: contain`. Jekt body, jekt raw and context-delivery body use it. The tool log keeps its own layout (it fills a flex panel), but takes the same cap variable it already uses.
- **TS:** one ref helper in `scroll-handoff.ts`, `previewBox(opts?)`, which returns a ref callback that attaches the hand-off and registers its cleanup. It skids when the box fits by default (`skidWhenFits: true`). `ToolOverlayLog`, `JektBubble` and `ContextDeliveryCard` call it, and the two local `handoff` copies go.
  - Not a wrapper component: the boxes are different elements (`<pre>`, `<div>`) with their own classes, and a ref helper keeps their markup as it is.
  - `attachScrollHandoff` stays as the low-level call its tests already cover.
- Delete the dead `.agent-tool-agent-result` rule.

**What's not worth merging: the row shells.**
- `ToolBlock` and `CollapsibleMessage` look alike (a header row that toggles a body) but do different jobs. `ToolBlock` has the live tail, the held-open hold, the always-mounted panel with its collapse transition, the result pill and the content-first mode. `CollapsibleMessage` is a plain summary/body toggle.
- Merging them would push tool-only behaviour into messages, or the reverse.
- The rule they must share, *when is a row open*, is already one function (`rowDisclosure`). That's where request 1 lands.

## 3. Decisions

- **D1. A jekt that arrives live.**
  - **Recommended:** a tool call that finishes live on screen stays open until it scrolls off the top (`expandedTools`). Give a live-arriving jekt the same hold: it shows open when it arrives, and collapses once read and scrolled past. Loaded history opens closed. `JektBubble` would call the same `onHoldOpen` `ToolBlock` uses, when a jekt row mounts within a few seconds of its timestamp (a live arrival).
  - **Alternative:** always closed, even on arrival.
- **D2. Sensitive jekts.**
  - **Recommended:** a `TIER=sensitive` jekt stays open by default, so a message the jekt rules may require the operator to look at (`ESCALATE=required`) isn't folded away. The node carries `tier` but not `ESCALATE`, so the exception keys on the tier.
  - **Alternative:** sensitive jekts collapse like the rest.
- **D3. Context-delivery boxes.**
  - **Recommended:** they skid when they fit too, so every capped box in the conversation behaves one way, which the shared box (§2.3) makes the default.
  - **Alternative:** keep context delivery on the old behaviour, by passing `skidWhenFits: false` to the helper there.

## 4. Tests

- `disclosure.test.ts`:
  - a `jekt_message` is closed by default, and pinning opens it;
  - a sensitive one is open (D2);
  - `disclosure.parity.test.tsx`, which pins that the components, the virtualizer and the estimator agree, gets the jekt's new default.
- `JektBubble` / `DocumentRow`:
  - a jekt renders collapsed with its badges, and a click pins it open;
  - D1's hold, if taken: open on a live arrival, closed once scrolled off.
- `scroll-handoff.test.ts`: `previewBox()` skids when the box fits by default, and honours `skidWhenFits: false`.
- `ToolOverlayLog`, `JektBubble` and `ContextDeliveryCard` tests: their boxes carry `scroll-handoff-box`.
- Feel check in an isolated `task dev`: a short jekt under the pointer holds the wheel for three notches, with the edge line, before the pane moves.

## 5. Delivery

- **One agentmux PR**, frontend only, with a changeset.
- **Docs:** this report, plus a dated amendment line on each spec whose decision it reverses: `SPEC_AGENT_PANE_ROW_DISCLOSURE_2026_09_26.md` (jekt default) and `SPEC_TOOL_PREVIEW_WHEEL_EDGE_SKID_2026_09_27.md` §8 decision 4.
- **Public docs:** none. No agentmux-docs page describes whether a jekt opens or closes by default.
