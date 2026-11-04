// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The window rows of the InstancePanel: an open window's row (OpenWindowRow with its name, rename
 * and opacity control wired up), a closed window's row, and the one list that holds both in their
 * slots (window-slots.ts).
 */

import { type WindowEntry } from "@/store/global";
import { type ClosedWindow } from "@/store/services";
import { MENU_ICON } from "@/app/util/menu-icons";
import { getObjectValue, makeORef } from "@/store/mos";
import { opacityFor } from "@/app/store/window-appearance";
import { settingsAtom } from "@/app/store/config-signals";
import { For, Show, type Accessor, type JSX } from "solid-js";
import { IconButton, openMenuAt } from "@/app/element/ui";
import { closedWindowLabel } from "./window-slots";
import { isHeldWindow } from "./window-slot-keys";
import { closedAtLabel } from "@/app/store/user-workspace";
import { tip } from "@/app/element/tip";
import { OpenWindowRow } from "./OpenWindowRow";
import type { WindowRename } from "./instance-panel-state";

// A closed window's row: its name and how many of its agents run without it. That it is closed shows in
// the row (greyed, a reopen button), so the row does not say so; the tooltip has the time. A row held
// until srv lists the close has nothing to reopen or remove yet: it only keeps its place.
export const ClosedRow = (props: { w: ClosedWindow; reopen: (w: ClosedWindow) => void; remove: (w: ClosedWindow) => void }): JSX.Element => (
    <Show
        when={!isHeldWindow(props.w)}
        fallback={
            <div class="instance-panel-window-row instance-panel-window-row-closed" {...tip("Closed")}>
                <span class="instance-panel-window-dot">◌</span>
                <span class="instance-panel-window-name">{closedWindowLabel(props.w.name, props.w.running)}</span>
            </div>
        }
    >
        <div
            class="instance-panel-window-row instance-panel-window-row-closed"
            {...tip(`${closedAtLabel(props.w.closed_at_ms)}. Click to reopen, right-click to remove`)}
            role="button"
            tabIndex={0}
            onClick={() => props.reopen(props.w)}
            onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    props.reopen(props.w);
                }
            }}
            onContextMenu={(e) => {
                e.preventDefault();
                e.stopPropagation();
                openMenuAt(
                    [{ label: "Remove from workspace", icon: MENU_ICON.close, click: () => props.remove(props.w) }],
                    e
                );
            }}
        >
            <span class="instance-panel-window-dot">◌</span>
            <span class="instance-panel-window-name">{closedWindowLabel(props.w.name, props.w.running)}</span>
            <IconButton
                icon="rotate-left"
                density="compact"
                class="instance-panel-row-action"
                label="Reopen"
                onClick={(e) => {
                    e.stopPropagation();
                    props.reopen(props.w);
                }}
                onKeyDown={(e) => e.stopPropagation()}
            />
        </div>
    </Show>
);

/** The inline opacity slider (InstancePanel.tsx): `persistWindowId` null for a floating pane, whose opacity is session-only. */
export type OpacityControlComponent = (p: {
    label: string;
    persistWindowId: string | null;
    currentOpacity: () => number;
}) => JSX.Element;

/** What an open window's row needs from the panel. */
export interface OpenRowContext {
    OpacityControl: OpacityControlComponent;
    entries: Accessor<WindowEntry[]>;
    myLabel: Accessor<string | null>;
    rename: WindowRename;
    resolveName: (entry: WindowEntry, idx: number) => string;
    onlyOpenWindow: () => boolean;
    focusWindow: (label: string) => Promise<void>;
    remove: (entry: WindowEntry) => void;
}

/** An open window's row. */
export const renderOpenRow = (entry: WindowEntry, c: OpenRowContext): JSX.Element => {
    const i = () => c.entries().findIndex((e) => e.label === entry.label);
    const isCurrent = () => entry.label === c.myLabel();
    const isEditing = () => c.rename.editingLabel() === entry.label;
    const currentName = () => c.resolveName(entry, i());
    const currentOpacity = () => {
        if (!entry.windowId) return 1.0;
        const win = getObjectValue<MuxWindow>(makeORef("window", entry.windowId));
        return opacityFor(win?.meta, settingsAtom());
    };
    return (
        <OpenWindowRow
            entry={entry}
            isCurrent={isCurrent}
            isEditing={isEditing}
            name={currentName}
            opacityControl={() => <c.OpacityControl label={entry.label} persistWindowId={entry.windowId} currentOpacity={currentOpacity} />}
            editValue={c.rename.editValue}
            onlyOpenWindow={c.onlyOpenWindow}
            setEditValue={c.rename.setEditValue}
            focus={() => void c.focusWindow(entry.label)}
            enterRename={() => c.rename.enterRename(entry, currentName())}
            commitRename={() => c.rename.commitRename(entry)}
            cancelRename={c.rename.cancelRename}
            remove={() => c.remove(entry)}
        />
    );
};

/** One list: a window keeps its slot whether it is open or closed (window-slots.ts). */
export const WindowSlotList = (props: {
    slotKeys: Accessor<string[]>;
    openByKey: Accessor<Map<string, WindowEntry>>;
    closedByKey: Accessor<Map<string, ClosedWindow>>;
    context: OpenRowContext;
    reopen: (w: ClosedWindow) => void;
    removeClosed: (w: ClosedWindow) => void;
}): JSX.Element => (
    <For each={props.slotKeys()}>
        {(key) => (
            <>
                <Show when={props.openByKey().get(key)} keyed>
                    {(entry) => renderOpenRow(entry, props.context)}
                </Show>
                <Show when={props.closedByKey().get(key)} keyed>
                    {(w) => <ClosedRow w={w} reopen={props.reopen} remove={props.removeClosed} />}
                </Show>
            </>
        )}
    </For>
);
