// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ModalLayer content swaps — SPEC_UNIVERSAL_INSTALL_DIALOG_2026_09_23.md
 * §4.3 rules 4 and 5. `replace` keeps the same `.modal-panel` (the scroll
 * container) mounted, so the new content must start at the top and move
 * focus to its own primary control.
 */

import { cleanup, render, waitFor } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("./modal-dispatch", () => ({
    requestLabel: (req: { kind: string }) => req.kind,
    // Each fake panel's primary button is its initial-focus target.
    renderRequest: (req: { kind: string }) => ({
        label: req.kind,
        panel: (
            <div class="modal-panel-body">
                <button type="button">secondary</button>
                <button type="button" data-modal-initial-focus>
                    {`primary ${req.kind}`}
                </button>
            </div>
        ),
    }),
}));

import { ModalLayer } from "./ModalLayer";
import { useModalLayer, type ModalLayerApi, type ModalLayerRequest } from "./modal-layer";

afterEach(() => cleanup());

const nextFrame = () => new Promise<void>((r) => requestAnimationFrame(() => r()));

function renderLayer(): { api: () => ModalLayerApi; container: HTMLElement } {
    let api: ModalLayerApi | null = null;
    const Grab = () => {
        api = useModalLayer();
        return null;
    };
    const { container } = render(() => (
        <ModalLayer scope="tab">
            <Grab />
        </ModalLayer>
    ));
    return { api: () => api!, container };
}

// The fake dispatcher only reads `kind`.
const req = (kind: string) => ({ kind }) as unknown as ModalLayerRequest;

describe("ModalLayer", () => {
    it("tags the panel with the request kind", async () => {
        const { api, container } = renderLayer();
        api().open(req("install-agent"));
        await waitFor(() => expect(container.querySelector(".modal-panel--install-agent")).not.toBeNull());
        api().replace(req("agent-prereqs"));
        await waitFor(() => expect(container.querySelector(".modal-panel--agent-prereqs")).not.toBeNull());
        expect(container.querySelector(".modal-panel--install-agent")).toBeNull();
    });

    it("focuses the panel's initial-focus control on open", async () => {
        const { api } = renderLayer();
        api().open(req("agent-prereqs"));
        await waitFor(() => expect(document.activeElement?.textContent).toBe("primary agent-prereqs"));
    });

    it("resets the panel's scroll and moves focus when content is replaced", async () => {
        const { api, container } = renderLayer();
        api().open(req("agent-prereqs"));
        await waitFor(() => expect(container.querySelector(".modal-panel")).not.toBeNull());
        await nextFrame();
        const panel = container.querySelector<HTMLElement>(".modal-panel")!;
        panel.scrollTop = 250;
        expect(panel.scrollTop).toBe(250);

        api().replace(req("install-agent"));
        await waitFor(() => expect(document.activeElement?.textContent).toBe("primary install-agent"));
        // Same panel element survived the swap; its scroll was reset.
        expect(container.querySelector(".modal-panel")).toBe(panel);
        expect(panel.scrollTop).toBe(0);
    });
});
