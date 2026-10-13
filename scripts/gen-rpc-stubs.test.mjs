// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Unit tests for gen-rpc-stubs.mjs.

import { describe, expect, it } from "vitest";
import { generate, namesUsed, renderStub, tsOf } from "./gen-rpc-stubs.mjs";

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

    it("a unit response is Promise<void>, and raw JSON is any", () => {
        expect(renderStub({ command: "u", method: "UCommand" }, row("u", "UReq", "null"))).toContain("Promise<void>");
        expect(renderStub({ command: "j", method: "JCommand" }, row("j", "JsonValue", "JsonValue"))).toContain("data: any, opts?: RpcOpts): Promise<any>");
    });

    it("keeps an opts override such as a longer timeout", () => {
        const src = renderStub({ command: "l", method: "LCommand", opts: "{ timeout: 360000, ...opts }" }, row("l", "LReq", "LResp"));
        expect(src).toContain('rpcCall("l", data, { timeout: 360000, ...opts })');
    });

    it("rejects a send value that doesn't fit the request", () => {
        expect(() => renderStub({ command: "r", method: "RCommand", send: "data ?? {}" }, row("r", "RReq", "RResp"))).toThrow(/only makes sense for an optional request/);
        expect(() => renderStub({ command: "n", method: "NCommand", send: "data" }, row("n", "NoArgsReq", "null"))).toThrow(/no-argument/);
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
