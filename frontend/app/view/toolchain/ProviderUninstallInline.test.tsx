// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({ uninstall: vi.fn(), invalidate: vi.fn() }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { InstallUninstallCommand: (_c: unknown, data: unknown) => h.uninstall(data) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/agent/install-check-cache", () => ({ invalidateInstallCheck: h.invalidate }));

import { ProviderUninstallInline } from "./ProviderUninstallInline";

beforeEach(() => {
    h.uninstall.mockReset();
    h.invalidate.mockReset();
});
afterEach(() => cleanup());

const mount = (onRemoved = vi.fn()) => {
    render(() => <ProviderUninstallInline providerId="goose" label="goose" onRemoved={onRemoved} />);
    return onRemoved;
};

describe("ProviderUninstallInline", () => {
    it("asks first, and a Cancel removes nothing", () => {
        mount();
        fireEvent.click(screen.getByRole("button", { name: "Uninstall" }));
        const question = screen.getByText(/Remove AgentMux's copy of goose\?/);
        // The question is its own line; Remove and Cancel share the row under it.
        expect(question.closest(".toolchain-detail-row")).toBeNull();
        expect(screen.getByRole("button", { name: "Remove" }).parentElement).toBe(screen.getByRole("button", { name: "Cancel" }).parentElement);
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        expect(h.uninstall).not.toHaveBeenCalled();
        expect(screen.getByRole("button", { name: "Uninstall" })).toBeInTheDocument();
    });

    it("removes the provider's copy, forgets the install check and has the row look again", async () => {
        h.uninstall.mockResolvedValue({ removed: true, bytes: 1 });
        const onRemoved = mount();
        fireEvent.click(screen.getByRole("button", { name: "Uninstall" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove" }));
        await waitFor(() => expect(onRemoved).toHaveBeenCalledTimes(1));
        expect(h.uninstall).toHaveBeenCalledWith({ providerId: "goose" });
        expect(h.invalidate).toHaveBeenCalledWith("goose");
    });

    it("says so when there was nothing of the shared copy to remove, instead of acting as if it worked", async () => {
        h.uninstall.mockResolvedValue({ removed: false, bytes: 0 });
        const onRemoved = mount();
        fireEvent.click(screen.getByRole("button", { name: "Uninstall" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove" }));
        expect(await screen.findByText(/Nothing was removed: this copy of goose is in an older AgentMux version's folder/)).toBeInTheDocument();
        expect(onRemoved).not.toHaveBeenCalled();
        expect(h.invalidate).not.toHaveBeenCalled();
    });

    it("shows why srv refused, and leaves the row as it was", async () => {
        h.uninstall.mockRejectedValue(new Error("a running agent uses goose; close it first"));
        const onRemoved = mount();
        fireEvent.click(screen.getByRole("button", { name: "Uninstall" }));
        fireEvent.click(screen.getByRole("button", { name: "Remove" }));
        expect(await screen.findByText("a running agent uses goose; close it first")).toBeInTheDocument();
        expect(onRemoved).not.toHaveBeenCalled();
        expect(h.invalidate).not.toHaveBeenCalled();
    });
});
