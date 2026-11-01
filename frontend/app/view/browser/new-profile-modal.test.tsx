// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The New profile dialog locks the browser pane it was asked from, not the
 * window: it mounts into the pane's `PaneModalScope` and, with no such
 * provider, says so on its root instead of silently covering the window.
 */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/platform/pane-overlay", () => ({ usePaneOverlay: () => {} }));
vi.mock("./browser-profiles", () => ({
    createBrowserProfile: vi.fn(),
    PROFILE_COLORS: ["#d33", "#3d3"],
    nextProfileColor: () => "#d33",
}));

import { PaneModalScope } from "@/element/modal";
import { NewProfileModal } from "./new-profile-modal";

afterEach(() => cleanup());

describe("NewProfileModal", () => {
    it("mounts inside the pane it is rendered in, not the window", () => {
        const pane = document.createElement("div");
        pane.id = "browser-pane";
        document.body.appendChild(pane);
        render(() => (
            <PaneModalScope.Provider value={() => pane}>
                <NewProfileModal close={() => {}} onCreated={() => {}} />
            </PaneModalScope.Provider>
        ));
        const root = pane.querySelector(".modal-root");
        expect(root).not.toBeNull();
        expect(root!.getAttribute("data-scope")).toBe("pane");
        expect(root!.hasAttribute("data-scope-fallback")).toBe(false);
        expect(root!.textContent).toContain("New profile");
        pane.remove();
    });

    it("marks itself when no pane scope hosts it, and logs why", () => {
        const logged = vi.spyOn(console, "error").mockImplementation(() => {});
        const warned = vi.spyOn(console, "warn").mockImplementation(() => {});
        render(() => <NewProfileModal close={() => {}} onCreated={() => {}} />);
        const root = document.querySelector(".modal-root");
        expect(root!.getAttribute("data-scope")).toBe("window");
        expect(root!.getAttribute("data-scope-fallback")).toBe("pane");
        expect(logged.mock.calls.length + warned.mock.calls.length).toBe(1);
        logged.mockRestore();
        warned.mockRestore();
    });
});
