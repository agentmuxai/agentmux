// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

import { atoms, globalStore, recordTEvent, refocusNode } from "@/app/store/global";
import { RpcApi } from "@/app/store/wshclientapi";
import { TabRpcClient } from "@/app/store/wshrpcutil";
import { Button } from "@/element/button";
import { ContextMenuModel } from "@/store/contextmenu";
import { fireAndForget } from "@/util/util";
import clsx from "clsx";
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import type { JSX } from "solid-js";
import { ObjectService } from "../store/services";
import { makeORef, useWaveObjectValue } from "../store/wos";
import { TabBarModel } from "./tabbar-model";
import "./tab.scss";

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
    onClose: (event: MouseEvent | null) => void;
    onDragStart: (event: MouseEvent) => void;
    onLoaded: () => void;
    onPinChange: () => void;
}

function Tab(props: TabProps): JSX.Element {
    const [tabData] = useWaveObjectValue<Tab>(makeORef("tab", props.id));
    const [originalName, setOriginalName] = createSignal("");
    const [isEditable, setIsEditable] = createSignal(false);
    const [isJiggling, setIsJiggling] = createSignal(false);

    const jiggleTrigger = TabBarModel.getInstance().jigglePinAtom;

    let editableRef!: HTMLDivElement;
    let tabRef!: HTMLDivElement;
    let editableTimeoutId: ReturnType<typeof setTimeout> | null = null;
    let loadedRef = false;

    createEffect(() => {
        const name = tabData()?.name;
        if (name) {
            setOriginalName(name);
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
        fireAndForget(() => ObjectService.UpdateTabName(props.id, newText));
        setTimeout(() => refocusNode(null), 10);
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
        } else if (curLen >= 14 && !["Backspace", "Delete", "ArrowLeft", "ArrowRight"].includes(event.key)) {
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

    createEffect(() => {
        if (tabRef && props.isNew) {
            const initialWidth = `${(props.tabWidth / 3) * 2}px`;
            tabRef.style.setProperty("--initial-tab-width", initialWidth);
            tabRef.style.setProperty("--final-tab-width", `${props.tabWidth}px`);
        }
    });

    createEffect(() => {
        const trigger = jiggleTrigger();
        if (props.active && props.isPinned && trigger > 0) {
            setIsJiggling(true);
            const timeout = setTimeout(() => {
                setIsJiggling(false);
            }, 500);
            onCleanup(() => clearTimeout(timeout));
        }
    });

    const handleMouseDownOnClose = (event: MouseEvent) => {
        event.stopPropagation();
    };

    const handleContextMenu = (e: MouseEvent) => {
        e.preventDefault();
        let menu: ContextMenuItem[] = [
            { label: props.isPinned ? "Unpin Tab" : "Pin Tab", click: () => props.onPinChange() },
            { label: "Rename Tab", click: () => handleRenameTab() },
            {
                label: "Copy TabId",
                click: () => fireAndForget(() => navigator.clipboard.writeText(props.id)),
            },
            { type: "separator" },
        ];
        const fullConfig = atoms.fullConfigAtom();
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
            const oref = makeORef("tab", props.id);
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
        menu.push({ label: "Close Tab", click: () => props.onClose(null) });
        ContextMenuModel.showContextMenu(menu, e);
    };

    return (
        <div
            ref={tabRef!}
            class={clsx("tab", {
                active: props.active,
                dragging: props.isDragging,
                "before-active": props.isBeforeActive,
                "new-tab": props.isNew,
            })}
            onMouseDown={props.onDragStart}
            onClick={props.onSelect}
            onContextMenu={handleContextMenu}
            data-tab-id={props.id}
        >
            <div class="tab-inner">
                <div
                    ref={editableRef!}
                    class={clsx("name", { focused: isEditable() })}
                    contentEditable={isEditable()}
                    onDblClick={() => handleRenameTab()}
                    onBlur={handleBlur}
                    onKeyDown={handleKeyDown}
                >
                    {tabData()?.name}
                </div>
                <Show
                    when={props.isPinned}
                    fallback={
                        <Button
                            className="ghost grey close"
                            onClick={props.onClose}
                            onMouseDown={handleMouseDownOnClose}
                            title="Close Tab"
                        >
                            <i class="fa fa-solid fa-xmark" />
                        </Button>
                    }
                >
                    <Button
                        className={clsx("ghost grey pin", { jiggling: isJiggling() })}
                        onClick={(e: MouseEvent) => {
                            e.stopPropagation();
                            props.onPinChange();
                        }}
                        title="Unpin Tab"
                    >
                        <i class="fa fa-solid fa-thumbtack" />
                    </Button>
                </Show>
            </div>
        </div>
    );
}

export { Tab };
