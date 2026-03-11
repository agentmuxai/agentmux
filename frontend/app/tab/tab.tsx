// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, globalStore, recordTEvent, refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { Button } from "@/element/button";
import { ContextMenuModel } from "@/store/contextmenu";
import { fireAndForget } from "@/util/util";
import clsx from "clsx";
import { useAtomValue } from "jotai";
import { forwardRef, memo, useCallback, useEffect, useImperativeHandle, useRef, useState } from "react";
import ReactDOM from "react-dom";
import { ObjectService } from "../store/services";
import { makeORef, useWaveObjectValue } from "../store/wos";
import { TabBarModel } from "./tabbar-model";
import "./tab.scss";

// 16-color palette arranged in a 4x4 grid
const TAB_COLORS: { name: string; hex: string | null }[] = [
    { name: "Red",    hex: "#ef4444" },
    { name: "Orange", hex: "#f97316" },
    { name: "Amber",  hex: "#f59e0b" },
    { name: "Yellow", hex: "#eab308" },
    { name: "Lime",   hex: "#84cc16" },
    { name: "Green",  hex: "#22c55e" },
    { name: "Teal",   hex: "#14b8a6" },
    { name: "Cyan",   hex: "#06b6d4" },
    { name: "Blue",   hex: "#3b82f6" },
    { name: "Indigo", hex: "#6366f1" },
    { name: "Violet", hex: "#8b5cf6" },
    { name: "Purple", hex: "#a855f7" },
    { name: "Pink",   hex: "#ec4899" },
    { name: "Rose",   hex: "#f43f5e" },
    { name: "Slate",  hex: "#64748b" },
    { name: "None",   hex: null },
];

interface TabColorPickerProps {
    anchor: DOMRect;
    currentColor: string | null | undefined;
    onSelect: (hex: string | null) => void;
    onClose: () => void;
}

const TabColorPicker = memo(({ anchor, currentColor, onSelect, onClose }: TabColorPickerProps) => {
    const pickerRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
        const handleClickOutside = (e: MouseEvent) => {
            if (pickerRef.current && !pickerRef.current.contains(e.target as Node)) {
                onClose();
            }
        };
        const handleKeyDown = (e: KeyboardEvent) => {
            if (e.key === "Escape") onClose();
        };
        document.addEventListener("mousedown", handleClickOutside);
        document.addEventListener("keydown", handleKeyDown);
        return () => {
            document.removeEventListener("mousedown", handleClickOutside);
            document.removeEventListener("keydown", handleKeyDown);
        };
    }, [onClose]);

    const style: React.CSSProperties = {
        position: "fixed",
        top: anchor.bottom + 4,
        left: anchor.left,
        zIndex: 9999,
    };

    return ReactDOM.createPortal(
        <div ref={pickerRef} className="tab-color-picker" style={style}>
            {TAB_COLORS.map(({ name, hex }) => (
                <div
                    key={name}
                    className={clsx("tab-color-swatch", { selected: (currentColor ?? null) === hex })}
                    title={name}
                    style={hex ? { backgroundColor: hex } : undefined}
                    onClick={() => onSelect(hex)}
                >
                    {!hex && <i className="fa fa-xmark" />}
                </div>
            ))}
        </div>,
        document.body
    );
});
TabColorPicker.displayName = "TabColorPicker";

interface TabProps {
    id: string;
    active: boolean;
    isFirst: boolean;
    isBeforeActive: boolean;
    isDragging: boolean;
    tabWidth: number;
    isNew: boolean;
    isPinned: boolean;
    onSelect: () => void;
    onClose: (event: React.MouseEvent<HTMLButtonElement, MouseEvent> | null) => void;
    onDragStart: (event: React.MouseEvent<HTMLDivElement, MouseEvent>) => void;
    onLoaded: () => void;
    onPinChange: () => void;
}

const Tab = memo(
    forwardRef<HTMLDivElement, TabProps>(
        (
            {
                id,
                active,
                isPinned,
                isBeforeActive,
                isDragging,
                tabWidth,
                isNew,
                onLoaded,
                onSelect,
                onClose,
                onDragStart,
                onPinChange,
            },
            ref
        ) => {
            const [tabData, _] = useWaveObjectValue<Tab>(makeORef("tab", id));
            const [originalName, setOriginalName] = useState("");
            const [isEditable, setIsEditable] = useState(false);
            const [isJiggling, setIsJiggling] = useState(false);
            const [showColorPicker, setShowColorPicker] = useState(false);
            const [colorPickerAnchor, setColorPickerAnchor] = useState<DOMRect | null>(null);

            const jiggleTrigger = useAtomValue(TabBarModel.getInstance().jigglePinAtom);

            const editableRef = useRef<HTMLDivElement>(null);
            const editableTimeoutRef = useRef<NodeJS.Timeout>(null);
            const loadedRef = useRef(false);
            const tabRef = useRef<HTMLDivElement>(null);

            useImperativeHandle(ref, () => tabRef.current as HTMLDivElement);

            const tabColor = tabData?.meta?.["tab:color"] as string | undefined | null;

            useEffect(() => {
                if (tabData?.name) {
                    setOriginalName(tabData.name);
                }
            }, [tabData]);

            useEffect(() => {
                return () => {
                    if (editableTimeoutRef.current) {
                        clearTimeout(editableTimeoutRef.current);
                    }
                };
            }, []);

            const selectEditableText = useCallback(() => {
                if (editableRef.current) {
                    const range = document.createRange();
                    const selection = window.getSelection();
                    range.selectNodeContents(editableRef.current);
                    selection.removeAllRanges();
                    selection.addRange(range);
                }
            }, []);

            const handleRenameTab: React.MouseEventHandler<HTMLDivElement> = (event) => {
                event?.stopPropagation();
                setIsEditable(true);
                editableTimeoutRef.current = setTimeout(() => {
                    selectEditableText();
                }, 0);
            };

            const handleBlur = () => {
                let newText = editableRef.current.innerText.trim();
                newText = newText || originalName;
                editableRef.current.innerText = newText;
                setIsEditable(false);
                fireAndForget(() => ObjectService.UpdateTabName(id, newText));
                setTimeout(() => refocusNode(null), 10);
            };

            const handleKeyDown: React.KeyboardEventHandler<HTMLDivElement> = (event) => {
                if ((event.metaKey || event.ctrlKey) && event.key === "a") {
                    event.preventDefault();
                    selectEditableText();
                    return;
                }
                // this counts glyphs, not characters
                const curLen = Array.from(editableRef.current.innerText).length;
                if (event.key === "Enter") {
                    event.preventDefault();
                    event.stopPropagation();
                    if (editableRef.current.innerText.trim() === "") {
                        editableRef.current.innerText = originalName;
                    }
                    editableRef.current.blur();
                } else if (event.key === "Escape") {
                    editableRef.current.innerText = originalName;
                    editableRef.current.blur();
                    event.preventDefault();
                    event.stopPropagation();
                } else if (curLen >= 14 && !["Backspace", "Delete", "ArrowLeft", "ArrowRight"].includes(event.key)) {
                    event.preventDefault();
                    event.stopPropagation();
                }
            };

            useEffect(() => {
                if (!loadedRef.current) {
                    onLoaded();
                    loadedRef.current = true;
                }
            }, [onLoaded]);

            useEffect(() => {
                if (tabRef.current && isNew) {
                    const initialWidth = `${(tabWidth / 3) * 2}px`;
                    tabRef.current.style.setProperty("--initial-tab-width", initialWidth);
                    tabRef.current.style.setProperty("--final-tab-width", `${tabWidth}px`);
                }
            }, [isNew, tabWidth]);

            useEffect(() => {
                if (active && isPinned && jiggleTrigger > 0) {
                    setIsJiggling(true);
                    const timeout = setTimeout(() => {
                        setIsJiggling(false);
                    }, 500);
                    return () => clearTimeout(timeout);
                }
            }, [jiggleTrigger, active, isPinned]);

            // Prevent drag from being triggered on mousedown
            const handleMouseDownOnClose = (event: React.MouseEvent<HTMLButtonElement, MouseEvent>) => {
                event.stopPropagation();
            };

            const handleColorSelect = useCallback(
                (hex: string | null) => {
                    const oref = makeORef("tab", id);
                    fireAndForget(async () => {
                        await ObjectService.UpdateObjectMeta(oref, { "tab:color": hex });
                    });
                    setShowColorPicker(false);
                },
                [id]
            );

            const handleContextMenu = useCallback(
                (e: React.MouseEvent<HTMLDivElement, MouseEvent>) => {
                    e.preventDefault();
                    let menu: ContextMenuItem[] = [
                        { label: isPinned ? "Unpin Tab" : "Pin Tab", click: () => onPinChange() },
                        { label: "Rename Tab", click: () => handleRenameTab(null) },
                        {
                            label: "Copy TabId",
                            click: () => fireAndForget(() => navigator.clipboard.writeText(id)),
                        },
                        { type: "separator" },
                        {
                            label: "Color",
                            click: () => {
                                const rect = tabRef.current?.getBoundingClientRect();
                                if (rect) {
                                    setColorPickerAnchor(rect);
                                    setShowColorPicker(true);
                                }
                            },
                        },
                        { type: "separator" },
                    ];
                    const fullConfig = globalStore.get(atoms.fullConfigAtom);
                    const bgPresets: string[] = [];
                    for (const key in fullConfig?.presets ?? {}) {
                        if (key.startsWith("bg@")) {
                            bgPresets.push(key);
                        }
                    }
                    bgPresets.sort((a, b) => {
                        const aOrder = fullConfig.presets[a]["display:order"] ?? 0;
                        const bOrder = fullConfig.presets[b]["display:order"] ?? 0;
                        return aOrder - bOrder;
                    });
                    if (bgPresets.length > 0) {
                        const submenu: ContextMenuItem[] = [];
                        const oref = makeORef("tab", id);
                        for (const presetName of bgPresets) {
                            const preset = fullConfig.presets[presetName];
                            if (preset == null) {
                                continue;
                            }
                            submenu.push({
                                label: preset["display:name"] ?? presetName,
                                click: () =>
                                    fireAndForget(async () => {
                                        await ObjectService.UpdateObjectMeta(oref, preset);
                                        RpcApi.ActivityCommand(TabRpcClient, { settabtheme: 1 }, { noresponse: true });
                                        recordTEvent("action:settabtheme");
                                    }),
                            });
                        }
                        menu.push({ label: "Backgrounds", type: "submenu", submenu }, { type: "separator" });
                    }
                    menu.push({ label: "Close Tab", click: () => onClose(null) });
                    ContextMenuModel.showContextMenu(menu, e);
                },
                [onPinChange, handleRenameTab, id, onClose, isPinned]
            );

            return (
                <>
                    <div
                        ref={tabRef}
                        className={clsx("tab", {
                            active,
                            dragging: isDragging,
                            "before-active": isBeforeActive,
                            "new-tab": isNew,
                            "tab-colored": !!tabColor,
                        })}
                        style={tabColor ? ({ "--tab-color": tabColor } as React.CSSProperties) : undefined}
                        onMouseDown={onDragStart}
                        onClick={onSelect}
                        onContextMenu={handleContextMenu}
                        data-tab-id={id}
                    >
                        <div className="tab-inner">
                            <div
                                ref={editableRef}
                                className={clsx("name", { focused: isEditable })}
                                contentEditable={isEditable}
                                onDoubleClick={handleRenameTab}
                                onBlur={handleBlur}
                                onKeyDown={handleKeyDown}
                                suppressContentEditableWarning={true}
                            >
                                {tabData?.name}
                            </div>
                            {isPinned ? (
                                <Button
                                    className={clsx("ghost grey pin", { jiggling: isJiggling })}
                                    onClick={(e) => {
                                        e.stopPropagation();
                                        onPinChange();
                                    }}
                                    title="Unpin Tab"
                                >
                                    <i className="fa fa-solid fa-thumbtack" />
                                </Button>
                            ) : (
                                <Button
                                    className="ghost grey close"
                                    onClick={onClose}
                                    onMouseDown={handleMouseDownOnClose}
                                    title="Close Tab"
                                >
                                    <i className="fa fa-solid fa-xmark" />
                                </Button>
                            )}
                        </div>
                    </div>
                    {showColorPicker && colorPickerAnchor && (
                        <TabColorPicker
                            anchor={colorPickerAnchor}
                            currentColor={tabColor}
                            onSelect={handleColorSelect}
                            onClose={() => setShowColorPicker(false)}
                        />
                    )}
                </>
            );
        }
    )
);

export { Tab };
