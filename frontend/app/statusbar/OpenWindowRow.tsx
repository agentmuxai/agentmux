// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { Show, type JSX } from "solid-js";
import type { WindowEntry } from "@/store/global";
import { IconButton, Tag } from "@/app/element/ui";
import { tip } from "@/app/element/tip";
import { DISPLAY_NAME_MAX_LEN } from "@/util/window-title";
import { LAST_WINDOW_TITLE } from "@/app/store/user-workspace";

/** What an open window's row reads from the panel, and what it asks the panel to do. */
export interface OpenWindowRowProps {
    entry: WindowEntry;
    isCurrent: () => boolean;
    isEditing: () => boolean;
    name: () => string;
    /** The window's opacity slider, drawn by the panel (it is shared with the floating panes' rows). */
    opacityControl: () => JSX.Element;
    editValue: () => string;
    onlyOpenWindow: () => boolean;
    setEditValue: (value: string) => void;
    focus: () => void;
    enterRename: () => void;
    commitRename: () => void;
    cancelRename: () => void;
    remove: () => void;
}

/** An open window's row. */
export const OpenWindowRow = (props: OpenWindowRowProps): JSX.Element => (
    <div
        class="instance-panel-window-row"
        classList={{
            "instance-panel-window-row-current": props.isCurrent(),
            "instance-panel-window-row-editing": props.isEditing(),
        }}
        onClick={(e) => {
            if (props.isEditing()) {
                e.stopPropagation();
                return;
            }
            // Focus at once, not after a double-click delay: the status bar's outside-click dismiss
            // unmounts the panel before such a timer fires, and the click would do nothing. A
            // double-click still focuses first (harmless), then the dblclick handler enters rename.
            props.focus();
        }}
        onDblClick={(e) => {
            e.preventDefault();
            e.stopPropagation();
            props.enterRename();
        }}
        onKeyDown={(e) => {
            if (props.isEditing()) return; // input owns key handling
            if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                    props.focus();
            } else if (e.key === "F2") {
                e.preventDefault();
                    props.enterRename();
            }
        }}
        {...tip(props.isCurrent() ? "This window — double-click to rename (F2)" : `Click to focus, double-click to rename (F2)`)}
        role="button"
        tabIndex={0}
    >
        <span class="instance-panel-window-dot">{props.isCurrent() ? "●" : "○"}</span>
        <Show when={props.isEditing()} fallback={<span class="instance-panel-window-name">{props.name()}</span>}>
            <input
                ref={(el) => {
                    if (el) {
                        queueMicrotask(() => {
                            el.focus();
                            el.select();
                        });
                    }
                }}
                class="instance-panel-window-name-input" size={1} aria-label="Window name" data-raw-text-field="the window row's inline rename: it takes the name's place and ends on Enter, Escape or blur, not a form field"
                value={props.editValue()}
                maxLength={DISPLAY_NAME_MAX_LEN}
                onInput={(e) => props.setEditValue(e.currentTarget.value)}
                onKeyDown={(e) => {
                    // Stop global key handlers from firing while editing.
                    e.stopPropagation();
                    if (e.key === "Enter") {
                        e.preventDefault();
                        props.commitRename();
                    } else if (e.key === "Escape") {
                        e.preventDefault();
                        props.cancelRename();
                    }
                }}
                onBlur={() => props.commitRename()}
                onClick={(e) => e.stopPropagation()}
                onDblClick={(e) => e.stopPropagation()}
            />
        </Show>
        <Show when={props.isCurrent() && !props.isEditing()}>
            <Tag size="xs" tone="accent">this</Tag>
        </Show>
        {/* Inline opacity — name + slider share one row
            (instance-panel-floating-panes.md §3.3). Stays
            visible during rename; its own stopPropagation
            keeps drags from focusing/renaming. */}
        <Show when={!!props.entry.windowId}>
            {props.opacityControl()}
        </Show>
        <IconButton
            icon="trash-can"
            density="compact"
            class="instance-panel-row-action"
            label={props.onlyOpenWindow() ? LAST_WINDOW_TITLE : "Remove from workspace"}
            disabled={props.onlyOpenWindow()}
            onClick={(e) => {
                e.stopPropagation();
                props.remove();
            }}
            onDblClick={(e) => e.stopPropagation()}
            onKeyDown={(e) => e.stopPropagation()}
        />
    </div>
);
