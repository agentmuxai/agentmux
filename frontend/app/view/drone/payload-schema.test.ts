// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import type { PayloadField } from "@/types/rpc/PayloadField";
import type { PayloadSchemaResp } from "@/types/rpc/PayloadSchemaResp";
import { isOutsideText, judge, triggerPathsIn } from "./payload-schema";

const f = (path: string, clean: boolean): PayloadField => ({ path, kind: "text", clean, note: "" });

describe("payload-schema", () => {
    it("finds each trigger path once, in nested data and with filters", () => {
        const data = { body: "Run {{trigger.run}} failed: {{ trigger.item.title | head:20 }}", more: { x: "{{trigger.run}} {{var.a}}" } };
        expect(triggerPathsIn(data)).toEqual(["run", "item.title"]);
    });

    it("a path is outside text unless its nearest field is clean", () => {
        const fields = [f("item.num", true), f("item.title", false), f("kind", true)];
        expect(isOutsideText(fields, "item.num")).toBe(false);
        expect(isOutsideText(fields, "item.title")).toBe(true);
        expect(isOutsideText(fields, "item")).toBe(true);
        expect(isOutsideText(fields, "other")).toBe(true);
        expect(isOutsideText(fields, "kind")).toBe(false);
    });

    it("with several jobs a field is clean only if every trigger says so", () => {
        const schema: PayloadSchemaResp = {
            by_hand: [f("*", false), f("run_id", true)],
            jobs: [
                { job_id: "a", job_name: "a", trigger: "poll", fields: [f("key", true), f("run_id", true)] },
                { job_id: "b", job_name: "b", trigger: "webhook_local", fields: [f("*", false), f("run_id", true)] },
            ],
        };
        const got = judge(schema, ["run_id", "key"]);
        expect(got).toEqual([
            { path: "run_id", outside: false },
            { path: "key", outside: true },
        ]);
    });

    it("a drone with no job is judged as a hand run is", () => {
        const schema: PayloadSchemaResp = { by_hand: [f("*", false), f("run_id", true)], jobs: [] };
        expect(judge(schema, ["run_id", "item.num"])).toEqual([
            { path: "run_id", outside: false },
            { path: "item.num", outside: true },
        ]);
    });

    it("with no schema yet everything is outside text", () => {
        expect(judge(null, ["a"])).toEqual([{ path: "a", outside: true }]);
    });
});
