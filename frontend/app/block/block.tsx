// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import {
    BlockComponentModel2,
    BlockNodeModel,
    BlockProps,
    FullBlockProps,
    FullSubBlockProps,
    SubBlockProps,
} from "@/app/block/blocktypes";
import { LauncherViewModel } from "@/app/view/launcher/launcher";
import { SysinfoViewModel } from "@/app/view/sysinfo/sysinfo";
import { AgentViewModel } from "@/app/view/agent";
import { ForgeViewModel } from "@/app/view/forge/forge";
import { SubagentViewModel } from "@/app/view/subagent/subagent";
import { ErrorBoundary } from "@/element/errorboundary";
import { CenteredDiv } from "@/element/quickelems";
import { NodeModel, useDebouncedNodeInnerRect } from "@/layout/index";
import {
    counterInc,
    getBlockComponentModel,
    registerBlockComponentModel,
    unregisterBlockComponentModel,
} from "@/store/global";
import { getWaveObjectAtom, makeORef, useWaveObjectValue } from "@/store/wos";
import { focusedBlockId, getElemAsStr } from "@/util/focusutil";
import { isBlank, useAtomValueSafe } from "@/util/util";
import { HelpViewModel } from "@/view/helpview/helpview";
import { TermViewModel } from "@/view/term/term";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, onCleanup, onMount, Show, Suspense } from "solid-js";
import "./block.scss";
import { BlockFrame } from "./blockframe";
import { blockViewToIcon, blockViewToName } from "./blockutil";

const BlockRegistry: Map<string, ViewModelClass> = new Map();
BlockRegistry.set("term", TermViewModel as any);
BlockRegistry.set("cpuplot", SysinfoViewModel as any);
BlockRegistry.set("sysinfo", SysinfoViewModel as any);
BlockRegistry.set("help", HelpViewModel as any);
BlockRegistry.set("launcher", LauncherViewModel as any);
BlockRegistry.set("agent", AgentViewModel as any);
BlockRegistry.set("forge", ForgeViewModel as any);
BlockRegistry.set("subagent", SubagentViewModel as any);

function makeViewModel(blockId: string, blockView: string, nodeModel: NodeModel): ViewModel {
    const ctor = BlockRegistry.get(blockView);
    if (ctor != null) {
        return new ctor(blockId, nodeModel as any);
    }
    return makeDefaultViewModel(blockId, blockView);
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

function BlockSubBlock({ nodeModel, viewModel }: FullSubBlockProps): JSX.Element {
    const [blockData] = useWaveObjectValue<Block>(makeORef("block", nodeModel.blockId));
    let blockRef: { current: HTMLDivElement | null } = { current: null };
    let contentRef: { current: HTMLDivElement | null } = { current: null };
    const blockViewType = createMemo(() => blockData()?.meta?.view);
    const viewElem = createMemo(
        () => getViewElem(nodeModel.blockId, blockRef, contentRef, blockViewType(), viewModel)
    );
    const noPadding = useAtomValueSafe(viewModel.noPadding);
    return (
        <Show when={blockData()}>
            <div class={clsx("block-content", { "block-no-padding": noPadding })} ref={(el) => { contentRef.current = el; }}>
                <ErrorBoundary>
                    <Suspense fallback={<CenteredDiv>Loading...</CenteredDiv>}>{viewElem()}</Suspense>
                </ErrorBoundary>
            </div>
        </Show>
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
        console.log("setFocusedChild", nodeModel.blockId, getElemAsStr(event.target));
        if (!isFocused()) {
            console.log("focusedChild focus", nodeModel.blockId);
            nodeModel.focusNode();
        }
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
                    <Suspense fallback={<CenteredDiv>Loading...</CenteredDiv>}>{viewElem()}</Suspense>
                </ErrorBoundary>
            </div>
        </BlockFrame>
    );
}

function Block(props: BlockProps): JSX.Element {
    counterInc("render-Block");
    counterInc("render-Block-" + props.nodeModel?.blockId?.substring(0, 8));
    const [blockData, loading] = useWaveObjectValue<Block>(makeORef("block", props.nodeModel.blockId));

    // Reactively create/update the viewModel when blockData loads or view type changes.
    // In SolidJS the component body runs once, so we use createEffect to handle async data.
    const [viewModel, setViewModel] = createSignal<ViewModel>(null);

    createEffect(() => {
        const bd = blockData();
        const view = bd?.meta?.view;
        if (!bd || !view) return;
        const bcm = getBlockComponentModel(props.nodeModel.blockId);
        let vm = bcm?.viewModel;
        if (vm == null || vm.viewType !== view) {
            vm = makeViewModel(props.nodeModel.blockId, view, props.nodeModel);
            registerBlockComponentModel(props.nodeModel.blockId, { viewModel: vm });
        }
        setViewModel(vm);
    });

    onCleanup(() => {
        unregisterBlockComponentModel(props.nodeModel.blockId);
        viewModel()?.dispose?.();
    });

    const ready = createMemo(() => !loading() && !isBlank(props.nodeModel.blockId) && blockData() != null && viewModel() != null);

    return (
        <Show when={ready()}>
            {props.preview
                ? <BlockPreview nodeModel={props.nodeModel} viewModel={viewModel()} preview={props.preview} />
                : <BlockFull nodeModel={props.nodeModel} viewModel={viewModel()} preview={props.preview} />
            }
        </Show>
    );
}

function SubBlock(props: SubBlockProps): JSX.Element {
    counterInc("render-Block");
    counterInc("render-Block-" + props.nodeModel?.blockId?.substring(0, 8));
    const [blockData, loading] = useWaveObjectValue<Block>(makeORef("block", props.nodeModel.blockId));

    const [viewModel, setViewModel] = createSignal<ViewModel>(null);

    createEffect(() => {
        const bd = blockData();
        const view = bd?.meta?.view;
        if (!bd || !view) return;
        const bcm = getBlockComponentModel(props.nodeModel.blockId);
        let vm = bcm?.viewModel;
        if (vm == null || vm.viewType !== view) {
            vm = makeViewModel(props.nodeModel.blockId, view, props.nodeModel as any);
            registerBlockComponentModel(props.nodeModel.blockId, { viewModel: vm });
        }
        setViewModel(vm);
    });

    onCleanup(() => {
        unregisterBlockComponentModel(props.nodeModel.blockId);
        viewModel()?.dispose?.();
    });

    return (
        <Show when={!loading() && !isBlank(props.nodeModel.blockId) && blockData() != null && viewModel()}>
            <BlockSubBlock nodeModel={props.nodeModel} viewModel={viewModel()} />
        </Show>
    );
}

export { Block, SubBlock };
