// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { atoms, recordTEvent, refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/rpc-api";
import { TabRpcClient } from "@/app/store/rpc-util";
import { Button } from "@/element/button";
import { fireAndForget } from "@/util/util";
import clsx from "clsx";
import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import { Portal } from "solid-js/web";
import type { JSX } from "solid-js";
import { ColorSwatchPalette } from "@/app/components/color-swatch-palette";
import { ObjectService } from "../store/services";
import { makeORef, useWaveObjectValue } from "../store/wos";
import { measureTabWidth } from "./tab-measure";
import "./tab.scss";

// 14 colors — same hues as the agent-pane border palette
// (agent-color.ts's AGENT_COLOR_PALETTE), desaturated to roughly halfway
// between the original Tailwind-500 vivid hues and a fully muted
// (S=45%/L=32%) set — per-hue lightness nudged down slightly where needed
// to keep WCAG AA (>=4.5:1) contrast against the existing white tab-label
// text. Deliberately a separate array, not derived from the agent border
// palette — see docs/specs/SPEC_TAB_COLOR_DESATURATION_2026_08_13.md for
// why the two must not share one source (editing this must never affect
// pane borders).
export const TAB_COLORS: { name: string; hex: string }[] = [
    { name: "Red",     hex: "#c22a2a" },
    { name: "Orange",  hex: "#b75e20" },
    { name: "Amber",   hex: "#9d6d1d" },
    { name: "Yellow",  hex: "#90731a" },
    { name: "Lime",    hex: "#5a821e" },
    { name: "Green",   hex: "#248749" },
    { name: "Teal",    hex: "#1e8479" },
    { name: "Cyan",    hex: "#1a8294" },
    { name: "Blue",    hex: "#2562c5" },
    { name: "Indigo",  hex: "#2d30cf" },
    { name: "Violet",  hex: "#5c29d2" },
    { name: "Fuchsia", hex: "#ae2ac2" },
    { name: "Pink",    hex: "#c02b75" },
    { name: "Rose",    hex: "#c42742" },
];

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
        "z-index": 9999,
    });

    return (
        <Portal>
            <div ref={panelRef!} class="tab-context-panel" style={style()} data-pane-overlay>
                <ColorSwatchPalette
                    colors={TAB_COLORS}
                    columns={7}
                    currentColor={props.currentColor}
                    onSelect={props.onColorSelect}
                />
                <div class="tab-context-actions">
                    <button class="tab-context-btn" onClick={() => { props.onRename(); props.onClose(); }}>
                        ✏️ Rename
                    </button>
                    <button class="tab-context-btn tab-context-btn-close" onClick={() => props.onClose()}>
                        ✕ Close menu
                    </button>
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
    const [tabData] = useWaveObjectValue<Tab>(makeORef("tab", props.id));
    const [originalName, setOriginalName] = createSignal("");
    const [isEditable, setIsEditable] = createSignal(false);
    const [showColorPicker, setShowColorPicker] = createSignal(false);
    const [colorPickerAnchor, setColorPickerAnchor] = createSignal<DOMRect | null>(null);

    let editableRef!: HTMLDivElement;
    let tabRef!: HTMLDivElement;
    let editableTimeoutId: ReturnType<typeof setTimeout> | null = null;
    let loadedRef = false;

    const tabColor = (): string | undefined | null => tabData()?.meta?.["tab:color"] as string | undefined | null;

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
                    "tab-colored": !!tabColor(),
                })}
                style={tabColor() ? ({ "--tab-color": tabColor() } as JSX.CSSProperties) : undefined}
                onClick={props.onSelect}
                onContextMenu={handleContextMenu}
                data-tab-id={props.id}
                data-drag-region="false"
            >
                <div class="tab-inner">
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
                        {/* VS Code's "close" codicon, inlined as SVG so we
                            don't pull in the codicons font. Path verbatim
                            from microsoft/vscode-codicons (close.svg) —
                            the same glyph VS Code uses for tab-close. */}
                        <svg
                            width="16"
                            height="16"
                            viewBox="0 0 16 16"
                            fill="currentColor"
                            xmlns="http://www.w3.org/2000/svg"
                            aria-hidden="true"
                        >
                            <path
                                fill-rule="evenodd"
                                clip-rule="evenodd"
                                d="M8 8.707l3.646 3.647.708-.707L8.707 8l3.647-3.646-.707-.708L8 7.293 4.354 3.646l-.707.708L7.293 8l-3.646 3.646.707.708L8 8.707z"
                            />
                        </svg>
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
