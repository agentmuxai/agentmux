// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-mux-events.mjs: the generated MuxEventPayloads map, and the check that a typed event
// isn't subscribed the untyped way.

import { describe, expect, it } from "vitest";
import { renderEvents, untypedSubscriptions, wpsNames } from "./gen-mux-events.mjs";

const rows = [
    { event: "controllerstatus", payloadTs: { name: "BlockControllerRuntimeStatus", imports: ["BlockControllerRuntimeStatus"] } },
    { event: "sysinfo", payloadTs: { name: "TimeSeriesData", imports: ["TimeSeriesData"] } },
];

describe("renderEvents", () => {
    it("maps each event's wire name to its payload type, importing it", () => {
        const src = renderEvents(rows);
        expect(src).toContain('import type { BlockControllerRuntimeStatus } from "@/types/rpc/BlockControllerRuntimeStatus";');
        expect(src).toContain('"controllerstatus": BlockControllerRuntimeStatus;');
        expect(src).toContain('"sysinfo": TimeSeriesData;');
        expect(src).toContain('export const TYPED_MUX_EVENTS = ["controllerstatus", "sysinfo"] as const;');
        expect(src).toMatch(/@generated[\s\S]*DO NOT EDIT/);
    });
});

describe("untypedSubscriptions", () => {
    const wps = wpsNames('export const WpsEvent = {\n    SysInfo: "sysinfo",\n    BlockClose: "blockclose",\n} as const;');
    const typed = new Set(["sysinfo"]);

    it("reads WpsEvent's names", () => {
        expect(wps.get("SysInfo")).toBe("sysinfo");
    });

    it("refuses muxEventSubscribe of a typed event, by WpsEvent name or by literal", () => {
        const sources = [
            ["frontend/a.ts", 'muxEventSubscribe({\n    eventType: WpsEvent.SysInfo,\n    handler: (e) => use(e.data),\n});'],
            ["frontend/b.ts", 'muxEventSubscribe({ eventType: "sysinfo", handler: () => {} });'],
        ];
        const errors = untypedSubscriptions(sources, typed, wps);
        expect(errors).toHaveLength(2);
        expect(errors[0]).toMatch(/^frontend\/a\.ts:2: "sysinfo" has a typed payload; subscribe with onMuxEvent\(WpsEvent\.SysInfo/);
        expect(errors[1]).toMatch(/^frontend\/b\.ts:1: "sysinfo"/);
    });

    it("refuses a typed event's name used any other way, such as a list mapped into muxEventSubscribe", () => {
        const sources = [["frontend/e.ts", "[WpsEvent.BlockClose, WpsEvent.SysInfo].map((eventType) =>\n    muxEventSubscribe({ eventType, handler }));"]];
        expect(untypedSubscriptions(sources, typed, wps)).toEqual([
            "frontend/e.ts:1: WpsEvent.SysInfo has a typed payload; pass it straight to onMuxEvent(WpsEvent.SysInfo, (data) => …)",
        ]);
    });

    it("leaves untyped events, onMuxEvent, an RPC's event field and a named constant alone", () => {
        const sources = [
            ["frontend/c.ts", "muxEventSubscribe({ eventType: WpsEvent.BlockClose, handler: () => {} });"],
            ["frontend/d.ts", "onMuxEvent(WpsEvent.SysInfo, (data) => use(data.values));"],
            ["frontend/f.ts", "EventReadHistoryCommand(client, { event: WpsEvent.SysInfo, scope });\nconst EVENT_SYS = WpsEvent.SysInfo;"],
        ];
        expect(untypedSubscriptions(sources, typed, wps)).toEqual([]);
    });
});
