// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block, resolveEffectiveViewType } from "@/app/block/block";
import { WOS } from "@/app/store/global";
import { getLayoutModelForStaticTab, type NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import { Key } from "@solid-primitives/keyed";
import { createMemo, createSignal, For, Show, type JSX } from "solid-js";

/**
 * Router `tabcontent.tsx` renders instead of `<Block>` directly — the
 * leaf-level entry point for the pane-tab-switch chrome-stability fix
 * (`docs/specs/SPEC_PANE_TAB_SWITCH_CHROME_STABILITY_2026_09_07.md`).
 *
 * ALWAYS renders a per-active-member, `<Key>`-scoped `<Block>` — the inner
 * remount boundary `layoutStack.ts`'s own header comment refers to. This
 * alone is what updates displayed content on a switch now that the OUTER,
 * whole-leaf remount (`tilelayout-shared.tsx`'s `DisplayNodesWrapper`) no
 * longer does — for every view type, uniformly, including one that never
 * hoists chrome at all: the end result (`<Block>` remounts on every switch)
 * is byte-for-byte identical to what the old outer-leaf-key mechanism
 * produced, just driven from a narrower boundary.
 *
 * When the active member's EFFECTIVE view type (`resolveEffectiveViewType`
 * — the same migration/rename redirects `block.tsx`'s own `makeViewModel`
 * applies, so a still-live "forge" block routes through this too) is
 * `"agent"`, wraps that `<Block>` in the currently-active `ViewModel`'s own
 * `renderPaneChrome` — a stable outer shell (header, tab strip,
 * progress-bar slot) that survives every subsequent switch, instead of
 * `BlockFrame`'s own per-switch-remounting header. Every agent pane, not
 * just already-stacked ones; see the `hoisted` memo below for why gating
 * that on stack size was a catch-22. Any other view type falls through to
 * the bare `content` below — zero behavior change for those, matching what
 * `<Block>` alone already did.
 */
/**
 * EFFECTIVE view types (post-`resolveEffectiveViewType`) whose `ViewModel`
 * implements `renderPaneChrome` — i.e. the ones that own an in-pane tab
 * strip and therefore need their chrome hoisted out of the per-block
 * remount boundary. Everything else takes the passthrough branch below,
 * unchanged from what a plain `<Block>` always did.
 *
 * A view type listed here MUST also set `noHeader` off
 * `nodeModel.paneChromeHoisted` (see AgentViewModel/TermViewModel), or its
 * inline BlockFrame header and its hoisted one will both render.
 */
const HOISTS_OWN_CHROME = new Set(["agent", "term"]);

/**
 * Subset of `HOISTS_OWN_CHROME` whose stack members stay mounted
 * SIMULTANEOUSLY instead of being swapped one-at-a-time behind the inner
 * `<Key>` — every member's own `<Block>` (xterm instance, PTY connection,
 * ViewModel) is created once and never torn down again while it stays in
 * the stack; switching tabs just toggles which one is visible. Reported
 * live after the chrome-stability fix landed: with header/strip already
 * stable, a terminal tab switch still showed a brief spinner/re-render
 * flash from `block.tsx`'s ready()-gate cross-fade, since the inner `<Key>`
 * was STILL fully remounting `<Block>` (new `TermViewModel`, new xterm.js
 * instance) on every switch — editor's own file tabs never do this at all
 * (one persistent block, no remount), which is why editor felt "flawless"
 * by comparison. Scoped to `"term"` only for now: each terminal tab is a
 * genuinely separate backend-backed block with its own live PTY, unlike
 * agent's picker/history tabs, which have more entangled per-tab state
 * (quick-fork, launch-in-place) not yet audited for keep-alive safety.
 */
const KEEP_ALIVE_TYPES = new Set(["term"]);

export function PaneLeafChrome(props: { nodeModel: NodeModel }): JSX.Element {
    const nodeModel = props.nodeModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;

    // Effective view type of the ACTIVE member, reactive — getWaveObjectAtom
    // inside a memo, not useWaveObjectValue, the same reactive-oref pattern
    // PR #3134 established for BlockFrame_Header (frontend/app/store/wos.ts's
    // own doc comments explain why: useWaveObjectValue's onCleanup-ref-count
    // is tied to THIS component's mount, and never re-subscribes if the
    // oref it was called with later changes).
    const activeBlockData = createMemo(() => WOS.getWaveObjectAtom<Block>(WOS.makeORef("block", activeBlockId()))());
    const effectiveViewType = createMemo(() => resolveEffectiveViewType(activeBlockData()?.meta?.view ?? ""));

    // Hoist for EVERY agent pane, not just ones whose stack has already
    // gone multi-member. An earlier version gated this on
    // `NodeModel.hasEverBeenMultiMember` to keep never-stacked panes on a
    // byte-for-byte passthrough — but that was a catch-22, found live:
    // AgentPaneChrome owns the tab strip, the tab strip owns the "+"
    // button, and "+" is the only way to reach a 2nd stack member. Gated
    // that way, a single-member pane rendered no strip and therefore no
    // "+", so `hasEverBeenMultiMember` could never become true and the
    // whole feature was unreachable. The strip's own visibility rules
    // (`shouldShowTabStrip` — hidden on a fresh picker pane, "+"-only for
    // one live conversation, pills once there are 2+) still live inside
    // chrome and are unchanged; this only decides whether chrome EXISTS.
    //
    // Latched for the same reason the ViewModel below is: `effectiveViewType()`
    // reads the ACTIVE member's block meta, which is briefly undefined
    // while a newly-activated member's data loads, so an unlatched read
    // would blip false->true on a switch and remount chrome — the exact
    // flash this file exists to prevent.
    let latchedHoisted = false;
    const hoisted = createMemo(() => {
        if (!latchedHoisted && HOISTS_OWN_CHROME.has(effectiveViewType())) {
            latchedHoisted = true;
        }
        return latchedHoisted;
    });

    // Same latch shape as `hoisted` above, over the narrower KEEP_ALIVE_TYPES
    // set — see that const's own doc comment for what this changes and why.
    let latchedKeepAlive = false;
    const keepAlive = createMemo(() => {
        if (!latchedKeepAlive && KEEP_ALIVE_TYPES.has(effectiveViewType())) {
            latchedKeepAlive = true;
        }
        return latchedKeepAlive;
    });

    // Block-scoped NodeModel wrapper for the inner, per-activation <Block>
    // mount. The LEAF's own NodeModel.blockId is frozen — captured once,
    // never re-derived (see that field's own doc comment in types.ts) —
    // and, now that layoutStack.ts no longer disposes this NodeModel on a
    // switch, would show the SAME member forever if passed straight into
    // <Block>. Reconstructed (new object identity) exactly when
    // activeBlockId() changes — i.e. exactly when the <Key> below remounts
    // anyway — so nothing ever observes a wrapper whose .blockId doesn't
    // match its own <Block> mount's lifetime, satisfying
    // NodeModel.blockId's "one instance, one immutable blockId" contract
    // at this narrower, per-activation granularity. Every other NodeModel
    // field (focus/magnify/minimize/close/etc.) delegates straight through
    // to the real, leaf-level nodeModel via the spread — those stay
    // leaf-scoped, unaffected by which member is active.
    // `paneChromeHoisted` tags the wrapper so a ViewModel can tell whether
    // something is rendering a replacement header ABOVE it (codex P2 on
    // this PR). AgentViewModel.noHeader reads it: an agent Block reached
    // through THIS file's hoisted branch suppresses BlockFrame's inline
    // header (chrome supplies one), but the same ViewModel class rendering
    // a drag-preview thumbnail — `tabcontent.tsx`'s `renderPreview`, a
    // plain `<Block preview>` with the raw leaf nodeModel and no chrome
    // around it — must keep its inline header or the thumbnail loses its
    // title entirely. A plain field, not a signal, deliberately: it's known
    // at wrapper-construction time, so there's no window where both headers
    // could render at once.
    //
    // Used only when !keepAlive() — the single-active-member path below,
    // unchanged from before KEEP_ALIVE_TYPES existed.
    const scopedNodeModel = createMemo<NodeModel>(() => ({
        ...nodeModel,
        blockId: activeBlockId(),
        paneChromeHoisted: hoisted(),
    }));

    // --- Keep-alive machinery (only ever touched when keepAlive() is true) ---
    //
    // Under keep-alive, EVERY stack member's <Block> mounts once and stays
    // mounted — so `nodeModel.setActiveViewModel`/`activeViewModel` (a
    // single shared slot, last-writer-wins) can no longer be trusted: every
    // kept-alive Block calls `setActiveViewModel` once at its OWN mount, not
    // on every switch, so the shared slot would end up holding whichever
    // tab happened to mount MOST RECENTLY — not necessarily the one
    // currently visible. `TermPaneChrome`'s own `runtimeLabel` and the
    // header's `viewModel` prop both read `nodeModel.activeViewModel()`
    // expecting it to track the ACTIVE tab on every switch (they don't
    // latch), so this isn't just a chrome-identity concern the way
    // `chromeVm` below is — it's a live-correctness one.
    //
    // Fixed by giving each kept-alive blockId its OWN private, owner-checked
    // slot (mirrors layoutNodeModels.ts's real activeViewModel/
    // setActiveViewModel pair), and deriving the LEAF-level accessor chrome
    // actually reads from a lookup keyed by the CURRENT activeBlockId() —
    // so switching tabs is just pointing the read at a different
    // already-populated slot, no remount, no blip.
    interface ViewModelSlot {
        get: () => ViewModel | null;
        set: (vm: ViewModel | null, owner: object) => void;
    }
    const viewModelSlots = new Map<string, ViewModelSlot>();
    function viewModelSlotFor(id: string): ViewModelSlot {
        let slot = viewModelSlots.get(id);
        if (!slot) {
            let owner: object | null = null;
            const [get, setSig] = createSignal<ViewModel | null>(null);
            const set = (vm: ViewModel | null, o: object) => {
                if (vm === null) {
                    if (owner !== o) return;
                    owner = null;
                    setSig(null);
                    return;
                }
                owner = o;
                setSig(() => vm);
            };
            slot = { get, set };
            viewModelSlots.set(id, slot);
        }
        return slot;
    }

    const keepAliveNodeModelCache = new Map<string, NodeModel>();
    function keepAliveNodeModelFor(id: string): NodeModel {
        let scoped = keepAliveNodeModelCache.get(id);
        if (!scoped) {
            const slot = viewModelSlotFor(id);
            scoped = {
                ...nodeModel,
                blockId: id,
                paneChromeHoisted: true,
                activeViewModel: slot.get,
                setActiveViewModel: slot.set,
            } as NodeModel;
            keepAliveNodeModelCache.set(id, scoped);
        }
        return scoped;
    }

    // The full stack, reactive — same `localTreeStateAtom()` + live
    // `findNode` lookup pattern term.tsx's own `termTabs` and agent-view.tsx's
    // `stackTabs` already use, falling back to a single-entry list when this
    // leaf hasn't split into a stack yet.
    const layoutModel = getLayoutModelForStaticTab();
    const stackBlockIds = createMemo<string[]>(() => {
        layoutModel.localTreeStateAtom();
        const node = findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);
        const stack = node?.data?.blockStack?.length ? node.data.blockStack : [activeBlockId()];
        if (keepAlive()) {
            // Drop cache/slot entries for blockIds no longer in the stack
            // (tab closed) so a closed tab's ViewModel/NodeModel wrapper
            // don't linger for the rest of the pane's lifetime.
            const live = new Set(stack);
            for (const id of keepAliveNodeModelCache.keys()) {
                if (!live.has(id)) keepAliveNodeModelCache.delete(id);
            }
            for (const id of viewModelSlots.keys()) {
                if (!live.has(id)) viewModelSlots.delete(id);
            }
        }
        return stack;
    });

    // The NodeModel chrome itself renders with — plain passthrough when not
    // keeping tabs alive (unchanged), otherwise `activeViewModel` is
    // overridden to the per-id lookup above so a switch re-points the read
    // at the newly-active tab's own slot instead of following whichever
    // Block mounted last.
    const chromeNodeModel = createMemo<NodeModel>(() => {
        if (!keepAlive()) return nodeModel;
        return {
            ...nodeModel,
            activeViewModel: () => viewModelSlots.get(activeBlockId())?.get() ?? null,
        };
    });

    const content = (
        <Show
            when={keepAlive()}
            fallback={
                <Key each={[scopedNodeModel()]} by={(nm) => nm.blockId}>
                    {(nm) => <Block nodeModel={nm()} preview={false} />}
                </Key>
            }
        >
            {/* Every stack member mounted simultaneously, absolutely
                positioned over one another inside the (already
                `position: relative`) content slot each keep-alive pane
                type reserves (e.g. term.scss's `.term-pane-stack-content`)
                — visibility, not existence, is what switches. A hidden
                member keeps its real (non-zero) box size the whole time
                (`visibility: hidden` does not collapse layout the way
                `display: none` would), so its own ResizeObserver-driven
                fit logic (e.g. TermWrap's) stays correct in the background
                and there is nothing to re-fit when it's revealed again. */}
            <For each={stackBlockIds()}>
                {(id) => (
                    <div
                        class="pane-leaf-keepalive-slot"
                        style={{
                            position: "absolute",
                            inset: "0",
                            visibility: id === activeBlockId() ? "visible" : "hidden",
                            "pointer-events": id === activeBlockId() ? "auto" : "none",
                        }}
                    >
                        <Block nodeModel={keepAliveNodeModelFor(id)} preview={false} />
                    </div>
                )}
            </For>
        </Show>
    );


    // ReAgent P1, confirmed by an empirical repro before trusting it:
    // `NodeModel.activeViewModel()` genuinely blips through `null` on
    // EVERY switch, not just the first hoist. `block.tsx`'s `onCleanup`
    // (clearing the OLD member's vm) runs synchronously during the inner
    // `<Key>`'s reconciliation; the `createEffect` that sets the NEW
    // member's vm is deferred to the next effects flush. A naive
    // `<Show when={nodeModel.activeViewModel()}>` callback form looks safe
    // (`<Show>` only re-runs its child function on a falsy->truthy edge),
    // but that real null blip IS such an edge every time, so it would
    // re-invoke `renderPaneChrome` — mounting a BRAND NEW `AgentPaneChrome`
    // on every switch and reproducing the exact flash this file exists to
    // eliminate. Fixed by latching the FIRST non-null vm ever observed
    // into a plain closure variable: once captured, this memo's OWN return
    // value stops changing (same object reference) regardless of how
    // `activeViewModel()` fluctuates underneath, so Solid's default `===`
    // memo comparison means no consumer — including `<Show>` below — ever
    // observes a falsy value again after the first real capture. Renders
    // via whichever vm happened to be active at that FIRST hoist and never
    // re-derives afterward — the same "frozen after first hoist" identity
    // `anchorBlockId` (agent-model.ts's own `renderPaneChrome` closure)
    // already assumes. Reads through `chromeNodeModel()` (not the raw
    // `nodeModel` prop) so the keep-alive path latches onto the per-id
    // lookup's current value rather than the shared (and, under keep-alive,
    // unreliable) leaf-level slot — doesn't change what gets latched for
    // the non-keep-alive path, where `chromeNodeModel()` IS `nodeModel`.
    let latchedChromeVm: ViewModel | null = null;
    const chromeVm = createMemo(() => {
        if (!latchedChromeVm) {
            latchedChromeVm = chromeNodeModel().activeViewModel?.() ?? null;
        }
        return latchedChromeVm;
    });

    return (
        <Show when={hoisted()} fallback={content}>
            {/* Guards only the leaf's own first-hoist window, before any
                ViewModel has been observed yet — does NOT itself unmount
                the outer chrome once mounted, since `chromeVm()` (above)
                never returns to a falsy value after its first real
                capture. */}
            <Show when={chromeVm()}>
                {(vm) => vm().renderPaneChrome!(chromeNodeModel(), content)}
            </Show>
        </Show>
    );
}
