// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Two agent panes side by side: Ctrl+F must toggle search in the pane the key
// was typed in — not in whichever pane holds a leftover text selection
// (REPORT_AGENT_PANE_SIDE_BY_SIDE_SCROLL_AND_FOCUS_QUIRKS_2026_09_23.md §6).

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useAgentKeyboard } from "./useAgentKeyboard";

function Pane(props: { blockId: string; onToggleSearch: () => void; focused?: boolean }) {
    useAgentKeyboard({ blockId: props.blockId, onToggleSearch: props.onToggleSearch });
    return (
        <div data-blockid={props.blockId} class={props.focused ? "block-focused" : ""}>
            <p data-testid={`text-${props.blockId}`}>transcript text</p>
            <textarea data-testid={`composer-${props.blockId}`} />
        </div>
    );
}

const ctrlF = (target: EventTarget) =>
    target.dispatchEvent(new KeyboardEvent("keydown", { key: "f", ctrlKey: true, bubbles: true }));

afterEach(() => {
    cleanup();
    document.getSelection()?.removeAllRanges();
});

describe("useAgentKeyboard — Ctrl+F with two panes", () => {
    it("text left selected in pane A doesn't pull Ctrl+F there when pane B is the selected pane", () => {
        const toggleA = vi.fn();
        const toggleB = vi.fn();
        const { getByTestId } = render(() => (
            <>
                <Pane blockId="A" onToggleSearch={toggleA} />
                <Pane blockId="B" onToggleSearch={toggleB} focused />
            </>
        ));
        // Drag-selected transcript text in A, then selected pane B by its
        // header: nothing has DOM focus, the selection is still in A.
        const range = document.createRange();
        range.selectNodeContents(getByTestId("text-A"));
        document.getSelection()!.addRange(range);
        (document.activeElement as HTMLElement | null)?.blur();

        ctrlF(document.body);

        expect(toggleB).toHaveBeenCalledTimes(1);
        expect(toggleA).not.toHaveBeenCalled();
    });

    it("toggles only the pane the key was typed in, even with text selected in the other", () => {
        const toggleA = vi.fn();
        const toggleB = vi.fn();
        const { getByTestId } = render(() => (
            <>
                <Pane blockId="A" onToggleSearch={toggleA} focused />
                <Pane blockId="B" onToggleSearch={toggleB} />
            </>
        ));
        const range = document.createRange();
        range.selectNodeContents(getByTestId("text-A"));
        document.getSelection()!.addRange(range);

        const composerB = getByTestId("composer-B");
        composerB.focus();
        ctrlF(composerB);

        expect(toggleB).toHaveBeenCalledTimes(1);
        expect(toggleA).not.toHaveBeenCalled();
    });

    it("with nothing focused, the selected pane gets it", () => {
        const toggleA = vi.fn();
        const toggleB = vi.fn();
        render(() => (
            <>
                <Pane blockId="A" onToggleSearch={toggleA} />
                <Pane blockId="B" onToggleSearch={toggleB} focused />
            </>
        ));
        ctrlF(document.body);
        expect(toggleB).toHaveBeenCalledTimes(1);
        expect(toggleA).not.toHaveBeenCalled();
    });
});
