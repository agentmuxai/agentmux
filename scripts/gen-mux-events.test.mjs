// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-mux-events.mjs: the generated MuxEventPayloads map, and the check that a typed event
// isn't subscribed the untyped way.

import { describe, expect, it } from "vitest";
import { eventShapedHandlers, renderEvents, untypedHistoryReads, untypedSubscriptions, wpsNames } from "./gen-mux-events.mjs";

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
            ["frontend/k.ts", "const re = /[/*]/;\n[WpsEvent.SysInfo].map((eventType) => muxEventSubscribe({ eventType, handler }));"],
            ["frontend/l.ts", "const t = `// ${a} /*`;\n[WpsEvent.SysInfo].map((eventType) => muxEventSubscribe({ eventType, handler }));"],
        ];
        expect(untypedSubscriptions(sources, typed, wps).map((e) => e.split(":")[0])).toEqual(["frontend/i.ts", "frontend/j.ts", "frontend/k.ts", "frontend/l.ts"]);
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

describe("eventShapedHandlers", () => {
    it("refuses a handler that takes the event: typed any, unknown or MuxEvent, or reading .data", () => {
        const sources = [
            ["frontend/a.ts", "onMuxEvent(WpsEvent.SysInfo, (event: any) => use(event));"],
            ["frontend/b.ts", "onMuxEvent(WpsEvent.SysInfo, (e: MuxEvent) => use(e));"],
            ["frontend/c.ts", "onMuxEvent(WpsEvent.SysInfo, (ev) => {\n    const d = ev?.data;\n});"],
            ["frontend/d.tsx", "onMuxEvent(`dronerun:${id}`, function (event: unknown) { use(event); });"],
            ["frontend/i.ts", "onMuxEvent(WpsEvent.SysInfo, (event) => use((event as any)?.data?.values));"],
        ];
        expect(eventShapedHandlers(sources).map((e) => e.split(":").slice(0, 2).join(":"))).toEqual([
            "frontend/a.ts:1",
            "frontend/b.ts:1",
            "frontend/c.ts:1",
            "frontend/d.tsx:1",
            "frontend/i.ts:1",
        ]);
    });

    it("leaves handlers that take the payload alone, and a .data on something else", () => {
        const sources = [
            ["frontend/e.ts", "onMuxEvent(WpsEvent.SysInfo, (data) => use(data.values));"],
            ["frontend/f.ts", "onMuxEvent(WpsEvent.SysInfo, ({ values }) => use(values));"],
            ["frontend/g.ts", "onMuxEvent(WpsEvent.SysInfo, (data, event) => use(data, event.data));"],
            ["frontend/h.ts", "onMuxEvent(WpsEvent.SysInfo, refresh);\nmuxEventSubscribe({ eventType: \"x\", handler: (e: any) => e.data });"],
        ];
        expect(eventShapedHandlers(sources)).toEqual([]);
    });
});

describe("untypedHistoryReads", () => {
    const wps = wpsNames('export const WpsEvent = {\n    SysInfo: "sysinfo",\n    SingletonClaim: "singleton:claim",\n} as const;');
    const typed = new Set(["sysinfo"]);

    it("refuses EventReadHistoryCommand of a typed event, however it is named", () => {
        const sources = [
            ["frontend/a.ts", "RpcApi.EventReadHistoryCommand(TabRpcClient, { event: WpsEvent.SysInfo, scope, maxitems: 1 });"],
            ["frontend/b.ts", 'RpcApi.EventReadHistoryCommand(client, { event: "sysinfo", scope, maxitems: 1 });'],
            ["frontend/c.ts", "const EVENT_SYS = WpsEvent.SysInfo;\nvoid RpcApi.EventReadHistoryCommand(client, {\n    event: EVENT_SYS,\n    scope,\n});"],
            ["frontend/d.ts", "RpcApi.EventReadHistoryCommand(client, { event: `dronerun:${id}`, scope: \"\" });"],
        ];
        expect(untypedHistoryReads(sources, typed, wps, new Set(["dronerun:"])).map((e) => e.split(":").slice(0, 2).join(":"))).toEqual([
            "frontend/a.ts:1",
            "frontend/b.ts:1",
            "frontend/c.ts:2",
            "frontend/d.ts:1",
        ]);
    });

    it("refuses a read whose event it can't see: a request built elsewhere, or an unknown name", () => {
        const sources = [
            ["frontend/g.ts", "const request = { event: WpsEvent.SysInfo, scope, maxitems: 1 };\nRpcApi.EventReadHistoryCommand(client, request);"],
            ["frontend/h.ts", "RpcApi.EventReadHistoryCommand(client, { event: pick(), scope, maxitems: 1 });"],
            ["frontend/i.ts", "RpcApi.EventReadHistoryCommand(client, { event, scope, maxitems: 1 });"],
        ];
        expect(untypedHistoryReads(sources, typed, wps).map((e) => e.split(":").slice(0, 2).join(":"))).toEqual([
            "frontend/g.ts:2",
            "frontend/h.ts:1",
            "frontend/i.ts:1",
        ]);
    });

    it("refuses an aliased or destructured reference, and a concrete per-id name", () => {
        const sources = [
            ["frontend/j.ts", "const readHistory = RpcApi.EventReadHistoryCommand;\nreadHistory(client, { event: WpsEvent.SysInfo, scope });"],
            ["frontend/k.ts", "const { EventReadHistoryCommand } = RpcApi;"],
            ["frontend/l.ts", 'RpcApi.EventReadHistoryCommand(client, { event: "dronerun:known-id", scope: "" });'],
        ];
        expect(untypedHistoryReads(sources, typed, wps, new Set(["dronerun:"])).map((e) => e.split(":").slice(0, 2).join(":"))).toEqual([
            "frontend/j.ts:1",
            "frontend/k.ts:1",
            "frontend/l.ts:1",
        ]);
    });

    it("leaves the generated stub's own declaration alone", () => {
        const sources = [["frontend/app/store/rpc-api/generated.ts", "export const RpcApi = {\n  EventReadHistoryCommand(client: RpcClient, data: X) {\n    return call(client);\n  },\n};"]];
        expect(untypedHistoryReads(sources, typed, wps)).toEqual([]);
    });

    it("leaves untyped events, and the typed helper itself, alone", () => {
        const sources = [
            ["frontend/e.ts", "RpcApi.EventReadHistoryCommand(client, { event: WpsEvent.SingletonClaim, scope, maxitems: 1 });"],
            ["frontend/app/store/mps-history.ts", "RpcApi.EventReadHistoryCommand(TabRpcClient, { event, scope, maxitems });"],
            ["frontend/f.ts", "readMuxEventHistory(WpsEvent.SysInfo, scope, 5);"],
        ];
        expect(untypedHistoryReads(sources, typed, wps)).toEqual([]);
        expect(untypedSubscriptions([sources[2]], typed, wps)).toEqual([]);
    });
});
