// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor tab strip — Phase 1B of SPEC_EDITOR_TABS_2026-05-26.md.
// Renders above CodeMirror; one tab per open file. Click to activate,
// × (hover-shown) to close, middle-click to close. No overflow chip yet
// (tabs compress to min-width when crowded); chip lands in Phase 2.

import { createSignal, For, onMount, Show, type JSX } from "solid-js";
import type { EditorViewModel } from "./editor-model";
import type { EditorTab } from "@/app/store/editor-pane-state-store";

interface Props {
    model: EditorViewModel;
    /** When set to a tab id, that tab shows an inline save-as path input. */
    saveAsTabId?: string | null;
    onSaveAsConfirm?: (path: string) => void;
    onSaveAsCancel?: () => void;
}

export function EditorTabStrip(props: Props): JSX.Element {
    const tabs = props.model.tabsAtom;
    const activeId = props.model.activeIdAtom;

    return (
        <div
            class="editor-tab-strip"
            // Double-click inside the strip should not maximize the pane —
            // matches the icon-toggle pattern from blockframe.tsx.
            onDblClick={(e) => e.stopPropagation()}
        >
            <For each={tabs()}>
                {(tab) => (
                    <Tab
                        tab={tab}
                        active={activeId() === tab.id}
                        model={props.model}
                        saveAsTabId={props.saveAsTabId}
                        onSaveAsConfirm={props.onSaveAsConfirm}
                        onSaveAsCancel={props.onSaveAsCancel}
                    />
                )}
            </For>
        </div>
    );
}

interface TabProps {
    tab: EditorTab;
    active: boolean;
    model: EditorViewModel;
    saveAsTabId?: string | null;
    onSaveAsConfirm?: (path: string) => void;
    onSaveAsCancel?: () => void;
}

function Tab(props: TabProps): JSX.Element {
    const basename = () => {
        if (props.tab.displayName) return props.tab.displayName;
        const fp = props.tab.filePath;
        const i = Math.max(fp.lastIndexOf("/"), fp.lastIndexOf("\\"));
        return i >= 0 ? fp.slice(i + 1) : fp;
    };

    const isSaveAsMode = () => props.saveAsTabId != null && props.saveAsTabId === props.tab.id;

    const onMouseDown = (e: MouseEvent) => {
        // Middle-click → close (matches VS Code / Chrome convention).
        if (e.button === 1) {
            e.preventDefault();
            props.model.closeTab(props.tab.id);
        }
    };

    const onClick = (e: MouseEvent) => {
        // Ignore middle-click here — onMouseDown already handled it.
        if (e.button !== 0) return;
        if (!props.active) props.model.switchTab(props.tab.id);
    };

    const onDblClick = (e: MouseEvent) => {
        e.stopPropagation();
        // Double-clicking a preview tab pins it (matches VS Code). For an
        // already-pinned tab, dblclick is a no-op at the strip layer
        // (.editor-tab-strip already stops the dblclick from reaching the
        // pane header, so the pane won't maximize either).
        if (props.tab.isPreview) {
            props.model.pinActiveTab();
        }
    };

    const onCloseClick = (e: MouseEvent) => {
        e.stopPropagation();
        props.model.closeTab(props.tab.id);
    };

    return (
        <div
            class="editor-tab"
            classList={{
                "editor-tab--active": props.active,
                "editor-tab--dirty": props.tab.dirty,
                "editor-tab--preview": props.tab.isPreview,
                "editor-tab--saveas": isSaveAsMode(),
            }}
            title={props.tab.isPreview ? `${props.tab.filePath} (preview — double-click to pin)` : props.tab.filePath}
            onMouseDown={onMouseDown}
            onClick={onClick}
            onDblClick={onDblClick}
        >
            <Show when={isSaveAsMode()} fallback={<span class="editor-tab-label">{basename()}</span>}>
                <SaveAsInput
                    onConfirm={props.onSaveAsConfirm ?? (() => undefined)}
                    onCancel={props.onSaveAsCancel ?? (() => undefined)}
                />
            </Show>
            <button
                class="editor-tab-close"
                onClick={onCloseClick}
                title={props.tab.dirty ? "Close (unsaved changes)" : "Close"}
                aria-label="Close tab"
            >
                {/* The × always renders for dirty tabs (since closing prompts
                    the user in a follow-up commit); hover-shown otherwise. */}
                ×
            </button>
        </div>
    );
}

function SaveAsInput(props: { onConfirm: (path: string) => void; onCancel: () => void }): JSX.Element {
    let inputRef: HTMLInputElement | undefined;
    const [value, setValue] = createSignal("~/");
    let committed = false;

    onMount(() => {
        inputRef?.focus();
        // Place cursor at end
        const len = inputRef?.value.length ?? 0;
        inputRef?.setSelectionRange(len, len);
    });

    const confirm = () => {
        if (committed) return;
        committed = true;
        props.onConfirm(value().trim());
    };
    const cancel = () => {
        if (committed) return;
        committed = true;
        props.onCancel();
    };

    return (
        <input
            ref={inputRef}
            class="editor-tab-saveas-input"
            type="text"
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); confirm(); }
                if (e.key === "Escape") { e.preventDefault(); cancel(); }
                e.stopPropagation();
            }}
            onClick={(e) => e.stopPropagation()}
            onBlur={cancel}
            placeholder="~/path/to/file.md"
            title="Type a path and press Enter to save (Esc to cancel)"
        />
    );
}
