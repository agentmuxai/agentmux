// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The key plumbing the decision and question panels share: pane scoping, the
 * inPanel/editable context, focus into the panel, and cleanup when the request
 * ends.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";
import { usePanelKeys } from "./use-panel-keys";

afterEach(() => cleanup());

const key = (el: Element, k = "a") => el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));

function setup(active = true) {
    const onKey = vi.fn();
    const onFocusIn = vi.fn();
    const [isActive, setActive] = createSignal(active);
    let root: HTMLElement | undefined;
    const Panel = () => {
        usePanelKeys(() => root, isActive, onKey, onFocusIn);
        return (
            <div ref={(el) => (root = el)} data-testid="panel">
                <button data-testid="option">A</button>
            </div>
        );
    };
    const { getByTestId } = render(() => (
        <>
            <div data-blockid="A">
                <textarea data-testid="composer" />
                <Panel />
            </div>
            <div data-blockid="B">
                <textarea data-testid="other-pane" />
            </div>
        </>
    ));
    return { onKey, onFocusIn, setActive, getByTestId };
}

describe("usePanelKeys", () => {
    it("reports where a key came from: inside the panel, or an editable elsewhere in the pane", () => {
        const { onKey, getByTestId } = setup();
        key(getByTestId("option"));
        expect(onKey.mock.calls[0][1]).toEqual({ inPanel: true, editable: false });
        key(getByTestId("composer"));
        expect(onKey.mock.calls[1][1]).toEqual({ inPanel: false, editable: true });
    });

    it("ignores keys from another pane", () => {
        const { onKey, getByTestId } = setup();
        key(getByTestId("other-pane"));
        expect(onKey).not.toHaveBeenCalled();
    });

    it("passes on focus that lands inside the panel, not elsewhere", () => {
        const { onFocusIn, getByTestId } = setup();
        getByTestId("composer").dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
        expect(onFocusIn).not.toHaveBeenCalled();
        getByTestId("option").dispatchEvent(new FocusEvent("focusin", { bubbles: true }));
        expect(onFocusIn).toHaveBeenCalledTimes(1);
    });

    it("listens only while active, and stops when the request ends", () => {
        const { onKey, setActive, getByTestId } = setup(false);
        key(getByTestId("option"));
        expect(onKey).not.toHaveBeenCalled();
        setActive(true);
        key(getByTestId("option"));
        expect(onKey).toHaveBeenCalledTimes(1);
        setActive(false);
        key(getByTestId("option"));
        expect(onKey).toHaveBeenCalledTimes(1);
    });
});
