// Copyright 2025-2026, AgentMux Corp.
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
    recordTEvent,
    useBlockAtom,
    WOS,
} from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { ErrorBoundary } from "@/element/errorboundary";
import { IconButton, ToggleIconButton } from "@/element/iconbutton";
import { BlockStatsBadge } from "@/element/blockstats";
import { MenuButton } from "@/element/menubutton";
import { MicButton } from "@/app/element/MicButton";
import { invokeCommand } from "@/app/platform/ipc";
import { NodeModel } from "@/layout/index";
import * as util from "@/util/util";
import { computeBgStyleFromMeta } from "@/util/waveutil";
import clsx from "clsx";
import type { JSX } from "solid-js";
import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { CopyButton } from "../element/copybutton";
import { detectAgentColor, detectAgentFromEnv, detectAgentTextColor, getEffectiveTitle, isUsableFocusRingColor } from "./autotitle";
import { buildPaneContextMenu } from "./pane-actions";
import { hueToHeaderBg, hueToActiveBorder, PANE_HUE_OPTIONS, setHue } from "./pane-color-menu";
import { BlockFrameProps } from "./blocktypes";
import { PaneSizeBadge } from "./pane-size-badge";
import { TitleBar } from "./titlebar";

const NumActiveConnColors = 8;

/**
 * Build a "Pane Color" submenu — mirrors the "Replace With..." submenu pattern
 * (pane-actions.ts). Looks and acts exactly like the rest of the context menu:
 * expands on hover, fully clickable, consistent with every other menu. Each
 * item carries an inline color swatch in its exact hue (the context menu is a
 * DOM overlay, so `swatchColor` renders a real colored square before the label
 * — see showJsContextMenu). The current color is marked with a checkmark;
 * "Default" clears it.
 */
function buildPaneColorSubmenu(blockData: Block): ContextMenuItem[] {
    const currentHue = blockData?.meta?.["frame:hue"] as number | undefined;
    const colorItems: ContextMenuItem[] = [
        {
            label: "Default",
            checked: currentHue == null,
            click: () => setHue(blockData.oid, null),
        },
        { type: "separator" as const },
        ...PANE_HUE_OPTIONS.map(({ label, hue }) => ({
            label,
            swatchColor: hueToActiveBorder(hue),
            checked: currentHue === hue,
            click: () => setHue(blockData.oid, hue),
        })),
    ];
    // Leading separator (not trailing): the caller may append a view-settings
    // group that also starts with a separator. A trailing separator here would
    // collide with it and render two consecutive dividers for views that
    // implement getSettingsMenuItems (term, sysinfo).
    return [
        { type: "separator" as const },
        { label: "Pane Color", type: "submenu" as const, submenu: colorItems },
    ];
}

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
        inspectAt: { x: e.clientX, y: e.clientY },
    }, viewModel);

    // Header-only: pane color submenu (mirrors "Replace With...")
    menu.push(...buildPaneColorSubmenu(blockData));

    // Header-only: view-specific settings (font size, theme, etc.)
    const extraItems = viewModel?.getSettingsMenuItems?.();
    if (extraItems && extraItems.length > 0) menu.push({ type: "separator" }, ...extraItems);

    ContextMenuModel.showContextMenu(menu, e);
}

/** Inline editable name field shown in the pane header when the viewModel provides setViewName. */
function ViewNameEditor(props: { name: string; onSave: (v: string) => void }): JSX.Element {
    const [editing, setEditing] = createSignal(false);
    const [draft, setDraft] = createSignal(props.name);

    const commit = () => {
        const v = draft().trim();
        if (v) props.onSave(v);
        setEditing(false);
    };
    const cancel = () => {
        setDraft(props.name);
        setEditing(false);
    };

    return (
        <Show
            when={editing()}
            fallback={
                <div
                    class="block-frame-view-type block-frame-view-type--editable"
                    onClick={() => { setDraft(props.name); setEditing(true); }}
                    onDblClick={(e) => e.stopPropagation()}
                    title="Click to rename"
                >
                    {props.name}
                </div>
            }
        >
            <input
                class="block-frame-view-type block-frame-view-type--input"
                value={draft()}
                onInput={(e) => setDraft(e.currentTarget.value)}
                onBlur={commit}
                onKeyDown={(e) => {
                    if (e.key === "Enter") { e.preventDefault(); commit(); }
                    if (e.key === "Escape") { e.preventDefault(); cancel(); }
                    e.stopPropagation();
                }}
                onDblClick={(e) => e.stopPropagation()}
                ref={(el) => setTimeout(() => { el.focus(); el.select(); }, 0)}
            />
        </Show>
    );
}

function getViewIconElem(viewIconUnion: string | IconButtonDecl, blockData: Block): JSX.Element {
    if (viewIconUnion == null || typeof viewIconUnion === "string") {
        const viewIcon = viewIconUnion as string;
        return <div class="block-frame-view-icon">{getBlockHeaderIcon(viewIcon, blockData)}</div>;
    } else {
        // Swallow dblclick so fast clicking on a clickable view icon (e.g.
        // the editor's tree-toggle) doesn't bubble up to the header's
        // toggleMagnify handler.
        return (
            <span onDblClick={(e) => e.stopPropagation()}>
                <IconButton decl={viewIconUnion} className="block-frame-view-icon" />
            </span>
        );
    }
}

function OptMinimizeButton(props: { minimized: boolean; toggleMinimize: () => void }): JSX.Element {
    // OS-window minimize convention (a horizontal bar), matching the sibling
    // OptMagnifyButton's window-maximize/window-restore pairing just below —
    // the previous chevron-up/chevron-down pair read as a dropdown toggle
    // rather than a minimize control.
    const decl = createMemo<IconButtonDecl>(() => ({
        elemtype: "iconbutton",
        icon: props.minimized ? "window-restore" : "window-minimize",
        title: props.minimized ? "Restore" : "Minimize",
        click: props.toggleMinimize,
    }));
    return <IconButton decl={decl()} className="block-frame-minimize" />;
}

function OptMagnifyButton(props: { magnified: boolean; toggleMagnify: () => void; disabled: boolean }): JSX.Element {
    const magnifyDecl = createMemo<IconButtonDecl>(() => ({
        elemtype: "iconbutton",
        icon: props.magnified ? "window-restore" : "window-maximize",
        title: props.magnified ? "Restore" : "Maximize",
        click: props.toggleMagnify,
        disabled: props.disabled,
    }));
    return <IconButton decl={magnifyDecl()} className="block-frame-magnify" />;
}

/** Maximize button for a torn-off FLOATING pane (the floating half of the
 *  shared maximize button — docked panes use {@link OptMagnifyButton} →
 *  layout magnify; see SPEC_PANE_STATE_REDUCER §3.3a). Routes to the host
 *  `toggle_floating_maximize` IPC, which dispatches the reducer's
 *  `ToggleFloatingMaximize` and applies the OS-window geometry (maximize to
 *  the monitor work area / restore to the captured rect).
 *
 *  Deliberately a FIXED button: the icon and title don't reflect the
 *  maximized/normal state. A single click toggles maximize↔restore (users
 *  expect a maximize button to toggle), which keeps the button stateless —
 *  the reducer is the single source of truth for placement, not a mirrored
 *  frontend signal that can drift. */
function FloatingMaximizeButton(props: { label: string; blockId: string }): JSX.Element {
    const decl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "window-maximize",
        title: "Maximize",
        click: () => {
            invokeCommand("toggle_floating_maximize", { label: props.label, block_id: props.blockId }).catch(
                console.error
            );
        },
    };
    return <IconButton decl={decl} className="block-frame-magnify" />;
}

function EndIcons(props: {
    viewModel: ViewModel;
    nodeModel: NodeModel;
    onContextMenu: (e: MouseEvent) => void;
    /** View key from `blockData.meta.view`, used to render a context-aware
     *  tooltip on the per-pane mic button (e.g. "Speak into this terminal"). */
    blockView?: string;
}): JSX.Element {
    // createMemo so blockAtom reads inside endIconButtons() are tracked and
    // the button array re-evaluates when the agent loads/unloads.
    const endIconButtons = createMemo(() => util.useAtomValueSafe(props.viewModel?.endIconButtons));
    const magnified = () => props.nodeModel.isMagnified();
    const minimized = () => props.nodeModel.isMinimized();
    const ephemeral = () => props.nodeModel.isEphemeral();
    const numLeafs = () => props.nodeModel.numLeafs();
    const magnifyDisabled = () => false;
    // In a torn-off floating shell the window carries a `?windowLabel=floating-…`
    // URL param (set by the host when it opens the popup). When present, the
    // magnify button becomes an OS-window maximize/restore routed to the host
    // reducer; docked panes (no such label) keep layout magnify.
    const floatingLabel = createMemo(() => {
        const label = new URLSearchParams(window.location.search).get("windowLabel") ?? "";
        return label.startsWith("floating-") ? label : null;
    });

    const closeDecl: IconButtonDecl = {
        elemtype: "iconbutton",
        icon: "xmark-large",
        title: "Close",
        click: props.nodeModel.onClose,
    };

    return (
        <>
            <Show when={(endIconButtons()?.length ?? 0) > 0}>
                <For each={endIconButtons()}>
                    {(button) => <IconButton decl={button} />}
                </For>
                <div class="block-frame-btn-separator" aria-hidden="true" />
            </Show>
            {/* Agent panes render their own mic pinned beside the composer
                input (AgentFooter.tsx) instead of here — see
                SPEC_AGENT_WORKING_INDICATOR_SHIMMER_AND_MIC_RELOCATION_2026_07_08.md.
                Terminal keeps the header mic unchanged. */}
            <Show when={props.viewModel?.voiceHandle && props.blockView !== "agent"}>
                <MicButton
                    blockId={props.nodeModel.blockId}
                    handle={props.viewModel.voiceHandle!()}
                    paneTitle={
                        props.blockView === "term"
                            ? "Speak into this terminal (Ctrl+Shift+V)"
                            : undefined
                    }
                />
            </Show>

            {/* canMinimize: the last expanded pane loses its minimize button —
                the window must always keep at least one expanded pane, so
                another pane has to be restored before this one can collapse.
                Optional call: NodeModels are cached on the LayoutModel, so a
                hot-reload can pair this (new) header with a pre-canMinimize
                cached model — degrade to showing the button (the toggle's own
                countExpandedLeaves guard still enforces the policy). */}
            <Show when={!ephemeral() && !magnified() && numLeafs() > 1 && (props.nodeModel.canMinimize?.() ?? true)}>
                <OptMinimizeButton
                    minimized={minimized()}
                    toggleMinimize={props.nodeModel.toggleMinimize}
                />
            </Show>

            <Show when={ephemeral()} fallback={
                <Show when={floatingLabel()} fallback={
                    <OptMagnifyButton
                        magnified={magnified()}
                        toggleMagnify={props.nodeModel.toggleMagnify}
                        disabled={magnifyDisabled()}
                    />
                }>
                    <FloatingMaximizeButton label={floatingLabel()!} blockId={props.nodeModel.blockId} />
                </Show>
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
        // Native context menu. Pane color is a native "Pane Color" submenu inside
        // it (see handleHeaderContextMenu / buildPaneColorSubmenu) — mirroring
        // "Replace With...". Fully native: expands on hover, clickable, no DOM
        // overlay to conflict with the modal menu.
        handleHeaderContextMenu(
            e,
            blockData(),
            props.viewModel,
            props.nodeModel.isMagnified(),
            props.nodeModel.toggleMagnify,
            props.nodeModel.onClose
        );
    };
    const viewFaviconUrl = createMemo(() => util.useAtomValueSafe(props.viewModel?.viewFaviconUrl));
    const viewIconElem = createMemo(() => {
        const favUrl = viewFaviconUrl();
        // Diag for the favicon path — log every recomputation of this
        // memo for browser-view blocks so muxlog shows whether the
        // atom is being read with a non-empty value. Throttle is the
        // memo itself: it only re-fires when the dependent atom
        // changes, so this won't spam.
        if (blockData()?.meta?.view === "browser") {
            const vmId = (props.viewModel as any)?.__diagVmId ?? "?";
            console.log(`[browser-pane:diag][${(blockData()?.oid ?? "").slice(0, 7)} vm=${vmId}] header-render favUrl=${JSON.stringify(favUrl ?? "")}`);
        }
        if (favUrl) {
            return (
                <div class="block-frame-view-icon">
                    <img
                        class="browser-pane-favicon"
                        src={favUrl}
                        alt=""
                        width={14}
                        height={14}
                        onLoad={(e) => {
                            // Reagent P2 on #876: SolidJS reuses the same
                            // <img> DOM node when `src` reactively changes,
                            // so a `display:none` from a prior failed
                            // favicon persists across `src` swaps and hides
                            // all subsequent valid favicons. Reset on every
                            // successful load.
                            (e.currentTarget as HTMLImageElement).style.display = "";
                            console.log(`[browser-pane:diag][${(blockData()?.oid ?? "").slice(0, 7)}] favicon-load ok src=${JSON.stringify(favUrl)}`);
                        }}
                        onError={(e) => {
                            (e.currentTarget as HTMLImageElement).style.display = "none";
                            console.log(`[browser-pane:diag][${(blockData()?.oid ?? "").slice(0, 7)}] favicon-load fail src=${JSON.stringify(favUrl)}`);
                        }}
                    />
                </div>
            );
        }
        return getViewIconElem(viewIconUnion(), blockData());
    });

    const preIconButtonElem: JSX.Element = preIconButton
        ? <IconButton decl={preIconButton} className="block-frame-preicon-button" />
        : null;

    const headerTextElems = createMemo(() => {
        const elems: JSX.Element[] = [];
        const htu = headerTextUnion();
        if (typeof htu === "string") {
            if (!util.isBlank(htu)) {
                elems.push(
                    <div class="block-frame-text ellipsis">
                        &lrm;{htu}
                    </div>
                );
            }
        } else if (Array.isArray(htu)) {
            elems.push(...renderHeaderElements(htu, props.preview));
        }
        return elems;
    });
    // True when the textelems wrapper has visible content (summary text or an
    // error indicator). Used to conditionally apply max-width to the name
    // region — see block.scss .block-frame-default-header--has-summary.
    const hasSummary = createMemo(() => {
        if (props.error != null) return true;
        const htu = headerTextUnion();
        if (typeof htu === "string") return !util.isBlank(htu);
        if (Array.isArray(htu)) return htu.length > 0;
        return false;
    });
    const headerStyle = createMemo<JSX.CSSProperties>(() => {
        const style: JSX.CSSProperties = {};
        const hue = blockData()?.meta?.["frame:hue"];
        if (typeof hue === "number") {
            // User-chosen hue overrides the env-var agent color.
            style["background-color"] = hueToHeaderBg(hue);
        } else {
            const ac = agentColor();
            const atc = agentTextColor();
            if (ac) style["background-color"] = ac;
            if (atc) style.color = atc;
        }
        return style;
    });

    return (
        <div
            class="block-frame-default-header"
            classList={{ "block-frame-default-header--agent": blockData()?.meta?.view === "agent", "block-frame-default-header--has-summary": hasSummary() }}
            data-role="block-header"
            data-testid="block-header"
            ref={dragHandleRef ? (el) => { dragHandleRef.current = el; } : undefined}
            onContextMenu={onContextMenu}
            onDblClick={() => props.nodeModel.toggleMagnify()}
            style={headerStyle()}
        >
            {preIconButtonElem}
            <div class="block-frame-default-header-iconview">
                {viewIconElem()}
                <Show
                    when={props.viewModel?.setViewName}
                    fallback={<div class="block-frame-view-type">{viewName()}</div>}
                >
                    <ViewNameEditor name={viewName()} onSave={(v) => void props.viewModel.setViewName(v)} />
                </Show>
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
            <div class="block-frame-textelems-wrapper">
                {headerTextElems()}
                <Show when={props.error != null}>
                    <div
                        class="iconbutton disabled"
                        onClick={() => clipboardWriteText(props.error.message + "\n" + props.error.stack)}
                    >
                        <i
                            class="fa-sharp fa-solid fa-triangle-exclamation"
                            title={"Error Rendering View Header: " + props.error.message}
                        />
                    </div>
                </Show>
            </div>
            <div class="block-frame-end-icons" onDblClick={(e) => e.stopPropagation()}>
                <EndIcons
                    viewModel={props.viewModel}
                    nodeModel={props.nodeModel}
                    onContextMenu={onContextMenu}
                    blockView={blockData()?.meta?.view}
                />
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

    const handleTryReconnect = () => {
        const prtn = RpcApi.ConnConnectCommand(
            TabRpcClient,
            { host: connName(), logblockid: nodeModel.blockId },
            { timeout: 60000 }
        );
        prtn.catch((e) => console.log("error reconnecting", connName(), e));
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
        const textToCopy = errTexts.join("\n");
        await clipboardWriteText(textToCopy);
    };

    return (
        <Show when={!isLayoutMode && connStatus()?.status !== "connected" && !connModalOpen}>
            <div class="connstatus-overlay" ref={(el) => { overlayRef = el; }}>
                <div class="connstatus-content">
                    <div class={clsx("connstatus-status-icon-wrapper", { "has-error": showError() })}>
                        <Show when={showIcon()}>
                            <i class="fa-solid fa-triangle-exclamation"></i>
                        </Show>
                        <div class="connstatus-status ellipsis">
                            <div class="connstatus-status-text">{statusText()}</div>
                            <Show when={showError()}>
                                <div class="connstatus-error" style={{ "overflow-y": "auto" }}>
                                    <CopyButton className="copy-button" onClick={handleCopy} title="Copy" />
                                    <div>error: {connStatus()?.error}</div>
                                </div>
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
            // frame:activebordercolor is a passive default (per-agent
            // identity color, seeded once at launch — see
            // SPEC_AGENT_COLOR_2026_08_08.md); frame:hue is an explicit
            // user choice from the pane-header "Pane Color" picker
            // (pane-color-menu.ts's setHue). The explicit choice must win
            // whenever it's present, or picking a hue on an agent pane
            // would have no visible effect (reagent P1, PR #2477) — hue
            // is therefore checked LAST. Clearing the hue picker sets
            // frame:hue to `null` (not delete), which correctly falls
            // through here (typeof null !== "number") back to the
            // agent's default color rather than to no color at all.
            if (bd?.meta?.["frame:activebordercolor"]) {
                style["border-color"] = bd.meta["frame:activebordercolor"];
            }
            const hue = bd?.meta?.["frame:hue"];
            if (typeof hue === "number") {
                style["border-color"] = hueToActiveBorder(hue);
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
    // With only one pane in the tab, there's nothing to distinguish
    // "focused" from "unfocused" against, so the focus ring carries no
    // signal — same reasoning as the floating-pane-workspace's always-on
    // suppression (frontend/app/workspace/floating-pane-workspace.scss).
    const isAlone = () => nodeModel.numLeafs() <= 1;
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
    // Captured outer-frame ref for PaneSizeBadge. Live as long as the
    // frame is mounted; cleared on unmount via the callback ref.
    // SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md.
    const [frameEl, setFrameEl] = createSignal<HTMLDivElement | undefined>(undefined);

    // Agent color for border — matches header color on agent-loaded terminals.
    // Gated by isUsableFocusRingColor: the focused ring is the selection
    // affordance, so identity colors that would render it invisible
    // (near-black / transparent / unparseable) fall back to the accent ring.
    // The header keeps the raw color (see BlockFrame_Header's agentColor).
    const blockAgentColor = createMemo(() => {
        if (!props.preview && blockData()?.meta?.view === "term") {
            const blockEnv = blockData()?.meta?.["cmd:env"] as Record<string, string> | undefined;
            const agentId = detectAgentFromEnv(blockEnv);
            if (agentId) {
                const color = detectAgentColor(blockEnv, agentId);
                if (isUsableFocusRingColor(color)) {
                    return color;
                }
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
        const menu: ContextMenuItem[] = [];
        const bodyItems = props.viewModel?.getBodyContextMenuItems?.();

        if (bodyItems && bodyItems.length > 0) {
            menu.push(...bodyItems, { type: "separator" });
        }
        menu.push(...buildPaneContextMenu(blockData(), {
            magnified: isMagnified(),
            onMagnifyToggle: nodeModel.toggleMagnify,
            onClose: nodeModel.onClose,
            inspectAt: { x: e.clientX, y: e.clientY },
        }, props.viewModel));
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            class={clsx("block", "block-frame-default", "block-" + nodeModel.blockId, {
                "block-focused": isFocused() || props.preview,
                "block-preview": props.preview,
                "has-agent-color": !!blockAgentColor(),
                "pane-alone": isAlone(),
                ephemeral: isEphemeral(),
                magnified: isMagnified(),
            })}
            data-blockid={nodeModel.blockId}
            onClick={props.blockModel?.onClick}
            onFocusIn={props.blockModel?.onFocusCapture}
            onContextMenu={onBodyContextMenu}
            ref={(el) => {
                setFrameEl(el);
                if (props.blockModel?.blockRef) {
                    props.blockModel.blockRef.current = el;
                }
            }}
            style={
                {
                    "--magnified-block-opacity": magnifiedBlockOpacity(),
                    "--magnified-block-blur": `${magnifiedBlockBlur()}px`,
                    "--block-agent-color": blockAgentColor() ?? "transparent",
                } as JSX.CSSProperties
            }
            inert={props.preview || undefined}
        >
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
            {/* Pane size badge — bottom-left WxH while a splitter is
                being dragged. Gated on `isSplitterDragging` (not
                `isResizing`) so a window-resize tick doesn't flash the
                badge on every pane. The Show keeps the component (and
                its ResizeObserver) mounted only during the drag, so
                idle panes do no observer work. Codex P2 rounds 1 + 2
                on PR #1057.
                SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md. */}
            <Show when={!props.preview && nodeModel.isSplitterDragging()}>
                <PaneSizeBadge target={frameEl} />
            </Show>
            {/* BlockMask is last in DOM so it paints above all block content,
                including hardware-accelerated WebGL surfaces */}
            <BlockMask nodeModel={nodeModel} />
        </div>
    );
}

function BlockFrame_Default(props: BlockFrameProps): JSX.Element {
    return <BlockFrame_Default_Component {...props} />;
}

function BlockFrame(props: BlockFrameProps): JSX.Element {
    const blockId = props.nodeModel.blockId;
    const [blockData] = WOS.useWaveObjectValue<Block>(WOS.makeORef("block", blockId));
    return (
        <Show when={blockId && blockData()}>
            <BlockFrame_Default {...props} />
        </Show>
    );
}

export { BlockFrame, NumActiveConnColors };
