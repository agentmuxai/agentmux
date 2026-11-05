// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The preview's states for an attachment with no id: still processing in the
 * composer, versus gone from the store (transcript).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({
    useHiddenTabMarker: () => () => undefined, usePaneOverlay: () => {} }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { AttachmentsInfoCommand: vi.fn() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/util/app-api", () => ({ getApi: () => ({ getAuthKey: () => "k" }) }));
vi.mock("@/util/endpoints", () => ({ getWebServerEndpoint: () => "http://x" }));

import { ModalLayer } from "@/app/element/ModalLayer";
import { AttachmentLightbox } from "./AttachmentLightbox";

afterEach(() => cleanup());

const open = (item: { pending?: boolean; error?: string }) =>
    render(() => (
        <ModalLayer scope="pane">
            <AttachmentLightbox
                items={[{ key: "k", number: 1, name: "big.pdf", ...item }]}
                index={0}
                onIndex={() => {}}
                onClose={() => {}}
            />
        </ModalLayer>
    ));

describe("AttachmentLightbox", () => {
    it("says a composer attachment is still processing, not gone", () => {
        open({ pending: true });
        expect(screen.getByText("Processing…")).toBeTruthy();
        expect(screen.queryByText(/no longer available/)).toBeNull();
    });

    it("says a transcript attachment without an id is gone", () => {
        open({});
        expect(screen.getByText("This file is no longer available.")).toBeTruthy();
    });

    it("shows a processing failure", () => {
        open({ pending: false, error: "Couldn't read it." });
        expect(screen.getByText("Couldn't read it.")).toBeTruthy();
    });
});

describe("AttachmentLightbox as a pane modal", () => {
    const mount = (handlers: { onClose?: () => void; onIndex?: (i: number) => void } = {}) =>
        render(() => (
            <div data-testid="pane">
                <ModalLayer scope="pane">
                    <AttachmentLightbox
                        items={[
                            { key: "a", number: 1, name: "one.pdf", pending: true },
                            { key: "b", number: 2, name: "two.pdf", pending: true },
                        ]}
                        index={0}
                        onIndex={handlers.onIndex ?? (() => {})}
                        onClose={handlers.onClose ?? (() => {})}
                    />
                </ModalLayer>
            </div>
        ));

    it("covers the pane it is in, not the window", () => {
        const { container } = mount();
        const dialog = screen.getByRole("dialog", { name: "File 1: one.pdf" });
        expect(container.querySelector(".modal-layer-mount")?.contains(dialog)).toBe(true);
        expect(document.body.querySelectorAll(".agent-attachment-lightbox__dialog")).toHaveLength(1);
    });

    it("Escape closes it; the arrow keys step through the attachments", () => {
        const onClose = vi.fn();
        const onIndex = vi.fn();
        mount({ onClose, onIndex });
        const inner = document.querySelector<HTMLElement>(".agent-attachment-lightbox__dialog")!;
        fireEvent.keyDown(inner, { key: "ArrowRight" });
        expect(onIndex).toHaveBeenLastCalledWith(1);
        fireEvent.keyDown(inner, { key: "ArrowLeft" });
        expect(onIndex).toHaveBeenLastCalledWith(1);
        fireEvent.keyDown(inner, { key: "Escape" });
        expect(onClose).toHaveBeenCalledTimes(1);
    });
});
