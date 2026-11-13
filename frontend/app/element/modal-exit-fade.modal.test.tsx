// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/**
 * A modal's backdrop fades out however it closes: its `open` going false, or its owner removing it
 * (most dialogs are `<Modal open={true}>` shown under a `<Show>`).
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal, Show } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.unmock("@/app/element/modal-exit-fade");
vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("@/app/motion/motion", () => ({ shouldReduceMotion: () => false, scaledMs: () => 120 }));

import { Modal } from "./modal";
import { ModalLayer } from "./ModalLayer";

const [shown, setShown] = createSignal(false);
const [open, setOpen] = createSignal(true);

function Owner(props: { kept: boolean }) {
    return (
        <ModalLayer scope="pane" fill>
            <div>pane content</div>
            <Show when={shown()}>
                {props.kept ? (
                    <Modal open={open()} scope="pane" onClose={() => {}}>
                        <p>dialog</p>
                    </Modal>
                ) : (
                    <Modal open={true} scope="pane" onClose={() => {}}>
                        <p>dialog</p>
                    </Modal>
                )}
            </Show>
        </ModalLayer>
    );
}

const leaving = (c: HTMLElement) => c.querySelectorAll(".modal-leaving");

describe("a closing modal's backdrop", () => {
    beforeEach(() => {
        vi.useFakeTimers();
        setShown(false);
        setOpen(true);
    });
    afterEach(() => {
        cleanup();
        vi.useRealTimers();
    });

    it("fades when its owner removes it", () => {
        const { container } = render(() => <Owner kept={false} />);
        setShown(true);
        expect(container.querySelectorAll(".modal-root").length).toBe(1);
        setShown(false);
        expect(container.querySelectorAll(".modal-root").length).toBe(0);
        expect(leaving(container).length).toBe(1);
        expect(leaving(container)[0].querySelector(".modal-backdrop")).not.toBeNull();
        expect(leaving(container)[0].querySelector(".modal-panel")).toBeNull();
        vi.advanceTimersByTime(300);
        expect(leaving(container).length).toBe(0);
    });

    it("fades when its open flag goes false", () => {
        const { container } = render(() => <Owner kept={true} />);
        setShown(true);
        setOpen(false);
        expect(container.querySelectorAll(".modal-root").length).toBe(0);
        expect(leaving(container).length).toBe(1);
        vi.advanceTimersByTime(300);
        expect(leaving(container).length).toBe(0);
    });
});
