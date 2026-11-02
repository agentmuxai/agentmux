// Copyright 2025, Command Line Inc.
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Button } from "@/element/button";
import { Button as UiButton } from "@/element/ui";
import { fireAndForget } from "@/util/util";
import clsx from "clsx";
import { createEffect, createSignal, onCleanup, onMount, Show, on } from "solid-js";
import { Portal } from "solid-js/web";
import type { JSX } from "solid-js";
import { PaletteSwatches } from "@/app/components/palette-swatches";
import { flashElement, onActivityFlash } from "@/app/notification/activity-flash";
import { paneRoleColor } from "../block/pane-color-scheme";
import { isLightThemeActive } from "../block/pane-identity";
import { ObjectService } from "../store/services";
import { makeORef, useMuxObjectValue } from "../store/mos";
import { measureTabWidth } from "./tab-measure";
import { Z_TRANSIENT } from "../zindex";
import "./tab.scss";
import { renameRequest } from "./tab-rename-request";

// The stored hex only names a hue: one of the shared palette's 16 slots
// (theme/palette.ts), the same hues panes, agents and browser profiles use.
// Sharing the hues cannot recolour a pane border: the tab is drawn in the
// pane-tab pill tones for that hue (pane-color-scheme.ts), so a window tab and
// a pane tab of one colour read as the same family in dark and light themes.
export function windowTabToneVars(hex: string, isLightTheme: boolean): Record<string, string> | undefined {
    const bg = paneRoleColor(undefined, hex, isLightTheme, "pill");
    const bgActive = paneRoleColor(undefined, hex, isLightTheme, "pillActive");
    const underline = paneRoleColor(undefined, hex, isLightTheme, "identity");
    if (!bg || !bgActive || !underline) return undefined;
    return { "--tab-bg": bg, "--tab-bg-active": bgActive, "--tab-underline": underline };
}

interface TabContextPanelProps {
    anchor: DOMRect;
    currentColor: string | null | undefined;
    onColorSelect: (hex: string | null) => void;
    onRename: () => void;
    onClose: (e?: MouseEvent) => void;
}

const TabContextPanel = (props: TabContextPanelProps): JSX.Element => {
    let panelRef!: HTMLDivElement;

    onMount(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (panelRef && !panelRef.contains(e.target as Node)) {
                props.onClose();
            }
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") props.onClose();
        };
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown);
        onCleanup(() => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown);
        });
    });

    const style = (): JSX.CSSProperties => ({
        position: "fixed",
        top: `${props.anchor.bottom + 4}px`,
        left: `${props.anchor.left}px`,
        "z-index": Z_TRANSIENT,
    });

    return (
        <Portal>
            <div ref={panelRef!} class="tab-context-panel" style={style()} data-pane-overlay>
                <PaletteSwatches
                    label="Tab color"
                    value={props.currentColor ?? null}
                    noneLabel="No color"
                    onSelect={(slot) => props.onColorSelect(slot ? slot.hex : null)}
                />
                <div class="tab-context-actions">
                    <UiButton density="compact" icon="pen" class="tab-context-btn" onClick={() => { props.onRename(); props.onClose(); }}>
                        Rename
                    </UiButton>
                    <UiButton density="compact" icon="xmark" class="tab-context-btn tab-context-btn-close" onClick={() => props.onClose()}>
                        Close menu
                    </UiButton>
                </div>
            </div>
        </Portal>
    );
};

interface TabProps {
    id: string;
    active: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    isDragging: boolean;
    tabWidth: number;
    isNew: boolean;
    onSelect: () => void;
    onClose: (event: MouseEvent | null) => void;
    onDragStart: (event: DragEvent) => void;
    onLoaded: () => void;
    onNaturalWidth?: (width: number) => void;
}

function Tab(props: TabProps): JSX.Element {
    const [tabData] = useMuxObjectValue<Tab>(makeORef("tab", props.id));
    const [originalName, setOriginalName] = createSignal("");
    const [isEditable, setIsEditable] = createSignal(false);
    const [showColorPicker, setShowColorPicker] = createSignal(false);
    const [colorPickerAnchor, setColorPickerAnchor] = createSignal<DOMRect | null>(null);

    let editableRef!: HTMLDivElement;
    let tabRef!: HTMLDivElement;
    let tabInnerRef!: HTMLDivElement;
    let editableTimeoutId: ReturnType<typeof setTimeout> | null = null;
    let loadedRef = false;

    const tabColor = (): string | undefined | null => tabData()?.meta?.["tab:color"] as string | undefined | null;
    const tabTone = () => {
        const hex = tabColor();
        return hex ? windowTabToneVars(hex, isLightThemeActive()) : undefined;
    };

    createEffect(() => {
        const name = tabData()?.name;
        if (name) {
            setOriginalName(name);
        }
    });

    createEffect(() => {
        const name = tabData()?.name;
        if (name && props.onNaturalWidth) {
            props.onNaturalWidth(measureTabWidth(name));
        }
    });

    onCleanup(() => {
        if (editableTimeoutId) {
            clearTimeout(editableTimeoutId);
        }
    });

    const selectEditableText = () => {
        if (editableRef) {
            const range = document.createRange();
            const selection = window.getSelection();
            range.selectNodeContents(editableRef);
            selection.removeAllRanges();
            selection.addRange(range);
        }
    };

    const handleRenameTab = (event?: MouseEvent) => {
        event?.stopPropagation();
        setIsEditable(true);
        editableTimeoutId = setTimeout(() => {
            selectEditableText();
        }, 0);
    };

    // A shortcut asked this tab to start renaming (tab-rename-request.ts).
    createEffect(
        on(renameRequest, (req) => {
            if (req?.tabId === props.id) handleRenameTab();
        }, { defer: true })
    );

    const handleBlur = () => {
        let newText = editableRef.innerText.trim();
        newText = newText || originalName();
        editableRef.innerText = newText;
        setIsEditable(false);
        props.onNaturalWidth?.(measureTabWidth(newText));
        fireAndForget(() => ObjectService.UpdateTabName(props.id, newText));
        setTimeout(() => refocusNode(null), 10);
    };

    const handleRenameInput = () => {
        if (!editableRef || !props.onNaturalWidth) return;
        const text = editableRef.innerText || originalName();
        props.onNaturalWidth(measureTabWidth(text));
    };

    const handleKeyDown = (event: KeyboardEvent) => {
        if ((event.metaKey || event.ctrlKey) && event.key === "a") {
            event.preventDefault();
            selectEditableText();
            return;
        }
        const curLen = Array.from(editableRef.innerText).length;
        if (event.key === "Enter") {
            event.preventDefault();
            event.stopPropagation();
            if (editableRef.innerText.trim() === "") {
                editableRef.innerText = originalName();
            }
            editableRef.blur();
        } else if (event.key === "Escape") {
            editableRef.innerText = originalName();
            editableRef.blur();
            event.preventDefault();
            event.stopPropagation();
        } else if (curLen >= 128 && !["Backspace", "Delete", "ArrowLeft", "ArrowRight"].includes(event.key)) {
            event.preventDefault();
            event.stopPropagation();
        }
    };

    createEffect(() => {
        if (!loadedRef) {
            props.onLoaded();
            loadedRef = true;
        }
    });

    // The width-animation effect that used to live here wrote
    // `--initial-tab-width` / `--final-tab-width` CSS vars from
    // `props.tabWidth`, which has always been 0 (dead code). The
    // companion `expand-width-and-fade-in` keyframe in tab.scss
    // ran with `forwards`, pinning every new tab's width to 0 px
    // (clamped up to `min-width: 60px`) for its entire lifetime.
    // That was the source of the "tabs don't resize when renamed"
    // bug. The new-tab fade is now opacity-only; tabs size to
    // their content via flex layout. See the analysis at
    // docs/retro/RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md.

    // Visual twin of a tool-call tone from any pane in THIS tab, active or
    // not — docs/specs/SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md. A subtle
    // lift of the tab's own color (tab.scss); the pane's color is its pill's.
    onMount(() => {
        const unsubscribe = onActivityFlash((target) => {
            if (tabData()?.blockids?.includes(target.blockId)) flashElement(tabInnerRef, target);
        });
        onCleanup(unsubscribe);
    });

    const handleMouseDownOnClose = (event: MouseEvent) => {
        event.stopPropagation();
    };

    // Without this, a click on the close button bubbles past the button's
    // own onClick to the parent .tab div's onClick={props.onSelect} (only
    // mousedown propagation is stopped above), spuriously selecting the tab
    // that's simultaneously being closed. That races SetActiveTab against
    // CloseTab on the backend and produces a visible select/deselect flash —
    // see docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md.
    const handleCloseClick = (event: MouseEvent) => {
        event.stopPropagation();
        props.onClose(event);
    };

    const handleColorSelect = (hex: string | null) => {
        const oref = makeORef("tab", props.id);
        fireAndForget(async () => {
            await ObjectService.UpdateObjectMeta(oref, { "tab:color": hex } as MetaType);
        });
        setShowColorPicker(false);
    };

    const handleContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        e.stopPropagation();
        const rect = tabRef?.getBoundingClientRect();
        if (rect) {
            setColorPickerAnchor(rect);
            setShowColorPicker(true);
        }
    };


    return (
        <>
            <div
                ref={tabRef!}
                class={clsx("tab", {
                    active: props.active,
                    dragging: props.isDragging,
                    "before-active": props.isBeforeActive,
                    "new-tab": props.isNew,
                    "tab-colored": !!tabTone(),
                })}
                style={tabTone() as JSX.CSSProperties | undefined}
                onClick={props.onSelect}
                onContextMenu={handleContextMenu}
                data-tab-id={props.id}
                data-drag-region="false"
            >
                <div ref={tabInnerRef!} class="tab-inner">
                    <div
                        ref={editableRef!}
                        class={clsx("name", { focused: isEditable() })}
                        contentEditable={isEditable()}
                        // Only block native drag while actively renaming — text
                        // selection/caret placement over the auto-selected name
                        // must win over pragmatic-dnd's wrapper draggable="true"
                        // there (reagent P1, PR #2148). Unset otherwise so the
                        // name area — the tab's largest surface — still starts
                        // a reorder drag normally.
                        draggable={isEditable() ? false : undefined}
                        onDblClick={() => handleRenameTab()}
                        onBlur={handleBlur}
                        onKeyDown={handleKeyDown}
                        onInput={handleRenameInput}
                    >
                        {tabData()?.name}
                    </div>
                    <Button
                        className="ghost grey close"
                        onClick={handleCloseClick}
                        onMouseDown={handleMouseDownOnClose}
                        title="Close Tab"
                        draggable={false}
                    >
                        {/* The same X as every other close in the app: Free's `xmark`
                            (docs/reports/REPORT_PANE_HEADER_ICONS_FA_FREE_2026_10_11.md §8, D5). */}
                        <i class="fa-solid fa-xmark" aria-hidden="true" />
                    </Button>
                </div>
            </div>
            <Show when={showColorPicker() && colorPickerAnchor()}>
                <TabContextPanel
                    anchor={colorPickerAnchor()}
                    currentColor={tabColor()}
                    onColorSelect={handleColorSelect}
                    onRename={() => handleRenameTab()}
                    onClose={() => setShowColorPicker(false)}
                />
            </Show>
        </>
    );
}

export { Tab };
