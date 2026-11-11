// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A pane's modal takes the caret only while its pane is the selected one. A question that comes up in a
 * pane the person is not in leaves their caret where it is, and takes it when they go to that pane.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
const [selected, setSelected] = createSignal<string | null>(null);
const ensureSelectionFocused = vi.fn();
vi.mock("@/app/store/focusManager", () => ({
    focusManager: { blockFocusAtom: () => selected(), ensureSelectionFocused: (...a: unknown[]) => ensureSelectionFocused(...a) },
}));

import { ConfirmModal } from "./confirm-modal";
import { paneToWaitFor } from "./modal-focus-trap";
import { ModalLayer } from "./ModalLayer";

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));
const [asking, setAsking] = createSignal(false);
const onCancel = vi.fn();

/** A terminal-like pane the person types in, and a pane a question comes up in. */
function Window() {
    return (
        <div>
            <div data-blockid="term">
                <ModalLayer scope="pane" fill>
                    <textarea data-testid="terminal" />
                </ModalLayer>
            </div>
            <div data-blockid="remotes">
                <ModalLayer scope="pane" fill>
                    <button data-testid="add-remote">Add remote</button>
                    <Show when={asking()}>
                        <ConfirmModal
                            open={true}
                            scope="pane"
                            title="Install the helper on db1?"
                            confirmLabel="Install"
                            onConfirm={() => {}}
                            onCancel={onCancel}
                        />
                    </Show>
                </ModalLayer>
            </div>
        </div>
    );
}

const cancel = () => screen.getByRole("button", { name: "Cancel" });

beforeEach(() => {
    setAsking(false);
    setSelected(null);
    onCancel.mockReset();
    ensureSelectionFocused.mockReset();
});

afterEach(() => cleanup());

describe("a pane's modal and the caret", () => {
    it("takes the caret, on Cancel, when it opens in the selected pane", async () => {
        const { getByTestId } = render(() => <Window />);
        setSelected("remotes");
        getByTestId("add-remote").focus();
        setAsking(true);
        await nextFrame();
        expect(document.activeElement).toBe(cancel());
    });

    it("leaves the caret in the pane the person is typing in, and takes it once they select its pane", async () => {
        const { getByTestId } = render(() => <Window />);
        setSelected("term");
        getByTestId("terminal").focus();
        setAsking(true);
        await nextFrame();
        expect(cancel()).toBeTruthy();
        expect(document.activeElement).toBe(getByTestId("terminal"));
        // Enter typed for the terminal is not an answer.
        fireEvent.keyDown(document.activeElement!, { key: "Enter" });
        fireEvent.keyDown(document.activeElement!, { key: "Escape" });
        expect(onCancel).not.toHaveBeenCalled();

        setSelected("remotes");
        await nextFrame();
        expect(document.activeElement).toBe(cancel());
        fireEvent.keyDown(document.activeElement!, { key: "Escape" });
        expect(onCancel).toHaveBeenCalledTimes(1);
    });

    it("keeps the caret on the control of the modal the person clicked to get there", async () => {
        const { getByTestId } = render(() => <Window />);
        setSelected("term");
        getByTestId("terminal").focus();
        setAsking(true);
        await nextFrame();
        const install = screen.getByRole("button", { name: "Install" });
        install.focus();
        setSelected("remotes");
        await nextFrame();
        expect(document.activeElement).toBe(install);
    });

    it("gives no caret back when it closes without ever having had it", async () => {
        const { getByTestId } = render(() => <Window />);
        setSelected("term");
        getByTestId("add-remote").focus();
        getByTestId("terminal").focus();
        setAsking(true);
        await nextFrame();
        setAsking(false);
        await nextFrame();
        expect(document.activeElement).toBe(getByTestId("terminal"));
        expect(ensureSelectionFocused).not.toHaveBeenCalled();
    });

    it("gives the caret to the selection when it closes after taking it late", async () => {
        const { getByTestId } = render(() => <Window />);
        setSelected("term");
        getByTestId("terminal").focus();
        setAsking(true);
        await nextFrame();
        setSelected("remotes");
        await nextFrame();
        setAsking(false);
        await nextFrame();
        expect(ensureSelectionFocused).toHaveBeenCalledWith("modal-closed");
    });
});

describe("paneToWaitFor", () => {
    const inPane = (blockId?: string) => {
        const pane = document.createElement("div");
        if (blockId) pane.dataset.blockid = blockId;
        const region = pane.appendChild(document.createElement("div"));
        return region;
    };

    it("is the modal's pane only while another pane is the selected one", () => {
        expect(paneToWaitFor(inPane("a"), () => "b")).toBe("a");
        expect(paneToWaitFor(inPane("a"), () => "a")).toBeNull();
        expect(paneToWaitFor(inPane("a"), () => null)).toBeNull();
    });

    it("is none for a region in no pane, and does not ask what is selected", () => {
        const selectedNow = vi.fn(() => "b");
        expect(paneToWaitFor(inPane(), selectedNow)).toBeNull();
        expect(selectedNow).not.toHaveBeenCalled();
    });
});
