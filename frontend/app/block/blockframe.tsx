// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { blockViewToIcon, blockViewToName, ConnectionButton, getBlockHeaderIcon, Input } from "@/app/block/blockutil";
import { writeText as clipboardWriteText } from "@/util/clipboard";
import { Button } from "@/app/element/button";
import { ChangeConnectionBlockModal } from "@/app/modals/conntypeahead";
import { ContextMenuModel } from "@/app/store/contextmenu";
import {
    atoms,
    getBlockComponentModel,
    getConnStatusAtom,
    getSettingsKeyAtom,
    globalStore,
    recordTEvent,
    useBlockAtom,
    WOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { ErrorBoundary } from "@/element/errorboundary";
import { IconButton, ToggleIconButton } from "@/element/iconbutton";
import { BlockStatsBadge } from "@/element/blockstats";
import { MagnifyIcon } from "@/element/magnify";
import { MenuButton } from "@/element/menubutton";
import { NodeModel } from "@/layout/index";
import * as util from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/waveutil";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { CopyButton } from "../element/copybutton";
import { detectAgentColor, detectAgentFromEnv, detectAgentTextColor, getEffectiveTitle } from "./autotitle";
import { buildPaneContextMenu } from "./pane-actions";
import { BlockFrameProps } from "./blocktypes";
import { TitleBar } from "./titlebar";

const NumActiveConnColors = 8;

function handleHeaderContextMenu(
    e: MouseEvent,
    blockData: Block,
    viewModel: ViewModel,
    magnified: boolean,
    onMagnifyToggle: () => void,
    onClose: () => void
) {
    e.preventDefault();
    e.stopPropagation();

    // Start with the shared pane actions (copy, paste, split, magnify, close)
    const menu: ContextMenuItem[] = buildPaneContextMenu(blockData, {
        magnified,
        onMagnifyToggle,
        onClose,
    }, viewModel);

    // Header-only: view-specific settings (font size, theme, etc.)
    const extraItems = viewModel?.getSettingsMenuItems?.();
    if (extraItems && extraItems.length > 0) menu.push({ type: "separator" }, ...extraItems);

    // Header-only: title management + Copy BlockId
    menu.push(
        { type: "separator" },
        {
            label: "Copy BlockId",
            click: () => {
                clipboardWriteText(blockData.oid);
            },
        },
        {
            label: "Edit Pane Title",
            click: () => {
                const titleElement = document.querySelector(
                    `.block-${blockData.oid} .pane-title-text`
                ) as HTMLElement;
                if (titleElement) {
                    titleElement.click();
                }
            },
        },
        {
            label: "Auto-Generate Title",
            click: async () => {
                const { generateAutoTitle } = await import("./autotitle");
                const fullConfig = atoms.fullConfigAtom();
                const settingsEnv = fullConfig?.settings?.["cmd:env"] as Record<string, string> | undefined;
                const autoTitle = generateAutoTitle(blockData, settingsEnv);
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", blockData.oid),
                    meta: { "pane-title": autoTitle } as any,
                });
            },
        },
        {
            label: "Clear Title",
            click: async () => {
                await RpcApi.SetMetaCommand(TabRpcClient, {
                    oref: WOS.makeORef("block", blockData.oid),
                    meta: { "pane-title": "" } as any,
                });
            },
        }
    );

    ContextMenuModel.showContextMenu(menu, e);
}

function getViewIconElem(viewIconUnion: string | IconButtonDecl, blockData: Block): JSX.Element {
    if (viewIconUnion == null || typeof viewIconUnion === "string") {
        const viewIcon = viewIconUnion as string;
        return <div class="block-frame-view-icon">{getBlockHeaderIcon(viewIcon, blockData)}</div>;
    } else {
        return <IconButton decl={viewIconUnion} className="block-frame-view-icon" />;
    }
}

function OptMagnifyButton(props: { magnified: boolean; toggleMagnify: () => void; disabled: boolean }): JSX.Element {
    const magnifyDecl = createMemo<IconButtonDecl>(() => ({
        elemtype: "iconbutton",
        icon: <MagnifyIcon enabled={props.magnified} />,
        title: props.magnified ? "Minimize" : "Magnify",
        click: props.toggleMagnify,
        disabled: props.disabled,
    }));
    return <IconButton decl={magnifyDecl()} className="block-frame-magnify" />;
}

function EndIcons(props: {
    viewModel: ViewModel;
    nodeModel: NodeModel;
    onContextMenu: (e: MouseEvent) => void;
}): JSX.Element {
    const endIconButtons = util.useAtomValueSafe(props.viewModel?.endIconButtons);
    const magnified = () => props.nodeModel.isMagnified();
    const ephemeral = () => props.nodeModel.isEphemeral();
    const magnifyDisabled = () => false;

    const settingsDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "cog",
        title: "Settings",
        click: props.onContextMenu,
    };

    const closeDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "xmark-large",
        title: "Close",
        click: props.nodeModel.onClose,
    };

    return (
        <>
            <Show when={endIconButtons && endIconButtons.length > 0}>
                <For each={endIconButtons}>
                    {(button) => <IconButton decl={button} />}
                </For>
            </Show>
            <IconButton decl={settingsDecl} className="block-frame-settings" />
            <Show when={ephemeral()} fallback={
                <OptMagnifyButton
                    magnified={magnified()}
                    toggleMagnify={props.nodeModel.toggleMagnify}
                    disabled={magnifyDisabled()}
                />
            }>
                <IconButton decl={{
                    elemtype: "iconbutton",
                    icon: "circle-plus",
                    title: "Add to Layout",
                    click: () => { props.nodeModel.addEphemeralNodeToLayout(); },
                }} />
            </Show>
            <IconButton decl={closeDecl} className="block-frame-default-close" />
        </>
    );
}

function BlockFrame_Header(props: BlockFrameProps & { changeConnModalAtom: util.SignalAtom<boolean>; error?: Error }): JSX.Element {
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", props.nodeModel.blockId));
    const showBlockIds = getSettingsKeyAtom("blockheader:showblockids")();
    const preIconButton = util.useAtomValueSafe(props.viewModel?.preIconButton);
    const manageConnection = util.useAtomValueSafe(props.viewModel?.manageConnection);
    const dragHandleRef = props.preview ? null : props.nodeModel.dragHandleRef;
    const connName = blockData()?.meta?.connection;
    const connStatus = util.useAtomValueSafe(getConnStatusAtom(connName));
    const wshProblem = connName && !connStatus?.wshenabled && connStatus?.status == "connected";

    // Track previous magnified state for one-time activity report
    let prevMagnifiedState = props.nodeModel.isMagnified();
    createEffect(() => {
        const isMag = props.nodeModel.isMagnified();
        if (isMag && !prevMagnifiedState && !props.preview) {
            RpcApi.ActivityCommand(TabRpcClient, { nummagnify: 1 });
            const vn = util.useAtomValueSafe(props.viewModel?.viewName) ?? blockViewToName(blockData()?.meta?.view);
            recordTEvent("action:magnify", { "block:view": vn });
        }
        prevMagnifiedState = isMag;
    });

    const viewName = createMemo(() => {
        const bd = blockData();
        if (bd?.meta?.["frame:title"]) {
            return bd.meta["frame:title"];
        }
        let name = util.useAtomValueSafe(props.viewModel?.viewName) ?? blockViewToName(bd?.meta?.view);
        if (!bd?.meta?.["frame:title"] && bd?.meta?.view === "term") {
            const blockEnv = bd.meta["cmd:env"] as Record<string, string> | undefined;
            const agentId = detectAgentFromEnv(blockEnv);
            if (agentId) {
                name = agentId;
            }
        }
        return name;
    });

    const agentColor = createMemo(() => {
        const bd = blockData();
        if (!bd?.meta?.["frame:title"] && bd?.meta?.view === "term") {
            const blockEnv = bd.meta["cmd:env"] as Record<string, string> | undefined;
            const agentId = detectAgentFromEnv(blockEnv);
            if (agentId) return detectAgentColor(blockEnv, agentId);
        }
        return null;
    });

    const agentTextColor = createMemo(() => {
        const bd = blockData();
        if (!bd?.meta?.["frame:title"] && bd?.meta?.view === "term") {
            const blockEnv = bd.meta["cmd:env"] as Record<string, string> | undefined;
            const agentId = detectAgentFromEnv(blockEnv);
            if (agentId) return detectAgentTextColor(blockEnv, agentId);
        }
        return null;
    });

    const viewIconUnion = createMemo(() => {
        const bd = blockData();
        if (bd?.meta?.["frame:icon"]) return bd.meta["frame:icon"];
        return util.useAtomValueSafe(props.viewModel?.viewIcon) ?? blockViewToIcon(bd?.meta?.view);
    });

    const headerTextUnion = createMemo(() => {
        const bd = blockData();
        if (bd?.meta?.["frame:text"]) return bd.meta["frame:text"];
        return util.useAtomValueSafe(props.viewModel?.viewText);
    });

    const onContextMenu = (e: MouseEvent) => {
        handleHeaderContextMenu(e, blockData(), props.viewModel, props.nodeModel.isMagnified(), props.nodeModel.toggleMagnify, props.nodeModel.onClose);
    };
    const viewIconElem = getViewIconElem(viewIconUnion(), blockData());

    const preIconButtonElem: JSX.Element = preIconButton
        ? <IconButton decl={preIconButton} className="block-frame-preicon-button" />
        : null;

    const headerTextElems: JSX.Element[] = [];
    const htu = headerTextUnion();
    if (typeof htu === "string") {
        if (!util.isBlank(htu)) {
            headerTextElems.push(
                <div class="block-frame-text ellipsis">
                    &lrm;{htu}
                </div>
            );
        }
    } else if (Array.isArray(htu)) {
        headerTextElems.push(...renderHeaderElements(htu, props.preview));
    }
    if (props.error != null) {
        const copyHeaderErr = () => {
            clipboardWriteText(props.error.message + "\n" + props.error.stack);
        };
        headerTextElems.push(
            <div class="iconbutton disabled" onClick={copyHeaderErr}>
                <i
                    class="fa-sharp fa-solid fa-triangle-exclamation"
                    title={"Error Rendering View Header: " + props.error.message}
                />
            </div>
        );
    }
    const wshInstallButton: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "link-slash",
        title: "wsh is not installed for this connection",
    };
    const showNoWshButton = manageConnection && wshProblem && !util.isBlank(connName) && !connName.startsWith("aws:");

    const headerStyle = createMemo<JSX.CSSProperties>(() => {
        const style: JSX.CSSProperties = {};
        const ac = agentColor();
        const atc = agentTextColor();
        if (ac) style["background-color"] = ac;
        if (atc) style.color = atc;
        return style;
    });

    return (
        <div
            class="block-frame-default-header"
            data-role="block-header"
            data-testid="block-header"
            ref={dragHandleRef ? (el) => { dragHandleRef.current = el; } : undefined}
            onContextMenu={onContextMenu}
            onDblClick={() => props.nodeModel.toggleMagnify()}
            style={headerStyle()}
        >
            {preIconButtonElem}
            <div class="block-frame-default-header-iconview">
                {viewIconElem}
                <div class="block-frame-view-type">{viewName()}</div>
                <Show when={showBlockIds}>
                    <div class="block-frame-blockid">[{props.nodeModel.blockId.substring(0, 8)}]</div>
                </Show>
            </div>
            <Show when={manageConnection}>
                <ConnectionButton
                    ref={props.connBtnRef}
                    connection={blockData()?.meta?.connection}
                    changeConnModalAtom={props.changeConnModalAtom}
                />
            </Show>
            <Show when={showNoWshButton}>
                <IconButton decl={wshInstallButton} className="block-frame-header-iconbutton" />
            </Show>
            <div class="block-frame-textelems-wrapper">{headerTextElems}</div>
            <div class="block-frame-end-icons">
                <EndIcons viewModel={props.viewModel} nodeModel={props.nodeModel} onContextMenu={onContextMenu} />
            </div>
        </div>
    );
}

function HeaderTextElem({ elem, preview }: { elem: HeaderElem; preview: boolean }): JSX.Element {
    if (elem.elemtype == "iconbutton") {
        return <IconButton decl={elem} className={clsx("block-frame-header-iconbutton", elem.className)} />;
    } else if (elem.elemtype == "toggleiconbutton") {
        return <ToggleIconButton decl={elem} className={clsx("block-frame-header-iconbutton", elem.className)} />;
    } else if (elem.elemtype == "input") {
        return <Input decl={elem} className={clsx("block-frame-input", elem.className)} preview={preview} />;
    } else if (elem.elemtype == "text") {
        return (
            <div class={clsx("block-frame-text ellipsis", elem.className, { "flex-nogrow": elem.noGrow })}>
                <span ref={preview ? undefined : (el) => { if (elem.ref) (elem.ref as any).current = el; }} onClick={(e) => elem?.onClick?.(e)}>
                    &lrm;{elem.text}
                </span>
            </div>
        );
    } else if (elem.elemtype == "textbutton") {
        return (
            <Button className={elem.className} onClick={(e) => elem.onClick?.(e)} title={elem.title}>
                {elem.text}
            </Button>
        );
    } else if (elem.elemtype == "div") {
        return (
            <div
                class={clsx("block-frame-div", elem.className)}
                onMouseOver={elem.onMouseOver}
                onMouseOut={elem.onMouseOut}
            >
                <For each={elem.children}>
                    {(child, childIdx) => <HeaderTextElem elem={child} preview={preview} />}
                </For>
            </div>
        );
    } else if (elem.elemtype == "menubutton") {
        return <MenuButton className="block-frame-menubutton" {...(elem as MenuButtonProps)} />;
    }
    return null;
}

function renderHeaderElements(headerTextUnion: HeaderElem[], preview: boolean): JSX.Element[] {
    const headerTextElems: JSX.Element[] = [];
    for (let idx = 0; idx < headerTextUnion.length; idx++) {
        const elem = headerTextUnion[idx];
        const renderedElement = <HeaderTextElem elem={elem} preview={preview} />;
        if (renderedElement) {
            headerTextElems.push(renderedElement);
        }
    }
    return headerTextElems;
}

function ConnStatusOverlay({
    nodeModel,
    viewModel,
    changeConnModalAtom,
}: {
    nodeModel: NodeModel;
    viewModel: ViewModel;
    changeConnModalAtom: util.SignalAtom<boolean>;
}): JSX.Element {
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", nodeModel.blockId));
    const connModalOpen = changeConnModalAtom();
    const connName = createMemo(() => blockData()?.meta?.connection);
    const connStatus = createMemo(() => getConnStatusAtom(connName())());
    const isLayoutMode = atoms.controlShiftDelayAtom();
    const [width, setWidth] = createSignal<number>(null);
    let overlayRef: HTMLDivElement;
    const [showError, setShowError] = createSignal(false);
    const fullConfig = atoms.fullConfigAtom();
    const [showWshError, setShowWshError] = createSignal(false);

    onMount(() => {
        const rszObs = new ResizeObserver((entries) => {
            for (const entry of entries) {
                setWidth(entry.contentRect.width);
            }
        });
        if (overlayRef) rszObs.observe(overlayRef);
        onCleanup(() => rszObs.disconnect());
    });

    createEffect(() => {
        const w = width();
        if (w) {
            const cs = connStatus();
            const hasError = !util.isBlank(cs?.error);
            const show = hasError && w >= 250 && cs?.status == "error";
            setShowError(show);
        }
    });

    createEffect(() => {
        const cs = connStatus();
        const wshConfigEnabled = fullConfig?.connections?.[connName()]?.["conn:wshenabled"] ?? true;
        const showWshErrorTemp =
            cs?.status == "connected" &&
            cs?.wsherror &&
            cs?.wsherror != "" &&
            wshConfigEnabled;
        setShowWshError(showWshErrorTemp);
    });

    const handleTryReconnect = () => {
        const prtn = RpcApi.ConnConnectCommand(
            TabRpcClient,
            { host: connName(), logblockid: nodeModel.blockId },
            { timeout: 60000 }
        );
        prtn.catch((e) => console.log("error reconnecting", connName(), e));
    };

    const handleDisableWsh = async () => {
        const metamaptype: unknown = {
            "conn:wshenabled": false,
        };
        const data: ConnConfigRequest = {
            host: connName(),
            metamaptype: metamaptype,
        };
        try {
            await RpcApi.SetConnectionsConfigCommand(TabRpcClient, data);
        } catch (e) {
            console.log("problem setting connection config: ", e);
        }
    };

    const handleRemoveWshError = async () => {
        try {
            await RpcApi.DismissWshFailCommand(TabRpcClient, connName());
        } catch (e) {
            console.log("unable to dismiss wsh error: ", e);
        }
    };

    const statusText = createMemo(() => {
        const cs = connStatus();
        if (cs?.status == "connecting") return `Connecting to "${connName()}"...`;
        return `Disconnected from "${connName()}"`;
    });

    const showReconnect = createMemo(() => {
        const cs = connStatus();
        return cs?.status !== "connecting" && cs?.status !== "connected";
    });

    const reconClassName = createMemo(() => {
        const w = width();
        let base = "outlined grey";
        if (w && w < 350) {
            return clsx(base, "text-[12px] py-[5px] px-[6px]");
        }
        return clsx(base, "text-[11px] py-[3px] px-[7px]");
    });

    const showIcon = createMemo(() => connStatus()?.status != "connecting");

    const handleCopy = async (e: MouseEvent) => {
        const errTexts = [];
        if (showError()) {
            errTexts.push(`error: ${connStatus()?.error}`);
        }
        if (showWshError()) {
            errTexts.push(`unable to use wsh: ${connStatus()?.wsherror}`);
        }
        const textToCopy = errTexts.join("\n");
        await clipboardWriteText(textToCopy);
    };

    return (
        <Show when={showWshError() || (!isLayoutMode && connStatus()?.status !== "connected" && !connModalOpen)}>
            <div class="connstatus-overlay" ref={(el) => { overlayRef = el; }}>
                <div class="connstatus-content">
                    <div class={clsx("connstatus-status-icon-wrapper", { "has-error": showError() || showWshError() })}>
                        <Show when={showIcon()}>
                            <i class="fa-solid fa-triangle-exclamation"></i>
                        </Show>
                        <div class="connstatus-status ellipsis">
                            <div class="connstatus-status-text">{statusText()}</div>
                            <Show when={showError() || showWshError()}>
                                <div class="connstatus-error" style={{ "overflow-y": "auto" }}>
                                    <CopyButton className="copy-button" onClick={handleCopy} title="Copy" />
                                    <Show when={showError()}>
                                        <div>error: {connStatus()?.error}</div>
                                    </Show>
                                    <Show when={showWshError()}>
                                        <div>unable to use wsh: {connStatus()?.wsherror}</div>
                                    </Show>
                                </div>
                            </Show>
                            <Show when={showWshError()}>
                                <Button className={reconClassName()} onClick={handleDisableWsh}>
                                    always disable wsh
                                </Button>
                            </Show>
                        </div>
                    </div>
                    <Show when={showReconnect()}>
                        <div class="connstatus-actions">
                            <Button className={reconClassName()} onClick={handleTryReconnect}>
                                <Show
                                    when={width() && width() < 350}
                                    fallback="Reconnect"
                                >
                                    <i class="fa-sharp fa-solid fa-rotate-right"></i>
                                </Show>
                            </Button>
                        </div>
                    </Show>
                    <Show when={showWshError()}>
                        <div class="connstatus-actions">
                            <Button className={`fa-xmark fa-solid ${reconClassName()}`} onClick={handleRemoveWshError} />
                        </div>
                    </Show>
                </div>
            </div>
        </Show>
    );
}

function BlockMask({ nodeModel }: { nodeModel: NodeModel }): JSX.Element {
    const isFocused = () => nodeModel.isFocused();
    const blockNum = () => nodeModel.blockNum();
    const isLayoutMode = () => atoms.controlShiftDelayAtom();
    const showOverlayBlockNums = () => getSettingsKeyAtom("app:showoverlayblocknums")() ?? true;
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", nodeModel.blockId));

    const style = createMemo<JSX.CSSProperties>(() => {
        const style: JSX.CSSProperties = {};
        const bd = blockData();
        if (isFocused()) {
            const tabData = atoms.tabAtom();
            const tabActiveBorderColor = tabData?.meta?.["bg:activebordercolor"];
            if (tabActiveBorderColor) {
                style["border-color"] = tabActiveBorderColor;
            }
            if (bd?.meta?.["frame:activebordercolor"]) {
                style["border-color"] = bd.meta["frame:activebordercolor"];
            }
        } else {
            const tabData = atoms.tabAtom();
            const tabBorderColor = tabData?.meta?.["bg:bordercolor"];
            if (tabBorderColor) {
                style["border-color"] = tabBorderColor;
            }
            if (bd?.meta?.["frame:bordercolor"]) {
                style["border-color"] = bd.meta["frame:bordercolor"];
            }
        }
        return style;
    });

    const showBlockMask = () => isLayoutMode() && showOverlayBlockNums();

    return (
        <div class={clsx("block-mask", { "show-block-mask": showBlockMask() })} style={style()}>
            <Show when={showBlockMask()}>
                <div class="block-mask-inner">
                    <div class="bignum">{blockNum()}</div>
                </div>
            </Show>
        </div>
    );
}

function BlockFrame_Default_Component(props: BlockFrameProps): JSX.Element {
    const nodeModel = props.nodeModel;
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", nodeModel.blockId));
    const isFocused = () => nodeModel.isFocused();
    const customBg = util.useAtomValueSafe(props.viewModel?.blockBg);
    const manageConnection = util.useAtomValueSafe(props.viewModel?.manageConnection);
    const changeConnModalAtom = useBlockAtom(nodeModel.blockId, "changeConn", () => {
        return util.createSignalAtom(false);
    }) as util.SignalAtom<boolean>;
    const connModalOpen = () => changeConnModalAtom();
    const isMagnified = () => nodeModel.isMagnified();
    const isEphemeral = () => nodeModel.isEphemeral();
    const magnifiedBlockBlurAtom = getSettingsKeyAtom("window:magnifiedblockblurprimarypx");
    const magnifiedBlockBlur = () => magnifiedBlockBlurAtom();
    const magnifiedBlockOpacityAtom = getSettingsKeyAtom("window:magnifiedblockopacity");
    const magnifiedBlockOpacity = () => magnifiedBlockOpacityAtom();
    let connBtnRef: { current: HTMLDivElement | null } = { current: null };
    const noHeader = util.useAtomValueSafe(props.viewModel?.noHeader);

    // Compute agent color for border styling (matches header color)
    const blockAgentColor = createMemo(() => {
        if (!props.preview && blockData()?.meta?.view === "term") {
            const blockEnv = blockData()?.meta?.["cmd:env"] as Record<string, string> | undefined;
            const agentId = detectAgentFromEnv(blockEnv);
            if (agentId) {
                return detectAgentColor(blockEnv, agentId);
            }
        }
        return null;
    });

    createEffect(() => {
        if (!manageConnection) {
            return;
        }
        const bcm = getBlockComponentModel(nodeModel.blockId);
        if (bcm != null) {
            bcm.openSwitchConnection = () => {
                changeConnModalAtom._set(true);
            };
        }
        onCleanup(() => {
            const bcm = getBlockComponentModel(nodeModel.blockId);
            if (bcm != null) {
                bcm.openSwitchConnection = null;
            }
        });
    });

    createEffect(() => {
        // on mount, if manageConnection, call ConnEnsure
        if (!manageConnection || blockData() == null || props.preview) {
            return;
        }
        const connName = blockData()?.meta?.connection;
        if (!util.isBlank(connName)) {
            console.log("ensure conn", nodeModel.blockId, connName);
            RpcApi.ConnEnsureCommand(
                TabRpcClient,
                { connname: connName, logblockid: nodeModel.blockId },
                { timeout: 60000 }
            ).catch((e) => {
                console.log("error ensuring connection", nodeModel.blockId, connName, e);
            });
        }
    });

    const viewIconUnion = util.useAtomValueSafe(props.viewModel?.viewIcon) ?? blockViewToIcon(blockData()?.meta?.view);
    const viewIconElem = getViewIconElem(viewIconUnion, blockData());
    let innerStyle: JSX.CSSProperties = {};
    if (!props.preview) {
        innerStyle = computeBgStyleFromMeta(customBg);
    }
    const previewElem = <div class="block-frame-preview">{viewIconElem}</div>;
    const headerElem = (
        <BlockFrame_Header {...props} connBtnRef={connBtnRef} changeConnModalAtom={changeConnModalAtom} />
    );
    const headerElemNoView = (
        <BlockFrame_Header {...props} connBtnRef={connBtnRef} changeConnModalAtom={changeConnModalAtom} viewModel={null} />
    );

    // Body right-click handler
    const onBodyContextMenu = (e: MouseEvent) => {
        if (!blockData() || props.preview) return;
        e.preventDefault();
        e.stopPropagation();
        const menu = buildPaneContextMenu(blockData(), {
            magnified: isMagnified(),
            onMagnifyToggle: nodeModel.toggleMagnify,
            onClose: nodeModel.onClose,
        }, props.viewModel);
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            class={clsx("block", "block-frame-default", "block-" + nodeModel.blockId, {
                "block-focused": isFocused() || props.preview,
                "block-preview": props.preview,
                "block-no-highlight": props.numBlocksInTab === 1,
                "has-agent-color": !!blockAgentColor(),
                ephemeral: isEphemeral(),
                magnified: isMagnified(),
            })}
            data-blockid={nodeModel.blockId}
            onClick={props.blockModel?.onClick}
            onFocusIn={props.blockModel?.onFocusCapture}
            onContextMenu={onBodyContextMenu}
            ref={props.blockModel?.blockRef ? (el) => { props.blockModel.blockRef.current = el; } : undefined}
            style={
                {
                    "--magnified-block-opacity": magnifiedBlockOpacity(),
                    "--magnified-block-blur": `${magnifiedBlockBlur()}px`,
                    "--block-agent-color": blockAgentColor() ?? "transparent",
                } as JSX.CSSProperties
            }
            inert={props.preview || undefined}
        >
            <BlockMask nodeModel={nodeModel} />
            <Show when={!props.preview && props.viewModel != null}>
                <ConnStatusOverlay
                    nodeModel={nodeModel}
                    viewModel={props.viewModel}
                    changeConnModalAtom={changeConnModalAtom}
                />
            </Show>
            <div class="block-frame-default-inner" style={innerStyle}>
                {noHeader || <ErrorBoundary fallback={headerElemNoView}>{headerElem}</ErrorBoundary>}
                <Show when={!props.preview && blockData()}>
                    <TitleBar
                        blockId={nodeModel.blockId}
                        blockMeta={blockData()?.meta}
                        title={getEffectiveTitle(blockData(), false, atoms.fullConfigAtom()?.settings?.["cmd:env"] as Record<string, string> | undefined)}
                    />
                </Show>
                {props.preview ? previewElem : props.children}
                <BlockStatsBadge blockId={nodeModel.blockId} />
            </div>
            <Show when={!props.preview && props.viewModel != null && connModalOpen()}>
                <ChangeConnectionBlockModal
                    blockId={nodeModel.blockId}
                    nodeModel={nodeModel}
                    viewModel={props.viewModel}
                    blockRef={props.blockModel?.blockRef}
                    changeConnModalOpen={changeConnModalAtom}
                    setChangeConnModalOpen={(v) => changeConnModalAtom._set(v)}
                    connBtnRef={connBtnRef}
                />
            </Show>
        </div>
    );
}

function BlockFrame_Default(props: BlockFrameProps): JSX.Element {
    return <BlockFrame_Default_Component {...props} />;
}

function BlockFrame(props: BlockFrameProps): JSX.Element {
    const blockId = props.nodeModel.blockId;
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", blockId));
    const numBlocks = () => atoms.tabAtom()?.blockids?.length ?? 0;
    return (
        <Show when={blockId && blockData()}>
            <BlockFrame_Default {...props} numBlocksInTab={numBlocks()} />
        </Show>
    );
}

export { BlockFrame, NumActiveConnColors };
