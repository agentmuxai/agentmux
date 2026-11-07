// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The file tree's inline name editor: for a rename, and for the name of a new file or folder. */

import { createEffect, createSignal, type JSX } from "solid-js";

interface InlineInputProps {
    initialValue: string;
    placeholder?: string;
    onConfirm: (value: string) => void;
    onCancel: () => void;
}

export function InlineInput(props: InlineInputProps): JSX.Element {
    const [value, setValue] = createSignal(props.initialValue);
    let inputRef: HTMLInputElement | undefined;

    createEffect(() => {
        // Auto-focus and select-all on mount.
        if (inputRef) {
            inputRef.focus();
            inputRef.select();
        }
    });

    // Guard against Escape → onCancel unmounts the input → blur fires confirm.
    let committed = false;
    const confirm = () => {
        if (committed) return;
        committed = true;
        const v = value().trim();
        if (v) props.onConfirm(v);
        else props.onCancel();
    };
    const cancel = () => {
        if (committed) return;
        committed = true;
        props.onCancel();
    };

    return (
        <input
            ref={(el) => { inputRef = el; }}
            class="file-tree-inline-input"
            data-raw-text-field="the file tree's inline name editor: it sits in the row of the entry being named and ends on Enter, Escape or blur, not a form field"
            aria-label={props.placeholder ?? "New name"}
            type="text"
            value={value()}
            placeholder={props.placeholder}
            onInput={(e) => setValue(e.currentTarget.value)}
            onKeyDown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") confirm();
                if (e.key === "Escape") cancel();
            }}
            onBlur={confirm}
            onClick={(e) => e.stopPropagation()}
        />
    );
}
