// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The Trigger card at the left of the canvas names what starts the drone: a run by hand, and each job that runs it
 * with its trigger kind, and opens the Jobs pane. It is a view of the jobs, not a node of the graph. Real
 * DroneViewModel and DroneView; only RPC and the pane opener are mocked.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        ListBundlesCommand: () => Promise.resolve([]),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const openWarden = vi.fn(() => Promise.resolve());
vi.mock("@/app/view/warden/open-warden", () => ({ openWarden: (...a: unknown[]) => (openWarden as any)(...a) }));

import { DroneViewModel } from "./drone-model";
import { DroneView } from "./drone-view";
import { triggerKindLabel } from "./drone-trigger-card";

let model: DroneViewModel | null = null;

vi.stubGlobal(
    "ResizeObserver",
    class {
        observe() {}
        unobserve() {}
        disconnect() {}
    }
);

afterEach(() => {
    cleanup();
    model?.dispose();
    model = null;
    openWarden.mockClear();
});

const field = (path: string, clean: boolean) => ({ path, kind: "text", clean, note: "" });

function mount(jobs: { job_id: string; job_name: string; trigger: string }[]) {
    model = new DroneViewModel({ blockId: "drone-test-block", meta: () => ({}) } as any);
    model.setPayloadSchema({ by_hand: [field("*", false)], jobs: jobs.map((j) => ({ ...j, fields: [field("*", false)] })) });
    return render(() => <DroneView model={model!} />);
}

const rows = (root: HTMLElement) => [...root.querySelectorAll(".drone-trigger-row")].map((r) => r.textContent?.replace(/\s+/g, " ").trim());

describe("the Trigger card", () => {
    it("offers a run by hand with no job, and is not a node of the graph", () => {
        const { container } = mount([]);
        expect(container.querySelector(".drone-trigger-title")?.textContent).toBe("Trigger");
        expect(rows(container)).toEqual(["By hand▶ Run"]);
        expect(model!.draftAtom().graph.nodes).toHaveLength(0);
    });

    it("lists each job that runs the drone with how it is triggered", () => {
        const { container } = mount([
            { job_id: "a", job_name: "Nightly", trigger: "timer" },
            { job_id: "b", job_name: "Issues", trigger: "poll" },
            { job_id: "c", job_name: "Odd", trigger: "something_new" },
        ]);
        expect(rows(container)).toEqual(["By hand▶ Run", "NightlytimerOpen job", "IssuespollOpen job", "Oddsomething_newOpen job"]);
    });

    it("opens the Jobs pane from a job's row", () => {
        const { container } = mount([{ job_id: "a", job_name: "Nightly", trigger: "timer" }]);
        const open = [...container.querySelectorAll(".drone-trigger button")].find((b) => b.textContent === "Open job")!;
        fireEvent.click(open);
        expect(openWarden).toHaveBeenCalledWith("jobs");
    });

    it("keeps its Run in step with the bar's: off when the drone cannot run", () => {
        const { container } = mount([]);
        const run = container.querySelector(".drone-trigger button") as HTMLButtonElement;
        expect(run.disabled).toBe(!model!.validate().ok || model!.runningAtom());
    });

    it("names the kinds it knows and leaves another as it is", () => {
        expect(triggerKindLabel("file_watch")).toBe("file change");
        expect(triggerKindLabel("webhook_local")).toBe("local webhook");
        expect(triggerKindLabel("future_kind")).toBe("future_kind");
    });
});
