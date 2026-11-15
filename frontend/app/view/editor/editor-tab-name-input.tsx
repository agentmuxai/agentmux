// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The name box of a document tab, and the line under it
 * (docs/specs/SPEC_EDITOR_TAB_RENAME_2026_11_14.md §3, §11.9).
 *
 * The box takes the tab's label in the strip (DocTabStrip's `renderLabel`)
 * after a double-click, "Rename…" in the tab's menu, or F2. Enter commits,
 * Escape cancels, and leaving the box cancels: nothing is renamed or saved
 * without Enter. While a commit is on its way the box waits for the answer;
 * a refusal keeps it open, with the reason on the line under the strip.
 */

import { createSignal, onMount, Show, type Accessor, type JSX } from "solid-js";
import { nameStemEnd, nextNameSelection } from "./editor-rename-names";
import type { TabRename } from "./editor-tab-rename";

/** How far the box starts from the left edge of its column, in the column's own pixels (the Editor can be zoomed). */
function leftInColumn(input: HTMLElement): number {
    const column = input.closest<HTMLElement>(".editor-main-column");
    if (!column) return 0;
    const box = column.getBoundingClientRect();
    const scale = column.offsetWidth > 0 ? box.width / column.offsetWidth : 1;
    return Math.max(0, (input.getBoundingClientRect().left - box.left) / (scale || 1));
}

export function TabNameInput(props: {
    rename: TabRename;
    /** The id of the line under the strip, which describes this box. */
    lineId: string;
    /** Where the box starts, for the line to start under it. */
    onPlaced?: (left: number) => void;
}): JSX.Element {
    let input: HTMLInputElement | undefined;
    const initial = props.rename.initialName();
    const [value, setValue] = createSignal(initial);

    onMount(() => {
        if (!input) return;
        input.focus();
        // Typing replaces the name and keeps the extension.
        input.setSelectionRange(0, nameStemEnd(initial));
        input.scrollIntoView?.({ block: "nearest", inline: "nearest" });
        props.onPlaced?.(leftInColumn(input));
    });

    const onKeyDown = (e: KeyboardEvent) => {
        // The box's keys are its own: undo, select-all and the rest never reach the editor or the app.
        e.stopPropagation();
        // Enter that confirms an input-method composition is not a commit. Safari ends the
        // composition before that key arrives, and reports it as key code 229.
        if (e.isComposing || e.keyCode === 229) return;
        if (e.key === "Enter") {
            e.preventDefault();
            void props.rename.commit(value());
        } else if (e.key === "Escape") {
            e.preventDefault();
            props.rename.cancel(true);
        } else if (e.key === "F2" && input) {
            e.preventDefault();
            input.setSelectionRange(...nextNameSelection(value(), input.selectionStart ?? 0, input.selectionEnd ?? 0));
        }
    };

    return (
        <input
            ref={input}
            class="editor-tab-saveas-input editor-tab-name-input"
            data-raw-text-field="the file name box in the editor tab: it replaces the tab's name in place and ends on Enter, Escape or blur, not a form field"
            aria-label="File name"
            aria-describedby={props.lineId}
            aria-invalid={props.rename.line()?.kind === "error"}
            type="text"
            spellcheck={false}
            autocomplete="off"
            readOnly={props.rename.busy()}
            value={value()}
            onInput={(e) => {
                setValue(e.currentTarget.value);
                props.rename.setTyped(e.currentTarget.value);
            }}
            onKeyDown={onKeyDown}
            onClick={(e) => e.stopPropagation()}
            onDblClick={(e) => e.stopPropagation()}
            // Leaving cancels. The whole window losing focus (another app, an input-source
            // switch) is not leaving: the box is still focused when the window is back.
            onBlur={() => {
                if (document.hasFocus()) props.rename.cancel(false);
            }}
        />
    );
}

/** The line under the strip while a name box is open: where an Untitled document will go, or why the name was refused. */
export function TabRenameLine(props: { rename: TabRename; id: string; left: Accessor<number> }): JSX.Element {
    const line = () => (props.rename.tabId() ? props.rename.line() : null);
    return (
        <Show when={line()}>
            {(shown) => (
                <div
                    id={props.id}
                    class="editor-tab-rename-line"
                    classList={{ "editor-tab-rename-line--error": shown().kind === "error" }}
                    // An error is said aloud as it appears; the hint is read with the box it describes.
                    role={shown().kind === "error" ? "alert" : undefined}
                    style={{ "--rename-line-left": `${Math.round(props.left())}px` }}
                >
                    <Show when={shown().kind === "error"}>
                        <i class="fa fa-triangle-exclamation" aria-hidden="true" />
                    </Show>
                    <span>{shown().text}</span>
                </div>
            )}
        </Show>
    );
}
