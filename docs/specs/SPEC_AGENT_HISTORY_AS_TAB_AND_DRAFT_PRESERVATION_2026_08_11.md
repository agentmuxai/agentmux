# SPEC: Agent History as a pane tab, composer draft preservation, and a scrolling link row

**Date:** 2026-08-11
**Status:** proposed
**Severity:** Medium — UX correctness + recurring bug class
**Supersedes:** §4.2 of `SPEC_AGENT_PANE_SESSION_SCOPED_SCROLLBACK_AND_AGENT_HISTORY_VIEW_2026_08_09.md`
("View-swap mechanism") — the `bodyMode: "live" | "history"` in-place swap this
doc replaces. Everything else in that spec (§3 session-scope clamp, §4.3 data
layer reuse, §4.4 day separators/tsidx, §4.5 archives) is unaffected and this
doc builds on it directly.
**Related:** `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md` (the blockStack
tab mechanism this doc reuses), `SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md`

---

## 0. Ask

Live feedback after using the shipped P2 Agent History view (2026-08-10/11):

1. If a user has typed a draft into the composer, opens Agent History, then
   comes back — their draft is gone. It must survive the round trip.
2. The "Open Agent History" row (currently a `PaneRow`, appearing right after
   the "New Session" divider) is fixed in place above the scrollable
   transcript instead of scrolling with it. It should be a normal
   conversation node that scrolls with everything else.
3. Agent panes now have a real, working tab strip (blockStack tabs — forks,
   the "+" new-tab button). Agent History should open as **a second tab in
   the same agent pane**, not an in-place content swap. Swapping the body of
   one tab between "live" and "history" should not be possible at all —
   there should just be two tabs.
4. Add a second entry point: a right-click context-menu item **"Agent
   History"**, pinned at the top of the menu, using the existing submenu
   pattern.

---

## 1. Why the in-place swap has to go, not just get patched

`bodyMode` (`agent-view.tsx`) toggles which subtree renders in the *same*
mounted `AgentPresentationView` instance: `"live"` renders
`AgentDocumentView` + `AgentFooter`; `"history"` unmounts that and mounts
`AgentHistoryView` in its place; returning to `"live"` remounts the original
subtree from scratch.

That remount-in-place shape is the direct cause of two distinct problems
already:

- **The regression fixed just before this doc** (uncommitted at time of
  writing): the working-row-height `ResizeObserver` was wired up in a
  one-shot `onMount` that captured whichever DOM node existed at first
  mount. Returning from history mode created a *new* anchor div that the
  observer never re-attached to, so the floating "Working…"/"✓ Worked" row's
  reserved padding went stale and the row visually overlapped the last
  message. Fixed via a ref-callback that re-observes on every (re)mount —
  but the underlying hazard (this subtree can now remount at a time other
  than pane-open) remains for any *other* piece of state built on
  "this only happens once" assumptions.
- **The draft-loss bug reported here**: `AgentFooter`'s `<textarea>` is
  deliberately *uncontrolled* — "DOM owns the value" (see its own top-of-file
  comment; this was a deliberate perf choice, not an oversight — a prior
  controlled version cost ~22ms of forced layout per keystroke). An
  uncontrolled input's value lives only in that DOM node. Unmounting
  `AgentFooter` (which the `bodyMode` swap does on every trip into history
  mode) destroys it with nothing to restore from.

Both bugs are instances of the same root problem: **a subtree that used to
mount exactly once for a pane's lifetime now remounts on a user action**, and
various pieces of state (a `ResizeObserver`, an uncontrolled textarea) were
written under the "mounts once" assumption. Patching each downstream symptom
(a ref-callback fix here, a draft-snapshot-and-restore mechanism there) works
but invites a third instance of the same class next time something else
implicitly assumed single-mount. Removing the remount entirely removes the
whole bug class at once, which is what §3 below does.

---

## 2. Goals / non-goals

**Goals**

1. Agent History opens as a **second tab in the same agent pane**, using the
   existing blockStack tab mechanism (`PaneTabStrip`, `pushBlockOntoStack`,
   `setActiveBlockInStack` — see `SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md`),
   not a content swap inside one tab.
2. The live tab's `AgentPresentationView` instance is **never unmounted** by
   opening/closing/switching to the history tab. No more "subtree that
   assumed it mounts once now remounts."
3. An in-progress composer draft **always survives** switching to the history
   tab and back — whether because the live tab's DOM never went away (the
   natural outcome of #2) or, if pane-stack switching turns out to
   unmount/remount inactive members (see §6 open question), via an explicit
   per-block draft-persistence fallback. Either way this is a hard
   requirement, not "usually works."
4. The "Open Agent History" entry point renders as part of the scrolling
   transcript (a synthetic node next to the "New Session" divider it already
   sits beside), not a pinned row above the scroll region.
5. A second entry point: **"Agent History"** at the top of the pane's
   right-click context menu, via the existing `getBodyContextMenuItems` /
   `submenu` machinery — no new context-menu primitive needed.
6. Opening Agent History when a history tab for this agent is already open
   **focuses the existing tab** instead of stacking a duplicate.
7. Zero change to the data layer shipped in P2 (§4.3/§4.4 of the
   session-scoped-scrollback spec) — `AgentHistoryView`'s reader, day
   separators, and tsidx stamping are reused as-is. This doc only changes
   *how the view is reached and hosted*, not what it renders.

**Non-goals**

- Redesigning the history reader itself (day separators, session dividers,
  archives §4.5) — out of scope, already correct.
- Persisting the history tab across pane reopen/app restart — like the
  bodyMode it replaces, a history tab is a transient reading posture; closing
  the pane's live tab (or the app) does not need to reopen history tabs.
  Regular block/session persistence rules apply to it exactly as to any
  other stack tab, no special-casing.
- General composer-draft persistence across *pane close* (e.g. surviving an
  app restart) — this doc's draft-preservation requirement is scoped to the
  live↔history tab round trip within one running session.

---

## 3. Design

### 3.1 Agent History as a pane tab

Reuse the exact mechanism `AgentViewWrapper`'s "+" button already uses for a
blank new agent tab (`handleNewAgentTab`, `agent-view.tsx`), with one
difference: the new block is opened **pre-launched against this same
agent**, not blank/picker.

```ts
// New block, same agent, marked as a history reader — never shows AgentPicker.
const paneOpenResult = await TabRpcClient.rpcCall("pane.open", {
    view: "agent",
    skip_placement: true,
    meta: {
        view: "agent",
        agentId: currentAgentId,          // same definitionId — AgentViewWrapper's
                                           // `agentId()` gate passes immediately
        "agent:historyTabFor": currentAgentId, // marks this block as a read-only
                                                // history reader, not a live launch
    },
}, {});
pushBlockOntoStack(layoutModel, node.id, paneOpenResult.block_id);
```

`AgentViewWrapper` gains one early branch, before the existing
`agentId() ? <AgentPresentationView> : <AgentPicker>` gate:

```
agentId() && meta["agent:historyTabFor"]  → <AgentHistoryTabView agentId=... />  (new, thin)
agentId()                                  → <AgentPresentationView ...>          (existing, live, untouched)
else                                        → <AgentPicker ...>                   (existing)
```

`AgentHistoryTabView` is a thin wrapper around the *existing*
`AgentHistoryView` component (§4.1–4.3 of the session-scoped-scrollback
spec) — same reader, same data layer, same day separators. It drops:

- The `onClose` "← Back to conversation" affordance (closing this reading
  posture is now "close this tab," which `PaneTabStrip`'s × already does for
  every tab — no bespoke back button needed).
- Any notion of a body-mode toggle — this block *is* a history reader for
  its whole lifetime; it never flips to live. That invariant is what
  eliminates the remount bug class from §1: this component mounts once, the
  live block's component mounts once, and neither ever swaps into the other.

**Tab label:** `"History"` (or `"<agent name> · History"` if the pane already
disambiguates other tabs by name — match whatever `labelForBlock` already
does for a launched vs. picker tab, just reading the `agent:historyTabFor`
meta instead of `agentId` to pick the label).

**Idempotency (goal #6):** before opening a new tab, `combinedTabs()` (already
computed in `AgentViewWrapper`) is scanned for an existing member whose meta
has `agent:historyTabFor === currentAgentId`; if found, `handleTabSwitch` to
it instead of calling `pane.open` again. Mirrors how fork tabs are already
deduped against the stack by blockId (`combinedTabs`'s existing
`!stackIds.has(f.blockId)` filter) — same pattern, one more key to check.

### 3.2 Entry point 1 — link row as a scrolling document node

Per §3.4 of the session-scoped-scrollback spec, the "Earlier conversations /
Open Agent History" row was deliberately built as a `PaneRow` — explicitly
*not* a `DocumentNode`, "so it must not persist, not virtualize, not appear
in Agent History itself." That reasoning held for a **link that swapped the
current tab's content**; it stops holding once opening it just activates a
different tab (§3.1) — there is no reason left for it to be exempt from
virtualization, and the live feedback confirms it reads wrong pinned above
the scroll region while the "New Session" divider it's paired with scrolls
normally underneath it.

Convert it into a synthetic node alongside the existing `day_divider`
pattern (`virtualization/DocumentRow.tsx`, `renderers.ts`,
`expansion-source.ts`):

- New synthetic type, e.g. `history_link` — render-time only (like
  `day_divider`), never persisted, never appears inside `AgentHistoryView`
  itself (the reader has no need to link to itself).
- Injected immediately after the `fresh` `session_outcome` divider node
  (same insertion point the `PaneRow` occupies today), so visually nothing
  moves — it just becomes part of the normal-flow virtualized rows instead
  of a pinned accessory above `.agent-document-scroll-region`.
- Click behavior changes from `setBodyMode("history")` to the §3.1 open-or-
  focus-tab logic.
- Sizing: fixed small height like `day_divider`'s own row, registered with
  the same estimate/measure path so the virtual list's prefix-sum layout
  accounts for it correctly.

### 3.3 Entry point 2 — right-click context menu

`ContextMenuItem` already supports a `submenu` field end-to-end —
`_convertAndRegisterMenu` (`contextmenu.ts`) recursively converts
`item.submenu` into native submenu entries, and `sysinfo-model.ts`'s
`getBodyContextMenuItems()` (returning a `{ label: "Plot Type", submenu: [...] }`-
shaped entry) is a working precedent for exactly this shape. No new menu
primitive is needed — `AgentViewModel` just needs to implement the
already-declared, already-wired, currently-unused hook:

```ts
// types/custom.d.ts already declares this on the ViewModel interface;
// blockframe.tsx's body-right-click handler already calls it
// (`props.viewModel?.getBodyContextMenuItems?.()`) and splices the result
// at the TOP of the menu, before the shared pane actions + separator.
// agent-model.ts (AgentViewModel) currently doesn't implement it — new.
getBodyContextMenuItems(): ContextMenuItem[] {
    return [
        {
            label: "Agent History",
            click: () => openOrFocusHistoryTab(),  // same helper as §3.1's link row
        },
        { type: "separator" },
    ];
}
```

Since this repo's only current submenu precedent (`sysinfo-model.ts`'s "Plot
Type") is a *choice among options*, not a *single action item with children*,
and the ask here is one top-level clickable action ("Agent History" itself
opens/focuses the tab) — a plain top-level item is the right shape, not a
submenu wrapping a single child. If a future need arises (e.g. "Agent
History ▸ This session / ▸ Full history / ▸ Archives"), the `submenu` field
on this same item is a drop-in extension point — noted here so the
one-vs-many-entries decision doesn't need re-litigating later, but not
built now since there is exactly one destination today.

### 3.4 Composer draft preservation

With §3.1's redesign, the live block's `AgentPresentationView` — and
therefore `AgentFooter`'s uncontrolled `<textarea>` — is never unmounted by
opening, switching to, or closing a history tab. That alone should resolve
the reported loss, **provided** inactive blockStack tabs stay mounted
(hidden) rather than being unmounted when not the pane's `activeBlockId` —
see §6, this needs to be confirmed against the actual stack-rendering
implementation before this goal can be marked done by inspection alone.

**Fallback, regardless of that answer:** add a small belt-and-suspenders
draft-persistence mechanism scoped to `AgentFooter`, independent of whether
the tab host keeps it mounted:

- On a debounced interval while typing (piggybacking the existing RAF-
  debounced `onTyping` callback — no new timer), write the current textarea
  value to a per-block in-memory store (`Map<blockId, string>`, module-level,
  matching the existing `sentHistory`/`histDraft` closure-`let` precedent
  already in this file for other per-pane ephemeral state).
- On mount, if an entry exists for this `blockId`, seed the textarea from it
  (same precedence slot as the ghost-text placeholder logic already handles
  — an explicit draft always wins over a suggestion).
- Cleared on send (already happens today) and on the block's own unmount-for-
  real (pane closed, not just tab-switched) via `onCleanup`.
- This is deliberately NOT persisted to backend/block-meta — it's a same-
  session, in-memory safety net, not a durability feature (see Non-goals).

This makes the guarantee correct either way the §6 question resolves: if
stack tabs already keep inactive members mounted, the fallback is inert
(the DOM value was never lost, so the seed-on-mount branch never fires). If
they don't, the fallback is what actually saves the draft.

---

## 4. What gets removed

- `agent-view.tsx`: the `bodyMode` signal, the `<Show when={bodyMode()===...}>`
  swap, the `earlierHistoryAvailable` `PaneRow` block (§3.2 replaces it with
  a virtualized node).
- `AgentHistoryView.tsx`: the `onClose` prop and its "← Back to conversation"
  affordance (tabs close via the standard ×).
- Any control-bar / cog-menu "View full history" entry that called
  `setBodyMode("history")` (§4.2 of the session-scoped-scrollback spec) is
  repointed at the same `openOrFocusHistoryTab()` helper §3.1–3.3 share.

## 5. Scope and blast radius

- **Frontend:**
  `agent-view.tsx` (`AgentViewWrapper`'s tab-open branch, remove `bodyMode`),
  new `AgentHistoryTabView.tsx` (thin wrapper, replaces the old swap branch),
  `agent-model.ts` (+`getBodyContextMenuItems`),
  `virtualization/{DocumentRow.tsx,renderers.ts,expansion-source.ts}`
  (+`history_link` synthetic row, same shape as `day_divider`),
  `components/AgentFooter.tsx` (+draft-persistence fallback, §3.4),
  `components/AgentControlBar.tsx` (repoint existing entry).
- **Backend:** none — this is purely a frontend hosting/entry-point change;
  the data layer (§4.3/§4.4 of the session-scoped-scrollback spec) is
  untouched.
- **Risk concentration:** the tab-open/focus-or-create logic (§3.1's
  idempotency check) — getting the "already open, focus don't duplicate"
  condition wrong either stacks duplicate history tabs or, worse, focuses
  the wrong block. Mirrors an already-solved case (fork-tab dedup in
  `combinedTabs`), so implement by extending that same memo/filter rather
  than writing new dedup logic.

## 6. Open questions

1. **Do inactive blockStack tabs stay mounted (hidden) or unmount on
   switch?** Not yet confirmed by inspection — the leaf-node rendering that
   consumes `node.data.activeBlockId` vs `blockStack` wasn't conclusively
   located in this pass. This matters beyond composer drafts: if switching
   away from the live tab tears down its `useAgentStream` WS subscription
   entirely, reconnecting on switch-back needs to be at least as robust as
   the existing reconnect-on-reopen path. §3.4's fallback covers the draft
   specifically regardless of the answer, but the WS-subscription question
   should be settled (by reading the actual tile/leaf rendering component,
   not this doc's own grep pass) before implementation starts, since it
   changes whether §3.4's fallback is the *only* thing needed or whether
   switch-away/back also needs the same reconnect treatment pane-reopen
   already has.
2. **Tab label collision:** if a user already renamed a fork tab to
   "History" (unlikely but possible — tab titles are free text per
   `handleTabRenameConfirm`), does the history tab need a disambiguating
   icon/prefix rather than relying on label uniqueness? Lean: give the
   history tab a distinct icon in `getTabClass`/`renderLabel` (matching how
   fork tabs already get their own status-accent class) rather than solving
   this via label text — cheap, and avoids the question entirely.
3. **Should `history_link`'s synthetic node also appear for the P3 archives
   entry point (§4.5 of the session-scoped-scrollback spec)?** Deferred —
   archives aren't shipped yet; revisit when §4.5 lands.

## 7. Testing

- **Reducer/parse units:** `history_link` synthetic-node injection (position
  relative to the `fresh` `session_outcome` node, stable id so it doesn't
  duplicate across `loadOlder` pages, never appears when
  `earlierHistoryAvailable`-equivalent condition is false).
- **Tab logic units:** open-or-focus dedup (opening twice focuses, doesn't
  duplicate; closing the history tab and reopening creates a fresh one;
  closing the *live* tab while a history tab is open behaves like closing
  any other stack member).
- **AgentFooter unit:** draft survives a simulated unmount/remount with the
  same `blockId` (proves the fallback works standing alone, independent of
  §6's answer); draft does NOT leak across two *different* `blockId`s typing
  concurrently (map is keyed correctly); cleared on send; cleared on real
  pane-close `onCleanup`.
- **Manual/live:** type a draft → open Agent History via the link row →
  switch back via the tab strip → draft intact. Repeat via the right-click
  "Agent History" entry. Open Agent History twice → confirms single tab,
  second click just focuses. Confirm the "Working…"/"✓ Worked" row (today's
  separate fix) has nothing left to regress against, since the remount it
  depended on no longer happens at all.

## 8. Phasing

| Phase | Scope | Outcome |
|-------|-------|---------|
| **P1** | §3.1 tab-based hosting (replaces `bodyMode`) + §4 removals | Root cause of the remount bug class gone; history is a real tab |
| **P2** | §3.2 link row as scrolling synthetic node | Visual fix — row scrolls with the "New Session" divider it's paired with |
| **P3** | §3.3 context-menu entry + §3.4 draft-preservation fallback | Second entry point; draft-loss guarantee closed regardless of §6's answer |
