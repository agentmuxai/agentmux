// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    start: vi.fn(),
    release: vi.fn(),
    watch: vi.fn(),
    state: null as unknown as () => string,
    setState: null as unknown as (s: string) => void,
}));

vi.mock("@/app/view/agent/install-sessions", () => ({
    agentInstallSession: () => ({ state: () => h.state(), start: h.start }),
    watchAgentInstall: (id: string) => {
        h.watch(id);
        return h.release;
    },
}));
vi.mock("@/element/install/InstallProgress", () => ({
    InstallProgress: () => <div data-testid="install-progress" />,
}));

import { ProviderUpdateInline } from "./ProviderUpdateInline";

beforeEach(() => {
    h.start.mockReset();
    h.release.mockReset();
    h.watch.mockReset();
    const [state, setState] = createSignal("idle");
    h.state = state;
    h.setState = setState;
});
afterEach(() => cleanup());

describe("ProviderUpdateInline", () => {
    it("offers the update to the pin and starts the install on press", () => {
        render(() => <ProviderUpdateInline providerId="codex" pinned="0.154.0" />);
        expect(screen.queryByTestId("install-progress")).toBeNull();
        fireEvent.click(screen.getByRole("button", { name: "Update to 0.154.0" }));
        expect(h.start).toHaveBeenCalledTimes(1);
    });

    it("shows the run while it is going, then offers to update again once it is done", () => {
        render(() => <ProviderUpdateInline providerId="codex" pinned="0.154.0" />);
        h.setState("running");
        expect(screen.getByText("Updating…")).toBeInTheDocument();
        expect(screen.getByTestId("install-progress")).toBeInTheDocument();
        expect(screen.queryByRole("button")).toBeNull();
        h.setState("done");
        expect(screen.getByText("Updated")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Update again" }));
        expect(h.start).toHaveBeenCalledTimes(1);
    });

    it("in install mode, says Install and Installed where the update says Update and Updated", () => {
        render(() => <ProviderUpdateInline providerId="goose" pinned="1.54.0" mode="install" />);
        fireEvent.click(screen.getByRole("button", { name: "Install 1.54.0" }));
        expect(h.start).toHaveBeenCalledTimes(1);
        h.setState("running");
        expect(screen.getByText("Installing…")).toBeInTheDocument();
        h.setState("done");
        expect(screen.getByText("Installed")).toBeInTheDocument();
        expect(screen.getByRole("button", { name: "Install again" })).toBeInTheDocument();
    });

    it("offers Retry after a failure", () => {
        render(() => <ProviderUpdateInline providerId="codex" pinned="0.154.0" />);
        h.setState("failed");
        expect(screen.getByText("Failed")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Retry" }));
        expect(h.start).toHaveBeenCalledTimes(1);
    });

    it("watches the install while mounted, so its end raises no new-agent toast, and lets go after", () => {
        const { unmount } = render(() => <ProviderUpdateInline providerId="codex" pinned="0.154.0" />);
        expect(h.watch).toHaveBeenCalledWith("codex");
        expect(h.release).not.toHaveBeenCalled();
        unmount();
        expect(h.release).toHaveBeenCalledTimes(1);
    });
});
