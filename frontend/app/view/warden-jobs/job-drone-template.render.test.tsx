// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * A Drone template in the Jobs pane's New job list makes a drone, not a job: it is marked Drone, offers Make a drone
 * (never Clone to a job), asks for its name and settings in the list item, and opens the new drone in the Drone pane.
 * Real JobEditor; only the model, RPC and the Drone pane opener are mocked.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({ makeDrone: vi.fn(), openDroneGraph: vi.fn() }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { JobsTemplatesMakeDroneCommand: (_c: unknown, d: unknown) => rpc.makeDrone(d) },
}));
vi.mock("@/app/view/drone/open-drone", () => ({ openDroneGraph: (id: string) => rpc.openDroneGraph(id) }));
vi.mock("@/app/view/agent/use-agent-list", () => ({ useAgentList: () => [() => [], () => null] }));
vi.mock("@/app/view/vault/vault-store", () => ({
    vaultAvailable: () => true,
    vaultEntries: () => [],
    refreshVault: vi.fn(),
}));

import { JobEditor } from "./job-editor";
import { DEFAULTS, mkTemplate } from "./jobs-fixtures";
import type { JobsModel } from "./jobs-model";

const droneTemplate = (() => {
    const base = mkTemplate({ id: "agentmux.drone.one" });
    return {
        ...base,
        manifest: {
            ...base.manifest!,
            id: "agentmux.drone.one",
            name: "Check and file",
            kind: "drone" as const,
            params: { type: "object" as const, properties: {}, required: [] },
        },
    };
})();
const model = { templates: vi.fn(), approveTemplate: vi.fn() };
const onClose = vi.fn();

const open = async (...templates: ReturnType<typeof mkTemplate>[]) => {
    model.templates.mockResolvedValue({ templates, publishers: [], catalogs: [] });
    render(() => (
        <JobEditor
            start={{ mode: "new" }}
            model={model as unknown as JobsModel}
            defaults={DEFAULTS}
            onClose={onClose}
        />
    ));
    fireEvent.click(screen.getByRole("radio", { name: "A template" }));
    await screen.findByText(templates[0].manifest!.name);
};

beforeEach(() => {
    rpc.makeDrone.mockResolvedValue({ drone: { id: "d-new", name: "Check and file" } });
    rpc.openDroneGraph.mockResolvedValue(undefined);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
});

describe("a Drone template in the New job list", () => {
    it("is marked Drone and offers Make a drone, not Clone to a job; a job template keeps Clone to a job", async () => {
        await open(droneTemplate, mkTemplate());
        const items = [...document.querySelectorAll(".warden-jobs-template")];
        expect(items[0].textContent).toContain("Drone");
        expect(items[0].textContent).toContain("Make a drone");
        expect(items[0].textContent).not.toContain("Clone to a job");
        expect(items[1].textContent).toContain("Clone to a job");
        expect(items[1].textContent).not.toContain("Make a drone");
    });

    it("asks for the name and makes the drone, opens it in the Drone pane and closes the editor", async () => {
        await open(droneTemplate);
        fireEvent.click(screen.getByRole("button", { name: "Make a drone" }));
        const name = document.querySelector(".drone-templates-form input") as HTMLInputElement;
        expect(name.value).toBe("Check and file");
        fireEvent.input(name, { target: { value: " My check " } });
        fireEvent.click(screen.getByRole("button", { name: "Make the drone" }));
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(rpc.makeDrone).toHaveBeenCalledWith({
            id: droneTemplate.id,
            version: droneTemplate.manifest.version,
            params: {},
            name: "My check",
        });
        expect(rpc.openDroneGraph).toHaveBeenCalledWith("d-new");
    });

    it("shows a refusal in the item and stays open; Back returns to the list", async () => {
        rpc.makeDrone.mockRejectedValue(new Error("template changed"));
        await open(droneTemplate);
        fireEvent.click(screen.getByRole("button", { name: "Make a drone" }));
        fireEvent.click(screen.getByRole("button", { name: "Make the drone" }));
        await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("template changed"));
        expect(onClose).not.toHaveBeenCalled();
        expect(rpc.openDroneGraph).not.toHaveBeenCalled();
        fireEvent.click(screen.getByRole("button", { name: "Back" }));
        expect(screen.getByRole("button", { name: "Make a drone" })).toBeInTheDocument();
    });
});
