// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import {
    BlockComponentModel2,
    BlockNodeModel,
    BlockProps,
    FullBlockProps,
} from "@/app/block/blocktypes";
import { getBlockViewClass } from "@/app/block/block-registry";
import { invokeCommand } from "@/app/platform/ipc";
import { BrainSpinner } from "@/app/element/BrainSpinner";
import { ErrorBoundary } from "@/element/errorboundary";
import { CenteredDiv } from "@/element/quickelems";
import { NodeModel, useDebouncedNodeInnerRect } from "@/layout/index";
import {
    atoms,
    counterInc,
    getBlockComponentModel,
    registerBlockComponentModel,
    unregisterBlockComponentModel,
} from "@/store/global";
import { getWaveObjectAtom, makeORef, useWaveObjectValue } from "@/store/wos";
import { focusedBlockId } from "@/util/focusutil";
import { isBlank, useAtomValueSafe } from "@/util/util";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, Suspense } from "solid-js";
import "./block.scss";
import "./pane-size-badge.scss";
import { BlockErrorBoundary } from "./BlockErrorBoundary";
import { BlockFrame } from "./blockframe";
import { blockViewToIcon, blockViewToName } from "./blockutil";
import { useSubagentBackfillGate } from "@/app/view/agent/hooks/useSubagentBackfillGate";

// Matches BrainSpinner.scss's own `.is-fading` opacity transition duration —
// Block's ready()-gate cross-fade (below) reuses the same visual timing as
// every other BrainSpinner fade in this codebase (agent-view.tsx's loading
// overlay and picker-fade, AgentPicker's own overlay).
const READY_GATE_FADE_MS = 200;

function makeViewModel(blockId: string, blockView: string, nodeModel: NodeModel): ViewModel {
    // Migration shims:
    //   * v0.33.197: forge was folded into the agent pane; redirect old
    //     "forge" blocks to "agent" so they keep rendering.
    //   * Drone rename (SPEC_RENAME_WORKFLOWS_TO_DRONE_2026_05_18): the
    //     Workflows feature was renamed to Drone. Existing user panes
    //     persist `meta.view: "workflows"` in the block store; the v10
    //     SQLite migration moves the DAG tables but does NOT rewrite
    //     block metadata, so redirect at the view-dispatch layer instead.
    //
    // "identity" was previously redirected here too, but as of PR-F.2
    // (#748) Identity is once again a first-class pane — `view: "identity"`
    // resolves to IdentityPaneViewModel via block-registry.ts.
    //   * Armory rename (docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md):
    //     the Trust Center pane was renamed to Armory. Existing user panes
    //     persist `meta.view: "trust"`; this is a pure UI rename with no
    //     SQLite migration, so redirect at the view-dispatch layer here
    //     (same pattern as workflows→drone).
    let effectiveView = blockView;
    if (effectiveView === "forge") effectiveView = "agent";
    if (effectiveView === "workflows") effectiveView = "drone";
    if (effectiveView === "trust") effectiveView = "armory";
    const ctor = getBlockViewClass(effectiveView);
    if (ctor != null) {
        return new ctor(blockId, nodeModel as any);
    }
    return makeDefaultViewModel(blockId, effectiveView);
}

function getViewElem(
    blockId: string,
    blockRef: { current: HTMLDivElement | null },
    contentRef: { current: HTMLDivElement | null },
    blockView: string,
    viewModel: ViewModel
): JSX.Element {
    if (isBlank(blockView)) {
        return <CenteredDiv>No View</CenteredDiv>;
    }
    if (viewModel.viewComponent == null) {
        return <CenteredDiv>No View Component</CenteredDiv>;
    }
    const VC = viewModel.viewComponent;
    return <VC blockId={blockId} blockRef={blockRef} contentRef={contentRef} model={viewModel} />;
}

function makeDefaultViewModel(blockId: string, viewType: string): ViewModel {
    const blockDataAtom = getWaveObjectAtom<Block>(makeORef("block", blockId));
    let viewModel: ViewModel = {
        viewType: viewType,
        viewIcon: createMemo(() => {
            const blockData = blockDataAtom();
            return blockViewToIcon(blockData?.meta?.view);
        }),
        viewName: createMemo(() => {
            const blockData = blockDataAtom();
            return blockViewToName(blockData?.meta?.view);
        }),
        preIconButton: createMemo(() => null),
        endIconButtons: createMemo(() => null),
        viewComponent: null,
    };
    return viewModel;
}

function BlockPreview({ nodeModel, viewModel }: FullBlockProps): JSX.Element {
    const [blockData] = useWaveObjectValue<Block>(makeORef("block", nodeModel.blockId));
    if (!blockData()) {
        return null;
    }
    return (
        <BlockFrame
            nodeModel={nodeModel}
            preview={true}
            blockModel={null}
            viewModel={viewModel}
        />
    );
}

function BlockFull({ nodeModel, viewModel }: FullBlockProps): JSX.Element {
    counterInc("render-BlockFull");
    let focusElemRef: { current: HTMLInputElement | null } = { current: null };
    let blockRef: { current: HTMLDivElement | null } = { current: null };
    let contentRef: { current: HTMLDivElement | null } = { current: null };
    const [blockClicked, setBlockClicked] = createSignal(false);
    const [blockData] = useWaveObjectValue<Block>(makeORef("block", nodeModel.blockId));
    const isFocused = nodeModel.isFocused;
    const disablePointerEvents = nodeModel.disablePointerEvents;
    const innerRect = useDebouncedNodeInnerRect(nodeModel);
    const noPadding = useAtomValueSafe(viewModel.noPadding);

    // Track previous focus state to handle blockClicked
    const [blockContentOffset, setBlockContentOffset] = createSignal<Dimensions>(null);

    const blockContentStyle = createMemo<JSX.CSSProperties>(() => {
        const retVal: JSX.CSSProperties = {
            "pointer-events": disablePointerEvents() ? "none" : undefined,
        };
        const rect = innerRect();
        const offset = blockContentOffset();
        if (rect?.width && rect?.height && offset) {
            retVal.width = `calc(${rect.width} - ${offset.width}px)`;
            retVal.height = `calc(${rect.height} - ${offset.height}px)`;
        }
        return retVal;
    });

    const blockViewType = createMemo(() => blockData()?.meta?.view);
    const viewElem = createMemo(
        () => getViewElem(nodeModel.blockId, blockRef, contentRef, blockViewType(), viewModel)
    );

    const handleChildFocus = (event: FocusEvent) => {
        if (!isFocused()) {
            nodeModel.focusNode();
        }
        // Any DOM element gaining focus lives in the main window's
        // render widget (pane HWNDs are OS-level and never fire DOM
        // focus events). Tell the host to move Win32 keyboard focus
        // back to the main HWND — without this, a previously-clicked
        // browser pane keeps Win32 focus and subsequent keystrokes
        // keep routing there instead of the now-focused element.
        // Browser panes' address-bar onFocus handler fires the same
        // IPC; this widens the trigger to every non-pane block
        // (terminal, agent, editor, ...). Idempotent when
        // focus is already on main.
        //
        // Pass `window_label` so the backend targets THIS window's
        // main browser. Without it, `main_window_focus` always
        // reclaims focus to `label=main` (the first non-pane browser
        // in state.browsers) — clicking an input in window 2 would
        // steal focus back to window 1.
        const params = new URLSearchParams(window.location.search);
        const windowLabel = params.get("windowLabel") ?? "main";
        invokeCommand("main_window_focus", { window_label: windowLabel }).catch(() => {});
    };

    const setFocusTarget = () => {
        const ok = viewModel?.giveFocus?.();
        if (ok) {
            return;
        }
        focusElemRef.current?.focus({ preventScroll: true });
    };

    const setBlockClickedTrue = () => {
        setBlockClicked(true);
    };

    // Handle blockClicked -> focus logic
    onMount(() => {
        // Measure content offset once DOM is ready
        if (blockRef.current && contentRef.current) {
            const blockRect = blockRef.current.getBoundingClientRect();
            const contentRect = contentRef.current.getBoundingClientRect();
            setBlockContentOffset({
                top: 0,
                left: 0,
                width: blockRect.width - contentRect.width,
                height: blockRect.height - contentRect.height,
            });
        }
    });

    // Watch isFocused to handle setBlockClicked
    // In SolidJS we use createEffect for reactive side effects, but here we just handle
    // the click in the onClick handler directly
    const handleBlockClick = () => {
        setBlockClicked(true);
        const focusWithin = focusedBlockId() == nodeModel.blockId;
        if (!focusWithin) {
            setFocusTarget();
        }
        if (!isFocused()) {
            nodeModel.focusNode();
        }
    };

    const blockModel: BlockComponentModel2 = {
        onClick: handleBlockClick,
        onFocusCapture: handleChildFocus,
        blockRef: blockRef,
    };

    return (
        <BlockFrame
            nodeModel={nodeModel}
            preview={false}
            blockModel={blockModel}
            viewModel={viewModel}
        >
            <div class="block-focuselem">
                <input
                    type="text"
                    value=""
                    ref={(el) => { focusElemRef.current = el; }}
                    id={`${nodeModel.blockId}-dummy-focus`}
                    class="dummy-focus"
                    onInput={() => {}}
                />
            </div>
            <div
                class={clsx("block-content", { "block-no-padding": noPadding })}
                ref={(el) => { contentRef.current = el; }}
                style={blockContentStyle()}
            >
                <ErrorBoundary>
                    <Suspense fallback={<BrainSpinner />}>{viewElem()}</Suspense>
                </ErrorBoundary>
            </div>
        </BlockFrame>
    );
}

function Block(props: BlockProps): JSX.Element {
    counterInc("render-Block");
    counterInc("render-Block-" + props.nodeModel?.blockId?.substring(0, 8));
    const [blockData, loading] = useWaveObjectValue<Block>(makeORef("block", props.nodeModel.blockId));

    // Track only the view type (not the full blockData) so the effect only re-runs
    // when the view changes (e.g. "Replace With..."), not on every meta update.
    // This prevents Solid.js from disposing ViewModel createMemo computations
    // that are owned by the effect when unrelated meta fields change.
    const viewType = createMemo(() => blockData()?.meta?.view);
    // Whether this block has a persisted session id (`agent:sessionid`) --
    // i.e. whether `scan_session_subagents` (agentmux-srv/src/server/reactive.rs)
    // WILL definitely run for this block, not just whether it happens to have
    // run yet. `useSubagentBackfillGate` needs this precise distinction — see
    // its own doc comment (reagentx P0, PR #2781 round 4) for why inferring it
    // from an empty history read alone is unsound.
    const hasPersistedSession = createMemo(() => !!blockData()?.meta?.["agent:sessionid"]);
    const [viewModel, setViewModel] = createSignal<ViewModel>(null);

    // Ownership tracking (SPEC_DRAG_SESSION_ARCHITECTURE_REFACTOR §3.4):
    // remember exactly which bcm THIS mount registered and which ViewModels
    // it created, so cleanup can neither clobber a newer mount's registration
    // nor dispose a ViewModel it merely adopted from the registry.
    let registeredBcm: BlockComponentModel | null = null;
    const createdViewModels: ViewModel[] = [];
    createEffect(() => {
        const view = viewType();
        if (!view) return;
        const bcm = getBlockComponentModel(props.nodeModel.blockId);
        let vm = bcm?.viewModel;
        if (vm == null || vm.viewType !== view) {
            vm = makeViewModel(props.nodeModel.blockId, view, props.nodeModel);
            createdViewModels.push(vm);
            registeredBcm = { viewModel: vm };
            registerBlockComponentModel(props.nodeModel.blockId, registeredBcm);
        }
        setViewModel(vm);
    });

    onCleanup(() => {
        if (registeredBcm) {
            unregisterBlockComponentModel(props.nodeModel.blockId, registeredBcm);
        }
        // Dispose only ViewModels this mount CREATED and that are not the
        // registry's live one (a newer mount may have adopted nothing from
        // us, but never dispose someone else's live vm out from under them).
        const liveVm = getBlockComponentModel(props.nodeModel.blockId)?.viewModel;
        for (const vm of createdViewModels) {
            if (vm !== liveVm) vm?.dispose?.();
        }
    });

    const ready = createMemo(() => !loading() && !isBlank(props.nodeModel.blockId) && blockData() != null && viewModel() != null);

    // reagentx P0 (PR #2781, round 5): a DEADLOCK, not a flip-flop. Folding
    // `subagentBackfillSettled()` directly into `ready()` above (round 4)
    // meant `<Show when={ready()}>` (below) would never mount `BlockFull`
    // at all for a persisted-session agent block — but `BlockFull` /
    // `AgentPresentationView` mounting is the ONLY thing that ever calls
    // `registerAgent` (agent-view.tsx), which is the ONLY thing that ever
    // triggers `scan_session_subagents` (the ONLY source of the
    // "started"/"done" this gate waits for). `ready()` needed
    // `subagentBackfillSettled()` to become true; `subagentBackfillSettled()`
    // needed `ready()` to become true first. Fixed by decoupling: `ready()`
    // (content mounting, unchanged from before this whole feature) no
    // longer depends on backfill status at all — the real content mounts
    // exactly as it always did, which is what lets it register and
    // actually trigger the backfill. Only the SPINNER OVERLAY's own
    // visibility (below) additionally waits on backfill settling, sitting
    // on top of the now-already-mounted (registering, backfilling) content
    // underneath — matching the retro's own framing ("the pulsing brain
    // until everything is ready") literally: a brain covering already-live
    // content, not a gate blocking that content from existing at all.
    const subagentBackfillSettled = useSubagentBackfillGate(props.nodeModel.blockId, viewType, hasPersistedSession);
    const spinnerReady = createMemo(() => ready() && subagentBackfillSettled());

    // Cross-fade the ready()-gate BrainSpinner out on top of the real
    // content instead of an instant hard cut (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md
    // §2.3/§4 Option B) — this is generic over every block type (agent,
    // terminal, browser, ...), unlike agent-view.tsx's picker-fade which is
    // agent-specific.
    //
    // An earlier version seeded `spinnerVisible` from `!ready()` read once
    // at construction (synchronous), then relied on `on(ready, ...,
    // {defer: true})`'s first (swallowed) run to treat "already ready" as
    // "nothing to do." Those are two DIFFERENT reads of `ready()` taken at
    // two different times — construction vs. the effect's first flush a
    // render pass later — so a block whose data resolves in that gap (the
    // common case for an already-warm cache) got seeded `true` and then
    // never corrected: `defer` skips the one call that would have set it
    // to `false`, and since `ready()` never changes again, nothing else
    // ever runs it either. The spinner stayed mounted at full opacity
    // indefinitely (see docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md).
    // Fixed by making the first observation of `ready()` and the seed the
    // same read, inside the same effect — no gap for them to disagree in.
    const [spinnerVisible, setSpinnerVisible] = createSignal(true);
    const [spinnerFading, setSpinnerFading] = createSignal(false);
    let spinnerFadeRaf: number | undefined;
    let spinnerFadeTimeout: ReturnType<typeof setTimeout> | undefined;
    let spinnerGateInitialized = false;
    onCleanup(() => {
        if (spinnerFadeRaf !== undefined) cancelAnimationFrame(spinnerFadeRaf);
        clearTimeout(spinnerFadeTimeout);
    });
    createEffect(() => {
        const isReady = spinnerReady();
        if (spinnerFadeRaf !== undefined) cancelAnimationFrame(spinnerFadeRaf);
        clearTimeout(spinnerFadeTimeout);
        if (!spinnerGateInitialized) {
            // First observation of `ready()` for this mount: reflect it
            // directly, no fade — there's nothing painted yet to fade
            // from either way.
            spinnerGateInitialized = true;
            setSpinnerVisible(!isReady);
            setSpinnerFading(false);
            return;
        }
        if (isReady) {
            if (!spinnerVisible()) return; // already past the transition
            // One rAF so the spinner paints at full opacity at least once
            // before the fade starts — flipping straight to the
            // "is-fading" class in this same tick would apply opacity:0
            // on the very first paint, with nothing to visibly transition
            // from.
            spinnerFadeRaf = requestAnimationFrame(() => setSpinnerFading(true));
            spinnerFadeTimeout = setTimeout(() => {
                setSpinnerVisible(false);
                setSpinnerFading(false);
            }, READY_GATE_FADE_MS);
        } else {
            // Not ready anymore (e.g. the view type changed out from
            // under this block) — show the spinner again immediately, no
            // fade needed going this direction.
            setSpinnerFading(false);
            setSpinnerVisible(true);
        }
    });

    // Per-block ErrorBoundary: a renderer fault in this pane only blanks
    // THIS pane, not the whole tab. See retro
    // docs/retro/retro-agent-pane-cascade-replacechild-2026-05-23.md.
    // The fallback reads only from props passed in (blockId, viewType,
    // error, reset, onClose) — never touches the broken pane's reactive
    // graph, which may be half-flushed.
    const viewTypeStr = createMemo(() => blockData()?.meta?.view);
    return (
        <>
            <Show when={ready()}>
                <BlockErrorBoundary
                    blockId={props.nodeModel.blockId}
                    viewType={viewTypeStr()}
                    onClose={props.nodeModel.onClose}
                >
                    {props.preview
                        ? <BlockPreview nodeModel={props.nodeModel} viewModel={viewModel()} preview={props.preview} />
                        : <BlockFull nodeModel={props.nodeModel} viewModel={viewModel()} preview={props.preview} />
                    }
                </BlockErrorBoundary>
            </Show>
            {/* Applied the instant ready() flips true (same render as the
                real content appearing) so this never sits in normal flow
                alongside it, even for one frame — see is-overlay's
                counterpart in agent-view.tsx's picker-fade. */}
            <Show when={spinnerVisible()}>
                <div
                    class="block-ready-gate-host"
                    classList={{
                        "is-overlay": ready(),
                        "is-fading": spinnerFading(),
                        "is-reduced-motion": atoms.prefersReducedMotionAtom(),
                    }}
                >
                    <BrainSpinner fading={spinnerFading()} />
                </div>
            </Show>
        </>
    );
}

export { Block };
