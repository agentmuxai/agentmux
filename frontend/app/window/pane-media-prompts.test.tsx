// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The browser pane's camera/microphone questions are pane modals (docs/specs/SPEC_PANE_MODALS_NOT_WINDOW_MODALS_2026_11_02.md
 * row 7): the permission prompt and "Stop camera and microphone access?" cover only the pane they are about, and
 * every way out of the permission prompt answers the host (deny unless the person pressed Allow).
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ModalLayer } from "@/app/element/ModalLayer";
import { __resetPaneRequests } from "@/app/store/pane-requests";
import { PaneRequestModals } from "@/app/view/agent/pane-requests/PaneRequestDock";
import { PaneMediaCaptureIndicator } from "./pane-media-capture-indicator";
import { PaneMediaPermissionPrompt } from "./pane-media-permission-prompt";

vi.mock("@/app/notification/waiting-for-you", () => ({ startWaitingForYou: () => {}, endWaitingForYou: () => {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: vi.fn(async () => ({})) }));
const panes = new Map<string, ReturnType<typeof createSignal<object | undefined>>>();
const pane = (id: string) => {
    if (!panes.has(id)) panes.set(id, createSignal<object | undefined>({ oid: id }));
    return panes.get(id)!;
};
vi.mock("@/app/store/global", () => ({
    MOS: { getMuxObjectAtom: (key: string) => pane(key.replace("block:", ""))[0] },
}));

const listeners = new Map<string, (payload: unknown) => void>();
const respondMediaPermission = vi.fn(async () => {});
const revokeMedia = vi.fn(async () => {});
vi.mock("@/util/app-api", () => ({
    getApi: () => ({
        listen: async (name: string, cb: (payload: unknown) => void) => {
            listeners.set(name, cb);
            return () => listeners.delete(name);
        },
        browserPanes: { respondMediaPermission, revokeMedia },
    }),
}));

function TwoPanes() {
    return (
        <>
            <PaneMediaPermissionPrompt />
            <PaneMediaCaptureIndicator />
            {["a", "b"].map((id) => (
                <div data-testid={`pane-${id}`}>
                    <ModalLayer scope="pane" fill>
                        <PaneRequestModals blockId={id} />
                        <button data-testid={`btn-${id}`}>content {id}</button>
                    </ModalLayer>
                </div>
            ))}
        </>
    );
}

const VIDEO = 1 << 1;
const AUDIO = 1 << 0;
const ask = (requestId: number, blockId: string, requested = VIDEO) =>
    listeners.get("pane-media-permission-request")!({ requestId, blockId, origin: "https://meet.example.com/room/1", requested });
const TITLE = "meet.example.com wants to use your camera";

beforeEach(async () => {
    render(() => <TwoPanes />);
    await vi.waitFor(() => expect(listeners.size).toBe(2));
});

afterEach(() => {
    cleanup();
    __resetPaneRequests();
    panes.clear();
    listeners.clear();
    vi.clearAllMocks();
    vi.useRealTimers();
});

describe("media permission prompt", () => {
    it("is drawn over the pane that asked, not the window or its sibling", () => {
        ask(7, "a");
        const dialog = screen.getByText(TITLE);
        expect(screen.getByTestId("pane-a").contains(dialog)).toBe(true);
        expect(screen.getByTestId("pane-b").querySelector(".modal-root")).toBeNull();
        expect(screen.getByTestId("btn-b").hasAttribute("inert")).toBe(false);
    });

    it("Allow answers yes, once, and takes the question away", () => {
        ask(7, "a", VIDEO | AUDIO);
        expect(screen.queryByText("meet.example.com wants to use your camera and microphone")).not.toBeNull();
        fireEvent.click(screen.getByText("Allow"));
        expect(respondMediaPermission).toHaveBeenCalledExactlyOnceWith(7, true);
        expect(screen.queryByText(/wants to use your/)).toBeNull();
    });

    it("Don't allow and Escape both deny", () => {
        ask(1, "a");
        fireEvent.click(screen.getByText("Don't allow"));
        expect(respondMediaPermission).toHaveBeenLastCalledWith(1, false);
        ask(2, "a");
        fireEvent.keyDown(document.querySelector(".modal-panel")!, { key: "Escape" });
        expect(respondMediaPermission).toHaveBeenLastCalledWith(2, false);
        expect(respondMediaPermission).toHaveBeenCalledTimes(2);
        expect(screen.queryByText(/wants to use your/)).toBeNull();
    });

    it("a second request from the same pane denies the first and shows only the second", () => {
        ask(1, "a");
        ask(2, "a", AUDIO);
        expect(respondMediaPermission).toHaveBeenCalledExactlyOnceWith(1, false);
        expect(screen.queryByText(TITLE)).toBeNull();
        expect(screen.queryByText("meet.example.com wants to use your microphone")).not.toBeNull();
    });

    it("two panes can each be asked at once", () => {
        ask(1, "a");
        ask(2, "b");
        expect(screen.getByTestId("pane-a").textContent).toContain(TITLE);
        expect(screen.getByTestId("pane-b").textContent).toContain(TITLE);
        expect(respondMediaPermission).not.toHaveBeenCalled();
    });

    it("closing the pane with the question open denies", async () => {
        ask(5, "a");
        pane("a")[1](undefined);
        await vi.waitFor(() => expect(respondMediaPermission).toHaveBeenCalledExactlyOnceWith(5, false));
    });

    it("an unanswered question goes away when the host's own minute is up", () => {
        vi.useFakeTimers();
        ask(5, "a");
        vi.advanceTimersByTime(60_000);
        expect(screen.queryByText(TITLE)).toBeNull();
        expect(respondMediaPermission).toHaveBeenCalledExactlyOnceWith(5, false);
    });
});

describe("Stop camera and microphone access", () => {
    const capture = (blockId: string) => listeners.get("pane-media-capture-changed")!({ blockId, hasVideo: true, hasAudio: false });

    it("asks inside the pane that is capturing, and a second click doesn't stack another", () => {
        capture("b");
        fireEvent.click(screen.getByText("Stop"));
        fireEvent.click(screen.getByText("Stop"));
        expect(screen.getByTestId("pane-b").textContent).toContain("Stop camera and microphone access?");
        expect(screen.getByTestId("pane-a").querySelector(".modal-root")).toBeNull();
        expect(document.querySelectorAll(".modal-panel")).toHaveLength(1);
        expect(revokeMedia).not.toHaveBeenCalled();
    });

    it("Stop and reload revokes that pane's grant; Cancel does nothing and can be asked again", () => {
        capture("b");
        fireEvent.click(screen.getByText("Stop"));
        fireEvent.click(screen.getByText("Cancel"));
        expect(revokeMedia).not.toHaveBeenCalled();
        fireEvent.click(screen.getByText("Stop"));
        fireEvent.click(screen.getByText("Stop and reload"));
        expect(revokeMedia).toHaveBeenCalledExactlyOnceWith("b");
        expect(screen.queryByText("Stop camera and microphone access?")).toBeNull();
    });
});
