// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The trigger fields a node is judged by belong to the drone that is open: a late answer for another drone is dropped. */

import { describe, expect, it, vi } from "vitest";

type Answer = (v: unknown) => void;
const asked = new Map<string, Answer>();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListDronesCommand: () => Promise.resolve([]),
        ListDroneRunsCommand: () => Promise.resolve([]),
        GetDroneCommand: (_c: unknown, { id }: { id: string }) => Promise.resolve({ id, name: id, graph: { nodes: [], edges: [] } }),
        GetDronePayloadSchemaCommand: (_c: unknown, { drone_id }: { drone_id: string }) =>
            new Promise((resolve) => asked.set(drone_id, resolve)),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { DroneViewModel } from "./drone-model";

const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
};
const schema = (job: string) => ({ by_hand: [], jobs: [{ job_id: job, job_name: job, trigger: "poll", fields: [] }] });

describe("DroneViewModel payload schema", () => {
    it("drops the answer for a drone that is no longer open", async () => {
        const model = new DroneViewModel({ blockId: "schema-block", meta: () => ({}) } as unknown as PaneTabHostContext);
        await model.openDrone("a");
        await model.openDrone("b");
        await flush();
        expect(model.payloadSchemaAtom()).toBeNull();
        asked.get("b")!(schema("job-b"));
        await flush();
        asked.get("a")!(schema("job-a"));
        await flush();
        expect(model.payloadSchemaAtom()?.jobs[0].job_id).toBe("job-b");
        model.dispose();
    });

    it("drops an answer that arrives after a blank drone was started", async () => {
        const model = new DroneViewModel({ blockId: "schema-block-2", meta: () => ({}) } as unknown as PaneTabHostContext);
        await model.openDrone("c");
        await flush();
        model.newDrone();
        asked.get("c")!(schema("job-c"));
        await flush();
        expect(model.payloadSchemaAtom()).toBeNull();
        model.dispose();
    });
});
