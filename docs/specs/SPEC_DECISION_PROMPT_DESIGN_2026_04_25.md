# Decision Prompt — Cohesive Design (Step-Back Doc)

**Date:** 2026-04-25
**Status:** Living design doc, sits next to
            `SPEC_DECISION_PROMPT_2026_04_24.md`
**Why this exists:** PR #556 surfaced 7+ P1 issues across four
            review rounds — each individual fix was correct in
            isolation but the component grew without a coherent
            keyboard / state model. This doc enumerates the full
            interaction matrix so the next implementation pass
            (or refactor) has a single ground-truth to check
            against.

---

## 1. Why so many bugs

The decision panel is doing five fundamentally interlocking jobs:

1. **Cross-context keyboard input** — handle Enter / Esc /
   Shift+Enter / scope-letters that may originate from the panel
   itself, the composer textarea, or any other element in the pane.
2. **Per-pane scoping** — multiple agent panes can each have
   their own pending prompt; one pane's keystroke must not
   activate another pane's panel.
3. **Per-request transient state** — high-risk armed flag, deny
   mode, scope selection, feedback text, deny error message — all
   need to reset when the head request changes.
4. **Lifecycle-driven rendering** — the panel mounts only when
   there's a pending request; some hooks (`usePaneOverlay`)
   capture state at mount time and don't reactively refresh.
5. **No-dead-end UX** — every action the user can take has to
   leave a path back to deciding the prompt, including Defer.

Each PR-556 review round caught one slice of one of these and we
patched in isolation. The patch added a new interaction the next
round caught. Hence the cascade.

This doc fixes the underlying problem: decide ONCE what the model
is, write it down, and check every change against it.

---

## 2. State model

The panel owns these reactive signals:

| Signal | Source | Reset when |
|---|---|---|
| `pending: ToolNode[]` | Parent prop (read from documentAtom) | Parent atom updates |
| `head: ToolNode \| null` | `pending[0]` | Auto via memo |
| `request: PermissionRequestEvent \| null` | `head?.pendingPermission` | Auto via memo |
| `scope` | Local `createSignal` | **Reset on `request.request_id` change** (via createEffect) |
| `denyMode: boolean` | Local | Reset on request change AND on successful dispatch |
| `feedback: string` | Local | Reset on request change AND on successful dispatch |
| `denyError: string \| null` | Local | Reset on textarea input AND on request change |
| `highRiskArmed: boolean` | Local + 500ms timer | Reset on request change AND on successful Allow AND on timer fire |
| `minimized: boolean` | Local | **Reset on request change** so a fresh prompt always re-expands |

**Invariant**: every per-request transient signal MUST reset when
`request.request_id` changes. There's exactly one place to do
this — a single `createEffect` reading `request_id` and clearing
all of: `scope`, `denyMode`, `feedback`, `denyError`,
`highRiskArmed`, `minimized`. Do NOT spread the resets across
multiple effects.

## 3. Keyboard model

### 3.1 Listener

ONE global capture-phase `keydown` listener installed via
`createEffect` while `request()` is non-null. Cleaned up on
request becoming null and on component unmount.

There is no `onKeyDown` on any DOM element rendered by this
panel. Mixing the two duplicated handling and broke the high-risk
gate (PR #556 round 2). Single source of truth.

### 3.2 Routing predicates

| Predicate | Definition |
|---|---|
| `inOwnPane` | `paneRoot.contains(target)` where `paneRoot = rootRef.closest('.agent-view')` |
| `inPanel` | `rootRef.contains(target)` |
| `editable` | target is `INPUT` / `TEXTAREA` / `[contenteditable]` |
| `inFeedback` | `inPanel && target.tagName === "TEXTAREA"` |

**Hard gate (early-return):** if `!inOwnPane`, ignore the event
entirely. This blocks cross-pane leakage.

### 3.3 Per-key behaviour

Within `inOwnPane`, each key maps as follows. The same `editable`
guard applies symmetrically to Enter, Esc, and the scope letters
— they all defer to the composer when the user is typing.

| Key | When `inFeedback` | When `inPanel && !inFeedback` | When `inOwnPane && !editable` | When `inOwnPane && editable && !inPanel` |
|---|---|---|---|---|
| `Enter` | newline (default) | Allow / arm / deny | Allow / arm | (no-op — composer handles) |
| `Shift+Enter` | dispatch deny | enter deny mode | enter deny mode | (no-op — composer handles) |
| `Esc` | minimize | minimize | minimize | (no-op — composer handles) |
| `o` / `s` / `p` / `g` | typed into textarea | set scope | set scope | (no-op — composer handles) |
| any other | typed into textarea | (default) | (default) | (default) |

`inFeedback` rule for Enter is the one exception that diverges
from `inPanel` general behaviour: Enter inside the feedback
textarea inserts a newline (browser default) so users can write
multi-line denials.

## 4. Lifecycle

### 4.1 Mounting

The panel root (or its minimized variant) is wrapped in
`<Show when={request()}>`. The hook `usePaneOverlay` MUST be
called from a child component mounted inside that `<Show>`, not
from the outer component. Otherwise `onMount` runs before
`rootRef` is attached and the airspace clip never registers.

Pattern: a tiny null-rendering `DecisionPanelClip` child that
calls `usePaneOverlay(p.getEl)` and is mounted inside the
`<Show>`. Same pattern as modal-v2's `ModalPaneOverlayClip`.

### 4.2 Request transitions

| Transition | Effect |
|---|---|
| `null → R1` | All transient state resets (already at defaults). Panel mounts. Airspace clip registers. |
| `R1 → R2` (head changes) | `createEffect` on `request_id` resets all transient state. Panel re-renders with R2's data. Airspace re-evaluates on next resize (acceptable — same screen rect). |
| `R1 → null` (resolved) | Panel unmounts via `<Show>`. Airspace clip cleaned up by `usePaneOverlay`'s `onCleanup`. All listeners detach. |
| Defer | Sets `minimized = true` (local). Parent is informed via `onDefer` prop for logging only — parent does NOT remove the request from `pending`. |
| Allow / Deny | Panel calls `onDecide`. Parent is responsible for transitioning the ToolNode out of `pending_approval`. New `request()` is null (or next pending), `<Show>` reacts. |

### 4.3 Minimized state

Defer / Esc collapses the panel into a compact button anchored at
the same location: `Decision pending — <tool> [click to decide]`.
- The minimized button still calls `usePaneOverlay` so its
  smaller rect is the airspace cut while collapsed.
- Clicking expands.
- A new request (request_id change) auto-expands via the §2
  reset rule.
- The keyboard listener stays armed while minimized — Enter from
  outside any editable still acts on the (minimized) head request.
  This is intentional: "decision is still pending" is the user's
  responsibility, and the state is visible.

## 5. High-risk Allow

### 5.1 Two-step gate

When `risk === "high"`, the first Allow trigger only **arms**
the gate (sets `highRiskArmed = true`) for 500ms. Within that
window, a second Allow trigger commits the decision.

### 5.2 What counts as "Allow trigger"

- Click the Allow button (without Shift)
- Press Enter when `inPanel || (inOwnPane && !editable)`

### 5.3 Bypass paths

A single keystroke commits when:
- `risk !== "high"` (no gate to begin with)
- The trigger event has `shiftKey === true` (deliberate shift-click)

### 5.4 Reset rules

`highRiskArmed` resets to false:
- On request change (§2 — common reset)
- On the 500ms timer firing
- On any successful Allow dispatch

The §2 reset closes the cross-request armed leak: arming
prompt A then advancing to prompt B within 500ms cannot
auto-commit B.

## 6. Multi-pane

Each `AgentDecisionPanel` instance:
- Has its own `rootRef` and own `paneRoot` (resolved via
  `closest('.agent-view')`)
- Has its own keyboard listener
- Hard-gates by `paneRoot.contains(target)`

When two panes both have pending prompts:
- Each panel's listener only fires for keystrokes in its own pane
- The user must click into a pane (or have focus there from
  composer) to operate that pane's prompt
- Two prompts = two visible panels (or minimized buttons), each
  scoped to its own pane

## 7. Parent contract (`agent-view.tsx`)

The parent must:
- Pass `pending: () => ToolNode[]` — the live array of pending
  ToolNodes from `documentAtom`, oldest first.
- Pass `onDecide(decision)` — handle the side effect of resolving
  the ToolNode (set status, write to backend in PR-3).
- Pass `onDefer()` — optional. Used for logging / audit only.
  The parent must NOT filter out deferred requests — the panel
  manages its own minimized state.

The parent must NOT:
- Maintain a "deferred" set. (PR #556 round 4 P1 — the panel owns
  this now.)
- Mount more than one `AgentDecisionPanel` per pane.
- Pass a custom keyboard handler. The panel installs its own.

## 8. Validation checklist

When changing the panel, verify each:

**State**

- [ ] All seven transient signals reset on a single
  `createEffect(request_id)` and nowhere else.
- [ ] High-risk armed flag does not survive a request change.

**Keyboard**

- [ ] Exactly one keyboard handler exists; no `onKeyDown`
  attribute on any panel-owned element.
- [ ] All key branches early-return on `!inOwnPane`.
- [ ] All key branches respect the `!editable || inPanel`
  guard (Enter, Esc, scope letters).

**Click handlers (parity with keyboard equivalents)**

- [ ] Defer button click is symmetric with Esc — both call
  `setMinimized(true)` and `props.onDefer?.()`. (Reagent
  round-5 caught the click branch missing the minimize call.)
- [ ] Allow button click is symmetric with Enter — both honour
  the high-risk gate (shift modifier or armed-state).
- [ ] Deny button click is symmetric with Shift+Enter and
  the deny-mode Enter dispatch.

**Lifecycle**

- [ ] `usePaneOverlay` is called from a child mounted INSIDE
  the `<Show>`, not from the outer component.
- [ ] Defer leaves the prompt reachable: minimized button
  still renders, click expands, new request auto-expands.
- [ ] Empty deny + non-`once` scope shows a visible error,
  doesn't silently no-op.

**Multi-pane / multi-instance**

- [ ] Two open panels (different panes) do not cross-trigger
  on a single keystroke.
- [ ] DOM identifiers that natively imply mutual exclusion
  (radio `name`, `<datalist>` ids, etc.) are per-instance via
  `createUniqueId()`. (Reagent round-5: shared
  `name="agent-decision-scope"` caused multi-pane scope
  selection to desync.)
- [ ] Two panels with different head requests show
  independent state — selecting scope in one does NOT change
  the rendered selection in the other.

## 9. Rollback path

If the cohesive refactor doesn't land cleanly, the safest
rollback for PR-556 is: revert the panel JSX/SCSS/agent-view
wiring; keep the type plumbing from PR #555 in main. PR-3
implementation can then build the panel from scratch against
this doc rather than against the patched-PR-556 history.

## 10. Cross-references

- `docs/specs/SPEC_DECISION_PROMPT_2026_04_24.md` — feature spec
- `docs/specs/SPEC_MODAL_PANE_CLIP_2026_04_24.md` —
  `usePaneOverlay` pattern
- `frontend/app/element/modal-v2.tsx` — `ModalPaneOverlayClip`
  reference implementation of the §4.1 mounting pattern
- agentmuxai/agentmux#551 — issue this implements
- agentmuxai/agentmux#556 — the PR whose review history
  motivated this doc
