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
import { PaneLoadingCover } from "@/app/element/PaneLoadingCover";
import { createPaneReadiness, type PaneReadinessPhase } from "@/app/store/pane-readiness";
import { ErrorBoundary } from "@/element/errorboundary";
import { CenteredDiv } from "@/element/quickelems";
import { NodeModel, useDebouncedNodeInnerRect } from "@/layout/index";
import {
    counterInc,
    getBlockComponentModel,
    registerBlockComponentModel,
    unregisterBlockComponentModel,
} from "@/store/global";
import { getMuxObjectAtom, makeORef, useMuxObjectValue } from "@/store/mos";
import { giveBlockFocus } from "@/app/store/focusManager";
import { focusedBlockId } from "@/util/focusutil";
import { isBlank, useAtomValueSafe } from "@/util/util";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createRoot, createSignal, onCleanup, onMount, Show, Suspense, untrack } from "solid-js";
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

/**
 * Applies the view-type migration/rename redirects below. Exported so
 * `pane-leaf-chrome.tsx` can apply the identical rules when deciding
 * whether a stack member's EFFECTIVE view is "agent" — e.g. a still-live
 * "forge" block must route through the same redirect `makeViewModel`
 * itself applies, not just a raw `meta.view === "agent"` check.
 */
export function resolveEffectiveViewType(blockView: string): string {
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
    return effectiveView;
}

// Each ViewModel's own reactive root, disposed with it (`disposeViewModel`).
const viewModelRoots = new WeakMap<ViewModel, () => void>();

/**
 * Builds a ViewModel in its OWN reactive root. It is called from inside
 * `Block`'s effect, and a constructor's memos/effects used to belong to that
 * effect run, while its reads (block meta, config) subscribed the effect to
 * them. The first meta change then re-ran the effect, which disposed the run
 * and with it every memo of the cached, reused ViewModel: Sysinfo's plot type
 * changed once and then froze. `createRoot` gives the constructor an owner
 * that lives as long as the ViewModel, and runs it untracked.
 * REPORT_SYSINFO_PLOT_TYPE_AND_BROWSER_PREVIEW_VM_2026_09_25.md §3.
 */
function makeViewModel(blockId: string, blockView: string, nodeModel: NodeModel): ViewModel {
    const effectiveView = resolveEffectiveViewType(blockView);
    const ctor = getBlockViewClass(effectiveView);
    let disposeRoot!: () => void;
    const vm = createRoot((dispose) => {
        disposeRoot = dispose;
        return ctor != null
            ? (new ctor(blockId, nodeModel as any) as ViewModel)
            : makeDefaultViewModel(blockId, effectiveView);
    });
    viewModelRoots.set(vm, disposeRoot);
    return vm;
}

function disposeViewModel(vm: ViewModel): void {
    vm.dispose?.();
    viewModelRoots.get(vm)?.();
    viewModelRoots.delete(vm);
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
    const blockDataAtom = getMuxObjectAtom<Block>(makeORef("block", blockId));
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
    const [blockData] = useMuxObjectValue<Block>(makeORef("block", nodeModel.blockId));
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

function BlockFull({ nodeModel, viewModel, covered }: FullBlockProps): JSX.Element {
    counterInc("render-BlockFull");
    let blockRef: { current: HTMLDivElement | null } = { current: null };
    let contentRef: { current: HTMLDivElement | null } = { current: null };
    const [blockClicked, setBlockClicked] = createSignal(false);
    const [blockData] = useMuxObjectValue<Block>(makeORef("block", nodeModel.blockId));
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

    // Same routine every pane-selection path uses — see giveBlockFocus().
    const setFocusTarget = () => giveBlockFocus(nodeModel.blockId);

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
                    {/* A view that suspends AFTER the pane is live (a
                        `createResource` refetch, say) still needs something on
                        screen — this is the one loading affordance that is NOT
                        about initial assembly, so it is not folded into the
                        cover. But while the pane IS still assembling the cover
                        is already up over this exact box, and rendering a
                        spinner underneath it is the "two brains at once" case
                        the consolidation exists to remove, so suppress it for
                        that window. See SPEC_PANE_LOADING_CONSOLIDATION §5.3. */}
                    <Suspense fallback={<Show when={!covered?.()}><BrainSpinner /></Show>}>
                        {viewElem()}
                    </Suspense>
                </ErrorBoundary>
            </div>
        </BlockFrame>
    );
}

function Block(props: BlockProps): JSX.Element {
    counterInc("render-Block");
    counterInc("render-Block-" + props.nodeModel?.blockId?.substring(0, 8));
    const [blockData, loading] = useMuxObjectValue<Block>(makeORef("block", props.nodeModel.blockId));

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
    // A preview mount's own ViewModel, kept across effect re-runs so the
    // effect does not strand a live one on every run (#3482 — see the preview
    // branch below). Rebuilt only when the view type actually changes.
    let previewViewModel: ViewModel | null = null;
    let previewViewType: string | null = null;
    createEffect(() => {
        const view = viewType();
        if (!view) return;
        // A drag-preview thumbnail (`tabcontent.tsx`'s `renderPreview`,
        // `<Block preview>` with the RAW leaf nodeModel) renders through
        // this SAME component but must never share a ViewModel instance —
        // or the leaf's `activeViewModel` signal — with the real,
        // non-preview mount of the same blockId. Two confirmed collisions
        // when it did:
        // 1. `getBlockComponentModel`/`registerBlockComponentModel` are
        //    keyed by blockId ALONE, with no preview/real distinction, and
        //    adoption below is last-writer-wins. If the preview mount's
        //    effect happened to run first, the real mount adopted ITS
        //    ViewModel — one whose cached `nodeModel` (the raw leaf model)
        //    has no `paneChromeHoisted` at all, silently and permanently
        //    breaking `noHeader()` for the real pane (reproduced live: two
        //    agent panes stuck with a double header, `noHeader()` frozen
        //    `false` because the adopted vm's `nodeModel` wasn't the
        //    correctly-hoisted wrapper). See custom.d.ts's own doc comment
        //    on `paneChromeHoisted` for the fix that made THIS symptom
        //    possible to diagnose (a live accessor, not a snapshot) — the
        //    accessor was never the bug; sharing the vm across mounts was.
        // 2. Worse: `setActiveViewModel` below has NO owner-check on its
        //    SET path (only its CLEAR path does — see
        //    layoutNodeModels.ts) — for every non-keep-alive hoisted type,
        //    the preview mount's raw leaf nodeModel and the real content
        //    mount's `scopedNodeModel` wrapper share the exact SAME
        //    underlying signal, so whichever mount's effect runs LAST
        //    simply overwrites the other's ViewModel in that signal — not
        //    just a broken header, chrome could render the PREVIEW's own
        //    ViewModel instance for the real, visible pane.
        // Fixed by never letting a preview mount touch either shared
        // surface: it gets its own private ViewModel, created fresh here,
        // disposed on its own unmount, and never registered or published
        // anywhere another mount could adopt or overwrite.
        if (props.preview) {
            // REUSE across effect re-runs rather than rebuilding. This effect
            // re-runs while the preview stays mounted — a block's meta
            // changing is enough — and rebuilding here stranded the previous
            // ViewModel alive, since the component-level onCleanup below only
            // ever disposes the last one. For an editor preview every
            // stranded instance keeps a live `editor:file_changed`
            // subscription, so a pane open during file churn accumulated tens
            // of thousands of them and took the renderer down (#3482,
            // confirmed by the leak guard's stack pointing exactly here).
            //
            // Deliberately fixed by NOT creating the extra ViewModel, rather
            // than by disposing it on re-run: `dispose()` is not safe to call
            // for a preview. A preview shares its blockId with the real mount
            // (tabcontent.tsx's renderPreview passes the same leaf nodeModel),
            // and some view models additionally register themselves in their
            // OWN blockId-keyed global store — `EditorViewModel.dispose()`
            // calls `unregisterEditorPane(this.blockId)`, which would delete
            // the slot the still-live real editor pane depends on, making its
            // next dispatch throw "dispatch for unregistered pane". The
            // surrounding comment's "never registered, never published" only
            // ever held for the BCM registry, not for those. Caught by
            // reagentx P1 on PR #3483.
            if (previewViewModel == null || previewViewType !== view) {
                previewViewModel = makeViewModel(props.nodeModel.blockId, view, props.nodeModel);
                previewViewType = view;
                createdViewModels.push(previewViewModel);
            }
            setViewModel(previewViewModel);
            return;
        }
        const bcm = getBlockComponentModel(props.nodeModel.blockId);
        let vm = bcm?.viewModel;
        if (vm == null || vm.viewType !== view) {
            vm = makeViewModel(props.nodeModel.blockId, view, props.nodeModel);
            createdViewModels.push(vm);
            registeredBcm = { viewModel: vm };
            registerBlockComponentModel(props.nodeModel.blockId, registeredBcm);
        }
        setViewModel(vm);
        // See NodeModel.activeViewModel/setActiveViewModel's own doc
        // comments (layout/lib/types.ts) — hoisted pane chrome's only live
        // pointer to a callable ViewModel, since the global registry
        // doesn't survive this component's own dispose-on-unmount. Owner
        // matches the SAME object identity `unregisterBlockComponentModel`
        // below would be called with, so an adopting (not creating) mount's
        // eventual cleanup correctly no-ops instead of clobbering the
        // creating mount's still-live registration.
        props.nodeModel.setActiveViewModel?.(vm, registeredBcm ?? bcm);
    });

    onCleanup(() => {
        // Preview mounts never touched either shared surface above — see
        // that branch's own comment — so there's nothing to unregister or
        // clear here for them, just their own private ViewModel(s) below.
        if (!props.preview) {
            // Owner is `registeredBcm` here too (not `registeredBcm ?? bcm`) —
            // an adopting mount never created its own registration object, so
            // this correctly no-ops for it instead of clearing the creating
            // mount's still-live activeViewModel out from under it.
            props.nodeModel.setActiveViewModel?.(null, registeredBcm);
            if (registeredBcm) {
                unregisterBlockComponentModel(props.nodeModel.blockId, registeredBcm);
            }
        }
        // Dispose only ViewModels this mount CREATED and that are not the
        // registry's live one (a newer mount may have adopted nothing from
        // us, but never dispose someone else's live vm out from under them).
        const liveVm = getBlockComponentModel(props.nodeModel.blockId)?.viewModel;
        for (const vm of createdViewModels) {
            if (vm && vm !== liveVm) disposeViewModel(vm);
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

    // ONE readiness authority for this pane (phase 3 of
    // SPEC_PANE_LOADING_CONSOLIDATION_2026_09_20.md). This block used to own a
    // hand-rolled visible/fading/unmount state machine of its own, a second one
    // lived in agent-view.tsx, and a third in AgentPicker.tsx — all rendering
    // their own spinner with their own timers, up to two of them measured on
    // screen at once. They are now the same controller and the same cover.
    //
    // This is a REVEAL gate, never a mount gate. `ready()` still decides
    // mounting entirely on its own and depends on nothing here — see the
    // deadlock note above, which is exactly what happens if that is blurred.
    const readiness = createPaneReadiness({ label: `block:${props.nodeModel.blockId.substring(0, 8)}` });
    const releaseMountGate = readiness.gate("content");
    createEffect(() => {
        if (ready()) releaseMountGate();
    });

    // `subagentBackfillSettled()` is deliberately NOT a gate, and that is the
    // whole point of this block of code.
    //
    // PaneReadiness is one-way by design: `assembling → revealing → live`, and
    // a gate registered after reveal is a no-op precisely so a late dependency
    // cannot yank the cover back over content the user is already reading.
    // But this signal is RE-ENTRANT — `useSubagentBackfillGate` calls
    // `setSettled(false)` on EVERY "started" event, including one arriving long
    // after the first cycle settled. Feeding it to a one-way gate compiles and
    // looks right, and silently drops every re-cover after the first: exactly
    // the behaviour the hook's own round-7 fix exists to provide. (reagent P1
    // on #3464.)
    //
    // So it drives its own small cycle, which the cover renders with the same
    // component. Same split as the `<Suspense>` fallback below and the browser
    // pane's post-first-paint badge (§6.1): one-time assembly is the
    // controller's job, mid-life re-covers are not.
    const [reCoverPhase, setReCoverPhase] = createSignal<PaneReadinessPhase>("live");
    let reCoverFade: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(reCoverFade));
    createEffect(() => {
        const settled = subagentBackfillSettled();
        // BOTH reads are tracked, deliberately. An earlier version read the
        // phase through `untrack`, so this only re-ran when the backfill signal
        // itself changed value — and the real hook initialises `settled` to
        // FALSE and leaves it there until the async backfill completes. The
        // ordinary case for a fast-resolving pane with a slow backfill is
        // therefore: settled is false and never changes, the pane reaches
        // `live`, and this effect never re-runs to notice, so the cover never
        // appears at all and the pane reveals mid-backfill — the exact Activity
        // Dock flicker rounds 2-8 of the hook exist to prevent. Nothing
        // corrected it later either: when `settled` finally flipped true the
        // effect early-returned, already believing itself live.
        // (reagent P1 on #3466.)
        //
        // While the pane is still assembling the initial cover is already up;
        // the controller owns the screen until it reaches `live`.
        if (readiness.phase() !== "live") return;
        clearTimeout(reCoverFade);
        if (!settled) {
            setReCoverPhase("assembling"); // re-covered, opaque, no fade in
            return;
        }
        if (untrack(reCoverPhase) === "live") return; // nothing to fade out
        setReCoverPhase("revealing");
        reCoverFade = setTimeout(() => setReCoverPhase("live"), READY_GATE_FADE_MS);
    });

    // One cover, two sources: the controller until it goes live, this block's
    // own re-cover cycle afterwards.
    const coverPhase = (): PaneReadinessPhase => {
        const phase = readiness.phase();
        return phase === "live" ? reCoverPhase() : phase;
    };

    // Cross-fade the cover out on top of the real content instead of an instant
    // hard cut (SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md §2.3/§4
    // Option B) — generic over every block type (agent, terminal, browser, ...),
    // unlike agent-view.tsx's picker-fade which is agent-specific.
    //
    // The first observation is special-cased, and the reason is a retro. An
    // earlier version seeded its spinner signal from `!ready()` read once at
    // construction, then relied on a deferred effect to correct it — two
    // DIFFERENT reads of `ready()` at two different times, so a block whose
    // data resolved in the gap (the common case for a warm cache) got seeded
    // "visible" and was never corrected, leaving the spinner up forever
    // (docs/retro/retro-block-ready-gate-spinner-stuck-visible-race-2026-08-23.md).
    // Here the phase starts at `assembling` unconditionally and only a gate
    // completion moves it, so there is no second read to disagree with — but a
    // block that is ALREADY ready on its first flush must still skip the fade
    // rather than flash an opaque cover over content that was never hidden.
    // Deliberately ONE effect reading the phase once. Splitting "was this the
    // first flush?" across two effects re-creates the retro's shape: two reads
    // at two times that can disagree.
    let firstFlush = true;
    let fadeTimeout: ReturnType<typeof setTimeout> | undefined;
    onCleanup(() => clearTimeout(fadeTimeout));
    createEffect(() => {
        const phase = readiness.phase();
        const wasFirstFlush = firstFlush;
        firstFlush = false;
        if (phase !== "revealing") return;
        clearTimeout(fadeTimeout);
        if (wasFirstFlush) {
            // Already revealed by the time this block first flushed: the gates
            // completed synchronously during setup (warm cache), so the cover
            // never painted and there is nothing to fade FROM. Go straight to
            // live, matching the pre-consolidation "reflect it directly, no
            // fade" first-observation behaviour.
            readiness.revealComplete();
            return;
        }
        fadeTimeout = setTimeout(() => readiness.revealComplete(), READY_GATE_FADE_MS);
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
                        : <BlockFull nodeModel={props.nodeModel} viewModel={viewModel()} preview={props.preview} covered={() => coverPhase() !== "live"} />
                    }
                </BlockErrorBoundary>
            </Show>
            {/* The same cover component agent-view.tsx and AgentPicker.tsx
                render, driven by the same controller — this block used to
                render a third, independent spinner here.

                `overlay` flips to true the instant ready() does (the same
                render as the real content appearing), so the cover never sits
                in normal flow alongside that content, even for one frame. */}
            <PaneLoadingCover phase={coverPhase} overlay={ready} />
        </>
    );
}

export { Block };
