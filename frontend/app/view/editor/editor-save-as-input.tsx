// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The inline Save As box: it takes a scratch tab's label in the strip
 * (DocTabStrip's `renderLabel`) when Ctrl+S or Ctrl+Shift+S is pressed on it.
 * Enter saves to the typed path, Escape or leaving the box cancels.
 */

import { createSignal, onMount, type JSX } from "solid-js";

export function SaveAsInput(props: { onConfirm: (path: string) => void; onCancel: () => void }): JSX.Element {
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
