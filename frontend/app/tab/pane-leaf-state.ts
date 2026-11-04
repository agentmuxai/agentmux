// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The state behind a pane leaf (pane-leaf-chrome.tsx): which kind of pane the active member
 * makes it, the per-tab ViewModel slots and the NodeModel wrappers handed to each Block, the
 * stack's members and which of them are mounted, the mount watch, the minimized flag, and the
 * ViewModel the chrome latches onto. The hooks are called in this order from PaneLeafChrome,
 * which is also the order their memos and effects are created.
 */

import { resolveEffectiveViewType } from "@/app/block/block";
import { getPaneTab, isKeepAliveView } from "@/app/store/pane-tab-registry";
import { setBlockMinimized } from "@/app/block/pane-minimized";
import { agentMountForgotten, agentMountSettled, expectAgentMount } from "@/app/tab/agent-mount-watch";
import { MOS } from "@/app/store/global";
import type { NodeModel } from "@/layout/index";
import { paneStackIds } from "@/layout/lib/layoutNodeModels";
import { createEffect, createMemo, createSignal, onCleanup, untrack, type Accessor } from "solid-js";

/**
 * Whether an EFFECTIVE view type (post-`resolveEffectiveViewType`) gets the
 * shared pane chrome (header + tab strip + "+", hoisted out of the per-block
 * remount boundary): every REGISTERED view type does.
 *
 * This used to be a hand-maintained `HOISTS_OWN_CHROME` list that each view
 * also had to match with its own `renderPaneChrome` and `noHeader`. Views
 * missing from it (toolchain, settings, launcher, identity, memory) got no
 * tab strip when a pane started with them, and a DOUBLE header when added to
 * a pane that already had chrome. Now the host decides for all of them:
 * chrome is `vm.renderPaneChrome ?? renderPaneChromeShell` (in `PaneLeafChrome`, pane-leaf-chrome.tsx), and
 * BlockFrame hides its inline header off `paneChromeHoisted` itself
 * (blockframe.tsx). SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §4, Phase 1.
 *
 * An empty or unregistered type (block meta not loaded yet, or an unknown
 * view) stays a plain `<Block>` until it resolves.
 */
function hoistsOwnChrome(viewType: string): boolean {
    return getPaneTab(viewType) != null;
}

export interface PaneKinds {
    /** The effective view type of the ACTIVE member. */
    effectiveViewType: Accessor<string>;
    /** The pane gets the shared chrome; latched on. */
    hoisted: Accessor<boolean>;
    /** The pane keeps its keep-alive members mounted; latched on. */
    keepAlive: Accessor<boolean>;
}

/**
 * Keep-alive view types (`isKeepAliveView`: the manifest's
 * `capabilities.lifecycle === "keepAlive"`, declared in block-registry.ts,
 * which also records why each of term, agent, browser and editor is one) —
 * all of which get the shared chrome, see `hoistsOwnChrome` — keep their
 * stack members mounted
 * SIMULTANEOUSLY instead of being swapped one-at-a-time behind the inner
 * `<Key>` — every member's own `<Block>` (xterm instance/PTY connection for
 * term; `AgentViewModel` + parsed document for agent) is created once and
 * never torn down again while it stays in the stack; switching tabs just
 * toggles which one is visible. Reported live after the chrome-stability fix
 * landed: with header/strip already stable, a terminal tab switch still
 * showed a brief spinner/re-render flash from `block.tsx`'s ready()-gate
 * cross-fade, since the inner `<Key>` was STILL fully remounting `<Block>`
 * (new `TermViewModel`, new xterm.js instance) on every switch — editor's
 * own file tabs never do this at all (one persistent block, no remount),
 * which is why editor felt "flawless" by comparison.
 *
 * Agent is keep-alive per SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md, after an
 * explicit audit of the entangled per-tab state this comment used
 * to warn about (quick-fork, launch-in-place): both reset via mechanisms
 * already internal to a single Block (an in-place `<Show when={agentId()}>`
 * swap for launch-in-place; a `pushBlockOntoStack` for quick-fork), never by
 * relying on the LEAF remounting the Block — so keep-alive doesn't disturb
 * either. The one real behavior change the audit found: two user-facing
 * timers (AgentQuestionPanel's auto-timeout, useAgentFailure's auto-retry)
 * used to implicitly pause when a backgrounded tab unmounted; under
 * keep-alive they'd otherwise keep running invisibly. Both are now
 * explicitly gated on `isBlockDormant` instead of relying on that
 * incidental pause — see each one's own doc comment.
 *
 * Browser and editor are keep-alive per the repo owner's decision in
 * SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §5: remounting a browser reloads
 * its page on every switch (scroll, form input and in-page state lost, title
 * and favicon re-loading), and remounting an editor loses cursor, scroll and
 * undo. A kept-alive browser's native page is collapsed while its tab is
 * dormant (`isBlockDormant` in use-pane-rect-sync.ts), so it can't show over
 * the active tab.
 *
 * This is PER TAB: only members whose OWN view type is keep-alive stay
 * mounted while inactive. Every other member (help, sysinfo, swarm, …) is
 * unmounted when it isn't the active tab, even in a pane that keeps other
 * tabs alive — see `mountedBlockIds` below. (It used to be per pane: one
 * agent or terminal tab kept every tab of its pane mounted, which is what let
 * Help's content ghost over the next tab.)
 */
export function usePaneKinds(activeBlockId: () => string): PaneKinds {
    // Effective view type of the ACTIVE member, reactive — getMuxObjectAtom
    // inside a memo, not useMuxObjectValue, the same reactive-oref pattern
    // PR #3134 established for BlockFrame_Header (frontend/app/store/mos.ts's
    // own doc comments explain why: useMuxObjectValue's onCleanup-ref-count
    // is tied to THIS component's mount, and never re-subscribes if the
    // oref it was called with later changes).
    const activeBlockData = createMemo(() => MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", activeBlockId()))());
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
    // ("+"-only for one live conversation, pills once there are 2+; "+" is
    // always shown, matching every other widget type) still live inside
    // chrome and are unchanged; this only decides whether chrome EXISTS.
    //
    // Latched for the same reason the ViewModel below is: `effectiveViewType()`
    // reads the ACTIVE member's block meta, which is briefly undefined
    // while a newly-activated member's data loads, so an unlatched read
    // would blip false->true on a switch and remount chrome — the exact
    // flash this file exists to prevent.
    let latchedHoisted = false;
    const hoisted = createMemo(() => {
        if (!latchedHoisted && hoistsOwnChrome(effectiveViewType())) {
            latchedHoisted = true;
        }
        return latchedHoisted;
    });

    // Same latch shape as `hoisted` above, over the narrower keep-alive view types
    // set — see `usePaneKinds`' own doc comment for what this changes and why.
    let latchedKeepAlive = false;
    const keepAlive = createMemo(() => {
        if (!latchedKeepAlive && isKeepAliveView(effectiveViewType())) {
            latchedKeepAlive = true;
        }
        return latchedKeepAlive;
    });

    return { effectiveViewType, hoisted, keepAlive };
}

interface ViewModelSlot {
    get: () => ViewModel | null;
    set: (vm: ViewModel | null, owner: object) => void;
}

export interface ViewModelScopes {
    viewModelSlotFor: (id: string) => ViewModelSlot;
    keepAliveNodeModelFor: (id: string) => NodeModel;
    /** Drop the slot and wrapper of every blockId that is not in `live`. */
    prune: (live: Set<string>) => void;
}

/**
 * Per-tab ViewModel slots (both paths) and the kept-alive members' NodeModel wrappers.
 *
 * Chrome must read the ViewModel of the tab that is active NOW. The leaf's
 * own `activeViewModel` is one shared, last-writer-wins slot, and it is
 * wrong on BOTH paths:
 * - Without keep-alive, a switch updates `activeBlockId()` at once, but the
 *   old tab's ViewModel stays in the shared slot until its <Block>
 *   unmounts, and the new one only lands in a later effect. Anything that
 *   read both in between (PaneChrome's per-pill label/favicon memory) saw
 *   tab X's ViewModel while tab Y was active, and saved X's title/favicon
 *   under Y: pills showing another tab's name, icon, or a browser's
 *   favicon (confirmed live with a diagnostic, 2026-09-24).
 * - Under keep-alive (see `usePaneKinds`), every member mounts once, so the shared slot
 *   holds whichever mounted last.
 * So every <Block> writes its ViewModel into a slot keyed by its OWN
 * blockId, and chrome reads the slot of `activeBlockId()`. During a switch
 * the new tab's slot is briefly empty (null), never another tab's.
 *
 * Under keep-alive, EVERY stack member's <Block> mounts once and stays
 * mounted — so `nodeModel.setActiveViewModel`/`activeViewModel` (a
 * single shared slot, last-writer-wins) can no longer be trusted: every
 * kept-alive Block calls `setActiveViewModel` once at its OWN mount, not
 * on every switch, so the shared slot would end up holding whichever
 * tab happened to mount MOST RECENTLY — not necessarily the one
 * currently visible. Terminal's own `runtimeLabel` and the
 * header's `viewModel` prop both read `nodeModel.activeViewModel()`
 * expecting it to track the ACTIVE tab on every switch (they don't
 * latch), so this isn't just a chrome-identity concern the way
 * `chromeVm` is — it's a live-correctness one.
 *
 * Fixed by giving each kept-alive blockId its OWN private, owner-checked
 * slot (mirrors layoutNodeModels.ts's real activeViewModel/
 * setActiveViewModel pair), and deriving the LEAF-level accessor chrome
 * actually reads from a lookup keyed by the CURRENT activeBlockId() —
 * so switching tabs is just pointing the read at a different
 * already-populated slot, no remount, no blip.
 */
export function createViewModelScopes(nodeModel: NodeModel, hoisted: Accessor<boolean>): ViewModelScopes {
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
                // The live `hoisted` memo, same as `useScopedNodeModel`'s
                // — not a bare `true` literal. This branch is only ever
                // reachable once `hoisted()` is already true, but keeping
                // it as the SAME shared accessor (rather than a
                // once-true-forever snapshot) is what lets `noHeader`
                // stay correct across a ViewModel adoption regardless of
                // which wrapper it's currently holding. See
                // `BlockNodeModel.paneChromeHoisted`'s own doc comment.
                paneChromeHoisted: hoisted,
                activeViewModel: slot.get,
                setActiveViewModel: slot.set,
            } as NodeModel;
            keepAliveNodeModelCache.set(id, scoped);
        }
        return scoped;
    }

    const prune = (live: Set<string>) => {
        for (const id of keepAliveNodeModelCache.keys()) {
            if (!live.has(id)) keepAliveNodeModelCache.delete(id);
        }
        for (const id of viewModelSlots.keys()) {
            if (!live.has(id)) viewModelSlots.delete(id);
        }
    };

    return { viewModelSlotFor, keepAliveNodeModelFor, prune };
}

/**
 * Block-scoped NodeModel wrapper for the inner, per-activation <Block>
 * mount. The LEAF's own NodeModel.blockId is frozen — captured once,
 * never re-derived (see that field's own doc comment in types.ts) —
 * and, now that layoutStack.ts no longer disposes this NodeModel on a
 * switch, would show the SAME member forever if passed straight into
 * <Block>. Reconstructed (new object identity) exactly when
 * activeBlockId() changes — i.e. exactly when the <Key> remounts
 * anyway — so nothing ever observes a wrapper whose .blockId doesn't
 * match its own <Block> mount's lifetime, satisfying
 * NodeModel.blockId's "one instance, one immutable blockId" contract
 * at this narrower, per-activation granularity. Every other NodeModel
 * field (focus/magnify/minimize/close/etc.) delegates straight through
 * to the real, leaf-level nodeModel via the spread — those stay
 * leaf-scoped, unaffected by which member is active.
 * `paneChromeHoisted` tags the wrapper so a ViewModel can tell whether
 * something is rendering a replacement header ABOVE it. BlockFrame's
 * `noHeader` reads it: a Block reached
 * through the hoisted branch suppresses BlockFrame's inline
 * header (chrome supplies one), but the same view rendering
 * a drag-preview thumbnail — `tabcontent.tsx`'s `renderPreview`, a
 * plain `<Block preview>` with the raw leaf nodeModel and no chrome
 * around it — must keep its inline header or the thumbnail loses its
 * title entirely.
 *
 * The `hoisted` MEMO ITSELF, not `hoisted()`'s current value — see
 * `BlockNodeModel.paneChromeHoisted`'s own doc comment (blocktypes.ts)
 * for why a frozen snapshot isn't safe here: block.tsx's ViewModel
 * registry can ADOPT an existing ViewModel (skipping construction)
 * for a later wrapper whose snapshot would've read differently.
 * Forwarding the live memo means every wrapper built for this leaf
 * — regardless of which one a ViewModel instance ends up holding —
 * reports the SAME, always-current value.
 *
 * Used only when !keepAlive() — the single-active-member path,
 * unchanged from before keep-alive view types existed.
 */
export function useScopedNodeModel(
    nodeModel: NodeModel,
    activeBlockId: () => string,
    hoisted: Accessor<boolean>,
    scopes: ViewModelScopes,
): Accessor<NodeModel> {
    return createMemo<NodeModel>(() => {
        const id = activeBlockId();
        const slot = scopes.viewModelSlotFor(id);
        return {
            ...nodeModel,
            blockId: id,
            paneChromeHoisted: hoisted,
            // Also record the ViewModel in THIS block's own slot (see
            // `createViewModelScopes`), so chrome reads the active tab's
            // ViewModel by id instead of the leaf's shared last-writer-wins
            // slot. Still forwarded to the leaf's own setter, which other
            // consumers of the raw NodeModel keep reading exactly as before.
            setActiveViewModel: (vm: ViewModel | null, owner: object) => {
                slot.set(vm, owner);
                nodeModel.setActiveViewModel?.(vm, owner);
            },
        } as NodeModel;
    });
}

export interface StackMembers {
    stackBlockIds: Accessor<string[]>;
    viewTypeOf: (id: string) => string;
    /** The members actually mounted in the keep-alive branch. */
    mountedBlockIds: Accessor<string[]>;
}

export function useStackMembers(
    nodeModel: NodeModel,
    activeBlockId: () => string,
    scopes: ViewModelScopes,
): StackMembers {
    const stackBlockIds = createMemo<string[]>(() => {
        const stack = paneStackIds(nodeModel);
        // Drop cache/slot entries for blockIds no longer in the stack (tab
        // closed or moved away) so a gone tab's ViewModel/NodeModel wrapper
        // don't linger for the rest of the pane's lifetime.
        scopes.prune(new Set(stack));
        return stack;
    });

    // Which members are actually mounted in the keep-alive branch: the active
    // one, plus any member whose OWN view type keeps its state
    // (keep-alive view types). A remount-type member (help, sysinfo, …) mounts only
    // while it's the active tab and unmounts as soon as another tab is picked,
    // exactly as it would in a pane with no keep-alive tabs at all. A member
    // whose block data hasn't loaded yet counts as remount until it has.
    // SPEC_PANE_TAB_CONTRACT_V1_2026_09_24.md §5.
    const viewTypeOf = (id: string) =>
        resolveEffectiveViewType(MOS.getMuxObjectAtom<Block>(MOS.makeORef("block", id))()?.meta?.view ?? "");
    const mountedBlockIds = createMemo<string[]>(() => {
        const active = activeBlockId();
        return stackBlockIds().filter((id) => id === active || isKeepAliveView(viewTypeOf(id)));
    });

    return { stackBlockIds, viewTypeOf, mountedBlockIds };
}

/**
 * Names the step that did not happen if an agent member's view never mounts
 * (agent-mount-watch.ts). A member whose data has not loaded yet is watched too:
 * an agent that never loads is the same silence.
 * Keyed on each member's view type alone, so a meta update to a block does not re-run it.
 */
export function useAgentMountWatch(nodeModel: NodeModel, activeBlockId: () => string, members: StackMembers): void {
    const { stackBlockIds, viewTypeOf, mountedBlockIds } = members;
    const memberViews = createMemo(() => stackBlockIds().map((id) => `${id}:${viewTypeOf(id)}`).join("|"));
    createEffect(() => {
        memberViews();
        for (const id of untrack(stackBlockIds)) {
            const view = untrack(() => viewTypeOf(id));
            if (view === "agent" || view === "") {
                expectAgentMount(
                    id,
                    () =>
                        `view=${viewTypeOf(id) || "not loaded"} active=${id === activeBlockId()} ` +
                        `mounted=${mountedBlockIds().includes(id)} minimized=${nodeModel.isMinimized()} stack=${stackBlockIds().length}`,
                );
            } else {
                agentMountSettled(id);
            }
        }
    });
    onCleanup(() => {
        for (const id of stackBlockIds()) agentMountForgotten(id);
    });
}

/**
 * A minimized pane is not on screen, whichever of its tabs is in front: every member is
 * dormant in the visibility signal, so a row of minimized agents stops doing the work of
 * visible ones. Restoring the pane makes the front member active again.
 */
export function useMinimizedMembers(nodeModel: NodeModel, stackBlockIds: Accessor<string[]>): void {
    createEffect(() => {
        // A maximized pane shows its content in the overlay, so it is on screen even while its flag stays minimized.
        const minimized = (nodeModel.isMinimized?.() ?? false) && !(nodeModel.isMagnified?.() ?? false);
        for (const id of stackBlockIds()) setBlockMinimized(id, minimized);
    });
    onCleanup(() => {
        for (const id of stackBlockIds()) setBlockMinimized(id, false);
    });
}

/**
 * The NodeModel chrome itself renders with — plain passthrough when not
 * keeping tabs alive (unchanged), otherwise `activeViewModel` is
 * overridden to the per-id lookup so a switch re-points the read
 * at the newly-active tab's own slot instead of following whichever
 * Block mounted last.
 *
 * `viewModelSlotFor(activeBlockId())` is called here, EAGERLY, not just
 * inside `keepAliveNodeModelFor` — closes a real race (found live, via
 * a diagnostic pass reproducing a permanently-blank pane): a slot is a
 * lazily-created `createSignal`, so `chromeVm` (which subscribes
 * via `viewModelSlots.get(id)?.get()`) only actually establishes a
 * Solid subscription if the slot ALREADY EXISTS at the moment it reads
 * it — `undefined?.get()` short-circuits and reads NOTHING, so if
 * `chromeVm` happens to evaluate before the `<For>` has mounted
 * this id's `<Block>` (which is what actually calls
 * `keepAliveNodeModelFor` and creates the slot), `chromeVm` latches
 * `null` FOREVER: nothing it read was reactive, so nothing can ever
 * wake it to try again, even once the real Block mounts moments later
 * and populates the slot. Reproduced live: `hoisted`/`keepAlive` both
 * true, `stackBlockIds` correctly containing the blockId, yet the pane
 * rendered permanently blank — not a data/layout-corruption issue (the
 * block's own data was fully intact), a pure ordering race in this
 * file. Calling `viewModelSlotFor` here guarantees the slot (and its
 * signal) exists BEFORE `chromeVm` ever gets a chance to read it —
 * `chromeNodeModel()` is called synchronously from inside `chromeVm`'s
 * own computation, so this always runs first on the same tick,
 * regardless of whether the `<For>` has mounted this id's Block yet.
 *
 * Now used on both paths (see `createViewModelScopes`): the slot
 * is created inside the accessor itself, so a read always subscribes to a
 * real signal, whichever path and whatever the mount order.
 */
export function useChromeNodeModel(
    nodeModel: NodeModel,
    activeBlockId: () => string,
    scopes: ViewModelScopes,
): Accessor<NodeModel> {
    return createMemo<NodeModel>(() => ({
        ...nodeModel,
        activeViewModel: () => scopes.viewModelSlotFor(activeBlockId()).get(),
    }));
}

/**
 * `NodeModel.activeViewModel()` genuinely blips through `null` on
 * EVERY switch, not just the first hoist (confirmed by an empirical repro, #3187).
 * `block.tsx`'s `onCleanup`
 * (clearing the OLD member's vm) runs synchronously during the inner
 * `<Key>`'s reconciliation; the `createEffect` that sets the NEW
 * member's vm is deferred to the next effects flush. A naive
 * `<Show when={nodeModel.activeViewModel()}>` callback form looks safe
 * (`<Show>` only re-runs its child function on a falsy->truthy edge),
 * but that real null blip IS such an edge every time, so it would
 * re-invoke `renderPaneChrome` — mounting a BRAND NEW `AgentPaneChrome`
 * on every switch and reproducing the exact flash this file exists to
 * eliminate. Fixed by latching the FIRST non-null vm ever observed
 * into a plain closure variable: once captured, this memo's OWN return
 * value stops changing (same object reference) regardless of how
 * `activeViewModel()` fluctuates underneath, so Solid's default `===`
 * memo comparison means no consumer — including the `<Show>` — ever
 * observes a falsy value again after the first real capture. Renders
 * via whichever vm happened to be active at that FIRST hoist and never
 * re-derives afterward — the same "frozen after first hoist" identity
 * `anchorBlockId` (agent-model.ts's own `renderPaneChrome` closure)
 * already assumes. Reads through `chromeNodeModel()` (not the raw
 * `nodeModel` prop) so the keep-alive path latches onto the per-id
 * lookup's current value rather than the shared (and, under keep-alive,
 * unreliable) leaf-level slot — doesn't change what gets latched for
 * the non-keep-alive path, where `chromeNodeModel()` IS `nodeModel`.
 */
export function useLatchedChromeVm(chromeNodeModel: Accessor<NodeModel>): Accessor<ViewModel | null> {
    let latchedChromeVm: ViewModel | null = null;
    return createMemo(() => {
        if (!latchedChromeVm) {
            latchedChromeVm = chromeNodeModel().activeViewModel?.() ?? null;
        }
        return latchedChromeVm;
    });
}
