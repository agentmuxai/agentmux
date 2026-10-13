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

    it("keys a per-id event by its prefix and any id, and knows which events carry no payload", () => {
        const src = renderEvents([
            ...rows,
            { event: "agents:changed", payloadTs: { name: "null", imports: [] } },
            { event: "agent:memory:changed:", payloadTs: { name: "null", imports: [] }, perId: true },
            { event: "dronerun:", payloadTs: { name: "RunEvent", imports: ["RunEvent"] }, perId: true },
        ]);
        expect(src).toContain("[event: `agent:memory:changed:${string}`]: null;");
        expect(src).toContain("[event: `dronerun:${string}`]: RunEvent;");
        expect(src).toContain('export const TYPED_MUX_EVENTS = ["controllerstatus", "sysinfo", "agents:changed"] as const;');
        expect(src).toContain('new Set(["agents:changed"])');
        expect(src).toContain('NO_PAYLOAD_PER_ID_PREFIXES: readonly string[] = ["agent:memory:changed:"]');
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

    it("refuses muxEventSubscribe of a per-id typed event, named by a template literal", () => {
        const sources = [["frontend/g.ts", "muxEventSubscribe({ eventType: `dronerun:${runId}`, handler });\nmuxEventSubscribe({ eventType: `other:${id}`, handler });"]];
        expect(untypedSubscriptions(sources, typed, wps, new Set(["dronerun:"]))).toEqual([
            'frontend/g.ts:1: "dronerun:<id>" has a typed payload; subscribe with onMuxEvent(`dronerun:${id}`, (data) => …), not muxEventSubscribe',
        ]);
    });

    it("refuses a typed event's name used any other way, such as a list mapped into muxEventSubscribe", () => {
        const sources = [["frontend/e.ts", "[WpsEvent.BlockClose, WpsEvent.SysInfo].map((eventType) =>\n    muxEventSubscribe({ eventType, handler }));"]];
        expect(untypedSubscriptions(sources, typed, wps)).toEqual([
            "frontend/e.ts:1: WpsEvent.SysInfo has a typed payload; pass it straight to onMuxEvent(WpsEvent.SysInfo, (data) => …)",
        ]);
    });

    it("still refuses a use after a comment or a `//` inside a string on the same line", () => {
        const sources = [
            ["frontend/i.ts", "/* why */ [WpsEvent.SysInfo].map((eventType) => muxEventSubscribe({ eventType, handler }));"],
            ["frontend/j.ts", 'const url = "http://x"; [WpsEvent.SysInfo].map((eventType) => muxEventSubscribe({ eventType, handler }));'],
        ];
        expect(untypedSubscriptions(sources, typed, wps).map((e) => e.split(":")[0])).toEqual(["frontend/i.ts", "frontend/j.ts"]);
    });

    it("leaves untyped events, onMuxEvent, an RPC's event field and a named constant alone", () => {
        const sources = [
            ["frontend/c.ts", "muxEventSubscribe({ eventType: WpsEvent.BlockClose, handler: () => {} });"],
            ["frontend/d.ts", "onMuxEvent(WpsEvent.SysInfo, (data) => use(data.values));"],
            ["frontend/h.ts", "/**\n * Streamed as `WpsEvent.SysInfo` events.\n */\n// see WpsEvent.SysInfo\nconst x = 1; // WpsEvent.SysInfo"],
            ["frontend/f.ts", "EventReadHistoryCommand(client, { event: WpsEvent.SysInfo, scope });\nconst EVENT_SYS = WpsEvent.SysInfo;"],
        ];
        expect(untypedSubscriptions(sources, typed, wps)).toEqual([]);
    });
});
