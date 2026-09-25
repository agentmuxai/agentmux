// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Block, resolveEffectiveViewType } from "@/app/block/block";
import { setKeepAliveBlockDormant } from "@/app/store/block-component-registry";
import { MOS } from "@/app/store/global";
import type { NodeModel } from "@/layout/index";
import { findNode } from "@/layout/lib/layoutNode";
import { Key } from "@solid-primitives/keyed";
import { createEffect, createMemo, createSignal, For, onCleanup, Show, type JSX } from "solid-js";

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
// Universal Pane Tabs (SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md
// §5, Task Group C): EVERY view type here registers the same shared
// `renderPaneChromeShell` (PaneChrome.tsx) via a one-line
// `this.renderPaneChrome =` field in its own ViewModel. There is no
// per-type chrome component any more — agent and term used to have their
// own (AgentPaneChrome/TermPaneChrome) and now express what was special
// about them through the optional `paneChromeModel` capability hook
// instead (custom.d.ts's `PaneChromeModel`).
// "cpuplot" is sysinfo's own secondary registered view key
// (block-registry.ts), same ViewModel class as "sysinfo".
const HOISTS_OWN_CHROME = new Set([
    "agent",
    "term",
    "browser",
    "editor",
    "sysinfo",
    "cpuplot",
    "swarm",
    "armory",
    "media",
    "drone",
    "help",
    "warden",
]);

/**
 * Subset of `HOISTS_OWN_CHROME` whose stack members stay mounted
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
 * `"agent"` added per SPEC_AGENT_PANE_TAB_KEEPALIVE_2026_09_18.md, after an
 * explicit audit of the entangled per-tab state this set's own comment used
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
 */
const KEEP_ALIVE_TYPES = new Set(["term", "agent"]);

export function PaneLeafChrome(props: { nodeModel: NodeModel }): JSX.Element {
    const nodeModel = props.nodeModel;
    const activeBlockId = () => nodeModel.activeBlockId?.() ?? nodeModel.blockId;

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


    // --- Per-tab ViewModel slots (both paths) ---
    //
    // Chrome must read the ViewModel of the tab that is active NOW. The leaf's
    // own `activeViewModel` is one shared, last-writer-wins slot, and it is
    // wrong on BOTH paths:
    // - Without keep-alive, a switch updates `activeBlockId()` at once, but the
    //   old tab's ViewModel stays in the shared slot until its <Block>
    //   unmounts, and the new one only lands in a later effect. Anything that
    //   read both in between (PaneChrome's per-pill label/favicon memory) saw
    //   tab X's ViewModel while tab Y was active, and saved X's title/favicon
    //   under Y: pills showing another tab's name, icon, or a browser's
    //   favicon (confirmed live with a diagnostic, 2026-09-24).
    // - Under keep-alive (below), every member mounts once, so the shared slot
    //   holds whichever mounted last.
    // So every <Block> writes its ViewModel into a slot keyed by its OWN
    // blockId, and chrome reads the slot of `activeBlockId()`. During a switch
    // the new tab's slot is briefly empty (null), never another tab's.
    //
    // Under keep-alive, EVERY stack member's <Block> mounts once and stays
    // mounted — so `nodeModel.setActiveViewModel`/`activeViewModel` (a
    // single shared slot, last-writer-wins) can no longer be trusted: every
    // kept-alive Block calls `setActiveViewModel` once at its OWN mount, not
    // on every switch, so the shared slot would end up holding whichever
    // tab happened to mount MOST RECENTLY — not necessarily the one
    // currently visible. Terminal's own `runtimeLabel` and the
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
                // The live `hoisted` memo, same as `scopedNodeModel` above
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
    // title entirely.
    //
    // The `hoisted` MEMO ITSELF, not `hoisted()`'s current value — see
    // `BlockNodeModel.paneChromeHoisted`'s own doc comment (blocktypes.ts)
    // for why a frozen snapshot isn't safe here: block.tsx's ViewModel
    // registry can ADOPT an existing ViewModel (skipping construction)
    // for a later wrapper whose snapshot would've read differently.
    // Forwarding the live memo means every wrapper built for this leaf
    // — regardless of which one a ViewModel instance ends up holding —
    // reports the SAME, always-current value.
    //
    // Used only when !keepAlive() — the single-active-member path below,
    // unchanged from before KEEP_ALIVE_TYPES existed.
    const scopedNodeModel = createMemo<NodeModel>(() => {
        const id = activeBlockId();
        const slot = viewModelSlotFor(id);
        return {
            ...nodeModel,
            blockId: id,
            paneChromeHoisted: hoisted,
            // Also record the ViewModel in THIS block's own slot (see
            // `viewModelSlotFor` below), so chrome reads the active tab's
            // ViewModel by id instead of the leaf's shared last-writer-wins
            // slot. Still forwarded to the leaf's own setter, which other
            // consumers of the raw NodeModel keep reading exactly as before.
            setActiveViewModel: (vm: ViewModel | null, owner: object) => {
                slot.set(vm, owner);
                nodeModel.setActiveViewModel?.(vm, owner);
            },
        } as NodeModel;
    });

    // The full stack, reactive — same `localTreeStateAtom()` + live
    // `findNode` lookup pattern term.tsx's own `termTabs` and agent-view.tsx's
    // `stackTabs` already use, falling back to a single-entry list when this
    // leaf hasn't split into a stack yet. `nodeModel.layoutModel`, NOT
    // `getLayoutModelForStaticTab()` — see that field's own doc comment
    // (layout/lib/types.ts) for why the global "active tab" lookup is wrong
    // here (SPEC_PANE_CHROME_LAYOUT_MODEL_TAB_BINDING_2026_09_18.md).
    const layoutModel = nodeModel.layoutModel;
    const stackBlockIds = createMemo<string[]>(() => {
        layoutModel.localTreeStateAtom();
        const node = findNode(layoutModel.treeState.rootNode, nodeModel.nodeId);
        const stack = node?.data?.blockStack?.length ? node.data.blockStack : [activeBlockId()];
        // Drop cache/slot entries for blockIds no longer in the stack (tab
        // closed or moved away) so a gone tab's ViewModel/NodeModel wrapper
        // don't linger for the rest of the pane's lifetime.
        const live = new Set(stack);
        for (const id of keepAliveNodeModelCache.keys()) {
            if (!live.has(id)) keepAliveNodeModelCache.delete(id);
        }
        for (const id of viewModelSlots.keys()) {
            if (!live.has(id)) viewModelSlots.delete(id);
        }
        return stack;
    });

    // The NodeModel chrome itself renders with — plain passthrough when not
    // keeping tabs alive (unchanged), otherwise `activeViewModel` is
    // overridden to the per-id lookup above so a switch re-points the read
    // at the newly-active tab's own slot instead of following whichever
    // Block mounted last.
    //
    // `viewModelSlotFor(activeBlockId())` is called here, EAGERLY, not just
    // inside `keepAliveNodeModelFor` — closes a real race (found live, via
    // a diagnostic pass reproducing a permanently-blank pane): a slot is a
    // lazily-created `createSignal`, so `chromeVm` below (which subscribes
    // via `viewModelSlots.get(id)?.get()`) only actually establishes a
    // Solid subscription if the slot ALREADY EXISTS at the moment it reads
    // it — `undefined?.get()` short-circuits and reads NOTHING, so if
    // `chromeVm` happens to evaluate before the `<For>` below has mounted
    // this id's `<Block>` (which is what actually calls
    // `keepAliveNodeModelFor` and creates the slot), `chromeVm` latches
    // `null` FOREVER: nothing it read was reactive, so nothing can ever
    // wake it to try again, even once the real Block mounts moments later
    // and populates the slot. Reproduced live: `hoisted`/`keepAlive` both
    // true, `stackBlockIds` correctly containing the blockId, yet the pane
    // rendered permanently blank — not a data/layout-corruption issue (the
    // block's own data was fully intact), a pure ordering race in this
    // file. Calling `viewModelSlotFor` here guarantees the slot (and its
    // signal) exists BEFORE `chromeVm` ever gets a chance to read it —
    // `chromeNodeModel()` is called synchronously from inside `chromeVm`'s
    // own computation, so this always runs first on the same tick,
    // regardless of whether the `<For>` has mounted this id's Block yet.
    //
    // Now used on both paths (see "Per-tab ViewModel slots" above): the slot
    // is created inside the accessor itself, so a read always subscribes to a
    // real signal, whichever path and whatever the mount order.
    const chromeNodeModel = createMemo<NodeModel>(() => ({
        ...nodeModel,
        activeViewModel: () => viewModelSlotFor(activeBlockId()).get(),
    }));

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
                {(id) => {
                    // Codex P1/P2 + ReAgent P1 on PR #3187: every "all
                    // panes" consumer of the block-component registry
                    // (multi-input broadcast, all-panes zoom, the
                    // multi-input-eligible terminal count) assumes one
                    // registry entry per currently-VISIBLE pane. Mark this
                    // member dormant whenever it isn't the active one, so
                    // those consumers keep seeing exactly what they did
                    // before keep-alive existed — see
                    // block-component-registry.ts's own comment for why
                    // this is tracked explicitly here rather than
                    // re-derived from a layout/tab lookup. Cleared on this
                    // row's own unmount (the id left the stack — tab
                    // closed), not just flipped to false, so a closed tab
                    // can never linger as a phantom dormant entry.
                    createEffect(() => {
                        setKeepAliveBlockDormant(id, id !== activeBlockId());
                    });
                    onCleanup(() => setKeepAliveBlockDormant(id, false));

                    return (
                        <div
                            class="pane-leaf-keepalive-slot"
                            style={{
                                position: "absolute",
                                inset: "0",
                                // `inherit`, not `visible`: an explicit `visible`
                                // would show through a hidden window tab kept laid out
                                // with `visibility: hidden` (workspace.tsx).
                                visibility: id === activeBlockId() ? "inherit" : "hidden",
                                "pointer-events": id === activeBlockId() ? "auto" : "none",
                            }}
                        >
                            <Block nodeModel={keepAliveNodeModelFor(id)} preview={false} />
                        </div>
                    );
                }}
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

    // What the chrome itself holds for the pane's whole life. Chrome renders
    // ONCE, so handing it a `chromeNodeModel()` snapshot froze whichever
    // variant was current at first hoist: a pane that started as a
    // non-keep-alive type (Swarm) got the leaf's own NodeModel, then an
    // agent tab latched keep-alive on and every member began reporting its
    // vm to its own per-id slot instead — so the chrome's
    // `activeViewModel()` stopped following the active tab, and the busy
    // ring's slot never reached the agent
    // (RETRO_AGENT_PANE_BUSY_RING_MISSING_IN_NON_AGENT_FIRST_STACK_2026_09_25.md).
    // `activeViewModel` delegates through the memo instead, so it follows
    // the keep-alive switch too.
    const paneChromeNodeModel: NodeModel = {
        ...nodeModel,
        activeViewModel: () => chromeNodeModel().activeViewModel?.() ?? null,
    };

    return (
        <Show when={hoisted()} fallback={content}>
            {/* Guards only the leaf's own first-hoist window, before any
                ViewModel has been observed yet — does NOT itself unmount
                the outer chrome once mounted, since `chromeVm()` (above)
                never returns to a falsy value after its first real
                capture. */}
            <Show when={chromeVm()}>
                {(vm) => vm().renderPaneChrome!(paneChromeNodeModel, content)}
            </Show>
        </Show>
    );
}
