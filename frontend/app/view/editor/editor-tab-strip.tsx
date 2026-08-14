// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Editor tab strip — Phase 1B of SPEC_EDITOR_TABS_2026-05-26.md.
// Renders above CodeMirror; one tab per open file. Click to activate,
// × (hover-shown) to close, middle-click to close. No overflow chip yet
// (tabs compress to min-width when crowded); chip lands in Phase 2.
//
// Chrome/behavior (click/middle-click/hover-close/tooltip/active-underline)
// is the shared <PaneTabStrip> (frontend/app/element/PaneTabStrip.tsx),
// promoted out of this file so agent-pane forks and terminal-pane shell
// tabs can reuse the identical strip instead of each pane type growing its
// own copy. Only the editor-specific bits (preview italics, pin-on-
// double-click, the inline Save-As path input, the "+" → new scratch
// buffer) stay local. See docs/specs/SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md.

import { createSignal, onMount, type JSX } from "solid-js";
import { PaneTabStrip } from "@/app/element/PaneTabStrip";
import type { EditorViewModel } from "./editor-model";
import type { EditorTab } from "@/app/store/editor-pane-state-store";

interface Props {
    model: EditorViewModel;
    /** When set to a tab id, that tab shows an inline save-as path input. */
    saveAsTabId?: string | null;
    onSaveAsConfirm?: (path: string) => void;
    onSaveAsCancel?: () => void;
}

function basenameOf(tab: EditorTab): string {
    if (tab.displayName) return tab.displayName;
    const fp = tab.filePath;
    const i = Math.max(fp.lastIndexOf("/"), fp.lastIndexOf("\\"));
    return i >= 0 ? fp.slice(i + 1) : fp;
}

export function EditorTabStrip(props: Props): JSX.Element {
    const tabs = props.model.tabsAtom;
    const activeId = props.model.activeIdAtom;

    const isSaveAsMode = (tab: EditorTab) =>
        props.saveAsTabId != null && props.saveAsTabId === tab.id;

    return (
        <PaneTabStrip<EditorTab>
            tabs={tabs()}
            activeId={activeId()}
            // Deliberately NOT model.zoomAtom here — <EditorTabStrip> renders
            // inside .editor-view (editor-view.tsx:701-705), which already
            // has `style={{ zoom: model.zoomAtom() }}` on that ancestor.
            // Unlike the agent pane (where the tab strip is a DOM SIBLING of
            // .agent-view, specifically so it needs its own explicit zoom —
            // SPEC_PANE_TAB_STRIP_CHROME_ZOOM_AND_SCROLL_CLEARANCE_2026_08_12.md
            // §A.3), the editor tab strip is a DESCENDANT of the
            // already-zoomed root: ambient ancestor zoom already scales this
            // entire subtree (including PaneTabStrip.scss's own height calc
            // and inner zoom, both of which default to 1/28px when this prop
            // is omitted) automatically. Passing zoomAtom here compounds it
            // (zoomAtom()²) — caught in review on PR #2566.
            getId={(tab) => tab.id}
            getLabel={basenameOf}
            getTooltip={(tab) =>
                tab.isPreview
                    ? `${tab.filePath} (preview — double-click to pin)`
                    : tab.filePath
            }
            getAttention={(tab) => tab.dirty}
            getTabClass={(tab) => ({
                "editor-tab--preview": tab.isPreview,
                "editor-tab--saveas": isSaveAsMode(tab),
            })}
            onActivate={(id) => props.model.switchTab(id)}
            onClose={(id) => props.model.closeTab(id)}
            onTabDoubleClick={(tab) => {
                // Double-clicking a preview tab pins it (matches VS Code).
                // No-op for an already-pinned tab.
                if (tab.isPreview) props.model.pinActiveTab();
            }}
            renderLabel={(tab) =>
                isSaveAsMode(tab) ? (
                    <SaveAsInput
                        onConfirm={props.onSaveAsConfirm ?? (() => undefined)}
                        onCancel={props.onSaveAsCancel ?? (() => undefined)}
                    />
                ) : (
                    <span class="pane-tab-label">{basenameOf(tab)}</span>
                )
            }
            onAdd={() => void props.model.openScratch()}
            addTitle="New scratch buffer"
        />
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
