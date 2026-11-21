// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * "New from a template" in the Drone pane: only the Drone templates are offered; one that is not approved is reviewed
 * and approved first, exactly as a job template is; a drone made from one opens on the canvas and nothing runs. Real
 * picker and DroneView; only RPC and the pane opener are mocked.
 */

import { cleanup, fireEvent, render, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { mkTemplate } from "@/app/view/warden-jobs/jobs-fixtures";
import type { JobTemplateInfo } from "@/types/rpc/JobTemplateInfo";

const rpc = vi.hoisted(() => ({
    list: vi.fn(),
    approve: vi.fn(),
    makeDrone: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
        JobsTemplatesListCommand: (_c: unknown) => rpc.list(),
        JobsTemplatesApproveCommand: (_c: unknown, d: unknown) => rpc.approve(d),
        JobsTemplatesMakeDroneCommand: (_c: unknown, d: unknown) => rpc.makeDrone(d),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/warden/open-warden", () => ({ openWarden: () => Promise.resolve() }));

import { DroneTemplatePicker, droneTemplatesIn } from "./drone-template-picker";
import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";

vi.stubGlobal(
    "ResizeObserver",
    class {
        observe() {}
        unobserve() {}
        disconnect() {}
    }
);

function droneTemplate(id: string, name: string, over: Partial<JobTemplateInfo> = {}): JobTemplateInfo {
    const base = mkTemplate();
    return { ...base, id, hash: `hash-${id}`, manifest: { ...base.manifest!, id, name, kind: "drone", params: { type: "object", properties: {}, required: [] } }, ...over };
}

const approved = droneTemplate("agentmux.drone.one", "Check and file");
const pending = droneTemplate("agentmux.drone.two", "Triage failures", { state: "needs_approval" });
const aJob = mkTemplate({ id: "agentmux.job.one" });

let model: DroneViewModel | null = null;
const names = (root: HTMLElement) => [...root.querySelectorAll(".drone-templates-item-head strong")].map((e) => e.textContent);
const button = (root: HTMLElement, label: string) => [...root.querySelectorAll("button")].find((b) => b.textContent?.trim() === label) as HTMLButtonElement;

beforeEach(() => {
    rpc.list.mockResolvedValue({ templates: [aJob, approved, pending] });
    rpc.approve.mockResolvedValue({ templates: [aJob, approved, { ...pending, state: "approved" }] });
    rpc.makeDrone.mockResolvedValue({ drone: { id: "d-new", name: "Check and file" } });
    model = new DroneViewModel({ blockId: "drone-test-block", meta: () => ({}) } as any);
});

afterEach(() => {
    cleanup();
    model?.dispose();
    model = null;
    vi.clearAllMocks();
});

describe("droneTemplatesIn", () => {
    it("keeps the Drone templates, in order, and leaves the job templates out", () => {
        expect(droneTemplatesIn([aJob, approved, pending]).map((i) => i.id)).toEqual([approved.id, pending.id]);
    });
});

describe("New from a template", () => {
    it("lists the Drone templates with their state: Use when approved, Review when not", async () => {
        const { container } = render(() => <DroneTemplatePicker model={model!} onClose={() => {}} />);
        await waitFor(() => expect(names(container)).toEqual(["Check and file", "Triage failures"]));
        const items = [...container.querySelectorAll(".drone-templates-item")];
        expect(items[0].textContent).toContain("Approved");
        expect(items[0].textContent).toContain("Use");
        expect(items[1].textContent).toContain("Needs approval");
        expect(items[1].textContent).toContain("Review and approve");
        expect(items[1].textContent).not.toContain("Use");
    });

    it("says so when no Drone template is installed", async () => {
        rpc.list.mockResolvedValue({ templates: [aJob] });
        const { container } = render(() => <DroneTemplatePicker model={model!} onClose={() => {}} />);
        await waitFor(() => expect(container.textContent).toContain("No Drone templates are installed."));
    });

    it("approves what was reviewed, for the hash it showed, and then offers it", async () => {
        const { container } = render(() => <DroneTemplatePicker model={model!} onClose={() => {}} />);
        await waitFor(() => expect(button(container, "Review and approve")).toBeTruthy());
        fireEvent.click(button(container, "Review and approve"));
        fireEvent.click(button(container, "Approve"));
        await waitFor(() => expect(rpc.approve).toHaveBeenCalledWith({ id: pending.id, hash: pending.hash, signer: "" }));
        await waitFor(() => expect(container.querySelectorAll(".drone-templates-item")).toHaveLength(2));
        expect([...container.querySelectorAll(".drone-templates-item")].every((i) => i.textContent?.includes("Use"))).toBe(true);
    });

    it("makes a drone from an approved template and opens it, without running anything", async () => {
        const open = vi.spyOn(model!, "openDrone").mockResolvedValue();
        const refresh = vi.spyOn(model!, "refreshList").mockResolvedValue();
        const onClose = vi.fn();
        const { container } = render(() => <DroneTemplatePicker model={model!} onClose={onClose} />);
        await waitFor(() => expect(button(container, "Use")).toBeTruthy());
        fireEvent.click(button(container, "Use"));
        const nameInput = container.querySelector(".drone-templates-form input") as HTMLInputElement;
        expect(nameInput.value).toBe("Check and file");
        fireEvent.input(nameInput, { target: { value: "  My check  " } });
        fireEvent.click(button(container, "Make the drone"));
        await waitFor(() => expect(onClose).toHaveBeenCalled());
        expect(rpc.makeDrone).toHaveBeenCalledWith({ id: approved.id, version: approved.manifest!.version, params: {}, name: "My check" });
        expect(open).toHaveBeenCalledWith("d-new");
        expect(refresh).toHaveBeenCalled();
    });

    it("shows a refusal and stays open", async () => {
        rpc.makeDrone.mockRejectedValue(new Error("template changed"));
        const onClose = vi.fn();
        const { container } = render(() => <DroneTemplatePicker model={model!} onClose={onClose} />);
        await waitFor(() => expect(button(container, "Use")).toBeTruthy());
        fireEvent.click(button(container, "Use"));
        fireEvent.click(button(container, "Make the drone"));
        await waitFor(() => expect(container.querySelector("[role=alert]")?.textContent).toContain("template changed"));
        expect(onClose).not.toHaveBeenCalled();
    });

    it("opens from the Drone bar and closes again, as a panel above the canvas", async () => {
        const { container } = render(() => <DroneView model={model!} />);
        expect(container.querySelector(".drone-templates")).toBeNull();
        fireEvent.click(button(container, "From a template…"));
        await waitFor(() => expect(container.querySelector(".drone-stage > .drone-templates")).toBeTruthy());
        fireEvent.click(button(container, "Close"));
        expect(container.querySelector(".drone-templates")).toBeNull();
    });
});
