// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor tab strip — Phase 1B of SPEC_EDITOR_TABS_2026-05-26.md.
// Renders above CodeMirror; one tab per open file. Click to activate,
// × (hover-shown) to close, middle-click to close. No overflow chip yet
// (tabs compress to min-width when crowded); chip lands in Phase 2.

import { For, type JSX } from "solid-js";
import type { EditorViewModel } from "./editor-model";
import type { EditorTab } from "@/app/store/editor-pane-state-store";

interface Props {
    model: EditorViewModel;
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
                {(tab) => <Tab tab={tab} active={activeId() === tab.id} model={props.model} />}
            </For>
        </div>
    );
}

function Tab(props: { tab: EditorTab; active: boolean; model: EditorViewModel }): JSX.Element {
    const basename = () => {
        const fp = props.tab.filePath;
        const i = Math.max(fp.lastIndexOf("/"), fp.lastIndexOf("\\"));
        return i >= 0 ? fp.slice(i + 1) : fp;
    };

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
            }}
            title={props.tab.filePath}
            onMouseDown={onMouseDown}
            onClick={onClick}
        >
            <span class="editor-tab-label">{basename()}</span>
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
