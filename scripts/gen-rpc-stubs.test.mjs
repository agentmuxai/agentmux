// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for gen-rpc-stubs.mjs.

import { describe, expect, it } from "vitest";
import { generate, generatedOnlyProblems, namesUsed, renderStub, tsOf } from "./gen-rpc-stubs.mjs";

const row = (command, req, resp, reqImports = [], respImports = []) => ({
    command,
    requestTs: { name: req, imports: reqImports },
    responseTs: { name: resp, imports: respImports },
});

describe("renderStub", () => {
    it("a required request becomes a required data parameter", () => {
        const src = renderStub({ command: "fs.list", method: "FsListCommand" }, row("fs.list", "FsListReq", "FsListResult"));
        expect(src).toContain("FsListCommand(client: RpcClient, data: FsListReq, opts?: RpcOpts): Promise<FsListResult> {");
        expect(src).toContain('return client.rpcCall("fs.list", data, opts);');
    });

    it("an Option request becomes an optional parameter that also takes null", () => {
        const src = renderStub({ command: "x", method: "XCommand" }, row("x", "XReq | null", "XResult"));
        expect(src).toContain("data?: XReq | null");
    });

    it("keeps `data ?? {}` for the stubs that send an empty object when given nothing", () => {
        const src = renderStub({ command: "fs.places", method: "FsPlacesCommand", send: "data ?? {}" }, row("fs.places", "FsPlacesReq | null", "FsPlacesResult"));
        expect(src).toContain('rpcCall("fs.places", data ?? {}, opts)');
    });

    it("a no-argument command takes no data, and sends {} unless told to send null", () => {
        expect(renderStub({ command: "a", method: "ACommand" }, row("a", "NoArgsReq | null", "null"))).toContain(
            'ACommand(client: RpcClient, opts?: RpcOpts): Promise<void> {\n        return client.rpcCall("a", {}, opts);'
        );
        expect(renderStub({ command: "b", method: "BCommand", send: "null" }, row("b", "NoArgsReq | null", "null"))).toContain('rpcCall("b", null, opts)');
        expect(renderStub({ command: "c", method: "CCommand" }, row("c", "null", "CResult"))).toContain("CCommand(client: RpcClient, opts?: RpcOpts)");
    });

    it("a () request sends null by default, and can't be told to send {}", () => {
        // serde deserializes () only from JSON null: {} would fail before the handler runs.
        expect(renderStub({ command: "u", method: "UCommand" }, row("u", "null", "UResult"))).toContain('rpcCall("u", null, opts)');
        expect(() => renderStub({ command: "u", method: "UCommand", send: "{}" }, row("u", "null", "UResult"))).toThrow(/only deserializes from null/);
    });

    it("an empty-struct request must send {}", () => {
        expect(() => renderStub({ command: "e", method: "ECommand", send: "null" }, row("e", "NoArgsReq", "null"))).toThrow(/only deserializes from {}/);
    });

    it("a unit response is Promise<void>, and raw JSON is any", () => {
        expect(renderStub({ command: "u", method: "UCommand" }, row("u", "UReq", "null"))).toContain("Promise<void>");
        expect(renderStub({ command: "j", method: "JCommand" }, row("j", "JsonValue", "JsonValue"))).toContain("data: any, opts?: RpcOpts): Promise<any>");
    });

    it("keeps an opts override such as a longer timeout", () => {
        const src = renderStub({ command: "l", method: "LCommand", opts: "{ timeout: 360000, ...opts }" }, row("l", "LReq", "LResp"));
        expect(src).toContain('rpcCall("l", data, { timeout: 360000, ...opts })');
    });

    it("a struct request whose fields all default can be sent as {} with no data parameter", () => {
        expect(renderStub({ command: "s", method: "SCommand", send: "{}" }, row("s", "SReq", "SResp"))).toContain("SCommand(client: RpcClient, opts?: RpcOpts)");
    });

    it("an optional request the stub always sends empty keeps a no-data signature", () => {
        const src = renderStub({ command: "m", method: "MCommand", send: "{}" }, row("m", "MReq | null", "MResp"));
        expect(src).toContain('MCommand(client: RpcClient, opts?: RpcOpts): Promise<MResp> {\n        return client.rpcCall("m", {}, opts);');
    });

    it("data ?? {} makes a required request's parameter optional", () => {
        const src = renderStub({ command: "r", method: "RCommand", send: "data ?? {}" }, row("r", "RReq", "RResp"));
        expect(src).toContain("data?: RReq, opts?: RpcOpts");
        expect(src).toContain('rpcCall("r", data ?? {}, opts)');
    });

    it("rejects a send value that doesn't fit the request", () => {
        expect(() => renderStub({ command: "r", method: "RCommand", send: "null" }, row("r", "RReq", "RResp"))).toThrow(/needs an Option or \(\) request/);
        expect(() => renderStub({ command: "n", method: "NCommand", send: "data" }, row("n", "NoArgsReq", "null"))).toThrow(/no-argument/);
        expect(() => renderStub({ command: "x", method: "XCommand", send: "whatever" }, row("x", "XReq", "XResp"))).toThrow(/"send" is/);
    });
});

describe("namesUsed and tsOf", () => {
    it("collects every type name in the signature, but not built-ins or the no-argument request", () => {
        expect([...namesUsed({}, row("x", "XReq | null", "Array<Item>"))].sort()).toEqual(["Item", "XReq"]);
        expect([...namesUsed({}, row("x", "NoArgsReq | null", "Result"))]).toEqual(["Result"]);
        expect([...namesUsed({}, row("x", "Req", "{ [key in string]?: string | null }"))]).toEqual(["Req"]);
    });

    it("maps JsonValue to any", () => {
        expect(tsOf("JsonValue")).toBe("any");
        expect(tsOf("Other")).toBe("Other");
    });
});

describe("generate", () => {
    const base = {
        schema: [row("fs.list", "FsListReq", "FsListResult"), row("skill.get", "SkillReq", "Skill")],
        ambient: new Set(["Skill", "RpcOpts"]),
        generatedTypes: new Set(["FsListReq", "FsListResult", "SkillReq"]),
        hand: new Map(),
    };

    it("imports generated types and uses hand-written globals by name", () => {
        const { source, errors } = generate({ ...base, manifest: { stubs: [{ command: "fs.list", method: "FsListCommand" }, { command: "skill.get", method: "SkillGetCommand" }] } });
        expect(errors).toBeUndefined();
        expect(source).toContain('import type { FsListReq } from "@/types/rpc/FsListReq";');
        expect(source).toContain('import type { SkillReq } from "@/types/rpc/SkillReq";');
        expect(source).not.toContain('from "@/types/rpc/Skill"');
        expect(source).toContain("Promise<Skill>");
    });

    it("refuses a command srv doesn't register as typed", () => {
        const { errors } = generate({ ...base, manifest: { stubs: [{ command: "nope", method: "NopeCommand" }] } });
        expect(errors.join()).toMatch(/doesn't register it with register_typed/);
    });

    it("refuses a type that is neither generated nor a global", () => {
        const { errors } = generate({ ...base, schema: [row("x", "Unknown", "null")], manifest: { stubs: [{ command: "x", method: "XCommand" }] } });
        expect(errors.join()).toMatch(/type Unknown is neither generated/);
    });

    it("refuses a method that is still hand-written", () => {
        const { errors } = generate({ ...base, hand: new Map([["FsListCommand", "fs.ts"]]), manifest: { stubs: [{ command: "fs.list", method: "FsListCommand" }] } });
        expect(errors.join()).toMatch(/still hand-written in rpc-api\/fs.ts/);
    });

    it("refuses duplicates", () => {
        const { errors } = generate({
            ...base,
            manifest: { stubs: [{ command: "fs.list", method: "FsListCommand" }, { command: "fs.list", method: "OtherCommand" }, { command: "skill.get", method: "OtherCommand" }] },
        });
        expect(errors.join()).toMatch(/fs.list: listed twice/);
        expect(errors.join()).toMatch(/OtherCommand: two commands generate it/);
    });
});

describe("raw-JSON overrides", () => {
    it("dataType and returns keep a precise type where the Rust side is serde_json::Value", () => {
        const src = renderStub({ command: "w", method: "WCommand", dataType: "WidgetHealthReq", returns: "FullConfigType" }, row("w", "JsonValue", "JsonValue"));
        expect(src).toContain("data: WidgetHealthReq, opts?: RpcOpts): Promise<FullConfigType>");
        expect([...namesUsed({ dataType: "WidgetHealthReq", returns: "FullConfigType" }, row("w", "JsonValue", "JsonValue"))].sort()).toEqual(["FullConfigType", "WidgetHealthReq"]);
    });

    it("refuses to override a type that isn't raw JSON", () => {
        expect(() => renderStub({ command: "t", method: "TCommand", returns: "Other" }, row("t", "TReq", "TResp"))).toThrow(/overrides only a serde_json::Value response/);
        const { errors } = generate({
            schema: [row("t", "TReq", "TResp")],
            manifest: { stubs: [{ command: "t", method: "TCommand", dataType: "Other" }] },
            ambient: new Set(),
            generatedTypes: new Set(["TReq", "TResp"]),
            hand: new Map(),
        });
        expect(errors.join()).toMatch(/overrides only a serde_json::Value request/);
    });
});

describe("optionalFields", () => {
    const fieldsOf = (t) => (t === "CommandResolveCliData" ? new Set(["provider_id", "cli_command", "block_id"]) : null);
    const schema = [row("resolvecli", "CommandResolveCliData", "Result")];
    const base = { schema, ambient: new Set(["Result"]), generatedTypes: new Set(["CommandResolveCliData"]), hand: new Map(), fieldsOf };

    it("makes fields srv defaults optional on the generated request type", () => {
        const src = renderStub({ command: "resolvecli", method: "ResolveCliCommand", optionalFields: ["block_id"] }, schema[0]);
        expect(src).toContain('data: Omit<CommandResolveCliData, "block_id"> & Partial<Pick<CommandResolveCliData, "block_id">>');
        const { errors } = generate({ ...base, manifest: { stubs: [{ command: "resolvecli", method: "ResolveCliCommand", optionalFields: ["block_id"] }] } });
        expect(errors).toBeUndefined();
    });

    it("refuses a field the generated type doesn't have", () => {
        const { errors } = generate({ ...base, manifest: { stubs: [{ command: "resolvecli", method: "ResolveCliCommand", optionalFields: ["nope"] }] } });
        expect(errors.join()).toMatch(/has no field\(s\) nope/);
    });

    it("refuses a request type with no generated file", () => {
        const { errors } = generate({ ...base, schema: [row("x", "Elsewhere", "Result")], generatedTypes: new Set(), ambient: new Set(["Result", "Elsewhere"]), manifest: { stubs: [{ command: "x", method: "XCommand", optionalFields: ["a"] }] } });
        expect(errors.join()).toMatch(/needs a generated request type/);
    });
});

describe("module type overrides", () => {
    it("an import(...) type in an override is left to tsc, not checked as a global", () => {
        const names = namesUsed({ returns: 'import("@/app/store/ambient-outcomes").AmbientOutcomes', dataType: "Record<string, never>" }, row("a", "JsonValue", "JsonValue"));
        expect([...names]).toEqual([]);
    });
});

describe("generatedOnlyProblems", () => {
    const ok = { hand: new Map(), indexSource: "export const RpcApi = GeneratedApi;\n", sources: [] };

    it("passes when every call goes through a generated stub", () => {
        const sources = [["frontend/a.ts", 'await RpcApi.PaneOpenCommand(TabRpcClient, { view: "term" }, {});']];
        expect(generatedOnlyProblems({ ...ok, sources })).toEqual([]);
    });

    it("refuses a hand-written stub and an RpcApi that spreads anything else", () => {
        const errors = generatedOnlyProblems({
            hand: new Map([["FooCommand", "misc.ts"]]),
            indexSource: "export const RpcApi = { ...MiscApi, ...GeneratedApi };\n",
            sources: [],
        });
        expect(errors).toHaveLength(2);
        expect(errors[0]).toMatch(/rpc-api\/misc\.ts: FooCommand is a hand-written stub/);
        expect(errors[1]).toMatch(/RpcApi must be exactly/);
    });

    it("refuses any raw rpcCall or rpcStream, whatever names the command", () => {
        const sources = [
            ["frontend/x.ts", 'void TabRpcClient.rpcCall("pane.open", { view: "settings" }, {});'],
            ["frontend/y.ts", "await TabRpcClient.rpcCall(\n    \"pane.moveTab\",\n    { block_id: b },\n);"],
            ["frontend/z.ts", "const cmd = pick();\nawait client.rpcCall(cmd, data, opts);"],
            ["frontend/s.ts", 'for await (const x of TabRpcClient.rpcStream<Foo>("tail", {}, {})) use(x);'],
            // A mention in a comment isn't a call.
            ["frontend/c.ts", "// we used to call client.rpcCall(...) here"],
        ];
        const msg = (where, fn) => `${where}: calls srv with ${fn} directly; call the command's generated stub on RpcApi instead`;
        expect(generatedOnlyProblems({ ...ok, sources })).toEqual([
            msg("frontend/x.ts:1", "rpcCall"),
            msg("frontend/y.ts:1", "rpcCall"),
            msg("frontend/z.ts:2", "rpcCall"),
            msg("frontend/s.ts:1", "rpcStream"),
        ]);
    });
});
