// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * PaneTabRenameInput — the inline "rename this tab" text input, shared by
 * every `PaneTabStrip` consumer that wires double-click-to-rename (agent
 * fork tabs, terminal tabs). Swapped in via `PaneTabStrip`'s existing
 * `renderLabel` prop, same mechanism the editor's Save-As path input
 * already uses (`frontend/app/view/editor/editor-tab-strip.tsx`).
 *
 * Commit-on-blur (unlike Save-As's cancel-on-blur — Save-As is "type a new
 * path," rename is "edit existing text," so blur here matches the
 * click-away-commits convention of `frontend/app/block/titlebar.tsx`'s pane
 * title editor). Auto-selects all text on mount so typing immediately
 * replaces it, matching `frontend/app/tab/tab.tsx`'s window-tab rename.
 *
 * Spec: docs/specs/SPEC_PANE_TAB_STRIP_COMPACT_SIZING_AND_RENAME_2026_07_22.md §3.3.
 */

import { createSignal, onMount, type JSX } from "solid-js";

export interface PaneTabRenameInputProps {
    initialValue: string;
    /** Called on Enter or blur, only when the trimmed value is non-empty and differs from initialValue. */
    onConfirm: (value: string) => void;
    /** Called on Escape, or on blur/Enter when the value is empty/unchanged. */
    onCancel: () => void;
}

export function PaneTabRenameInput(props: PaneTabRenameInputProps): JSX.Element {
    let inputRef: HTMLInputElement | undefined;
    const [value, setValue] = createSignal(props.initialValue);
    let committed = false;

    onMount(() => {
        inputRef?.focus();
        inputRef?.select();
    });

    const confirm = () => {
        if (committed) return;
        committed = true;
        const trimmed = value().trim();
        if (trimmed && trimmed !== props.initialValue) {
            props.onConfirm(trimmed);
        } else {
            props.onCancel();
        }
    };
    const cancel = () => {
        if (committed) return;
        committed = true;
        props.onCancel();
    };

    return (
        <input
            ref={inputRef}
            class="pane-tab-label pane-tab-rename-input"
            type="text"
            value={value()}
            onInput={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
                if (e.key === "Enter") { e.preventDefault(); confirm(); }
                if (e.key === "Escape") { e.preventDefault(); cancel(); }
                e.stopPropagation();
            }}
            onClick={(e) => e.stopPropagation()}
            onDblClick={(e) => e.stopPropagation()}
            onBlur={confirm}
        />
    );
}
