// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Every command cef-api.ts sends has a route in the CEF host (crates/cef/src/ipc.rs, or the stub
// list in commands/stubs.rs). `open_native_path` was sent for a month with no route at all, and the
// host answered "Unknown command" to a button that was never checked against it
// (RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md §4).

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const root = resolve(__dirname, "../..");
const read = (rel: string): string => readFileSync(resolve(root, rel), "utf8");

/** Names sent by `invokeCommand("name"` and `invokeCommand<T>("name"` in cef-api.ts. */
export function sentCommands(source: string): string[] {
    const names = new Set<string>();
    for (const m of source.matchAll(/invokeCommand(?:<[^(]*?>)?\(\s*"([a-z0-9_]+)"/g)) names.add(m[1]);
    return [...names].sort();
}

/** Names the host answers: match arms of `route_command`, and the stub list. */
export function routedCommands(ipc: string, stubs: string): Set<string> {
    const names = new Set<string>();
    const start = ipc.indexOf("async fn route_command");
    const body = start < 0 ? "" : ipc.slice(start);
    for (const m of body.matchAll(/^\s*"([a-z0-9_]+)"(?:\s*\|\s*"[a-z0-9_]+")*\s*(?:=>|\|)/gm)) names.add(m[1]);
    // Arms that list several names: `"a" | "b" =>`.
    for (const m of body.matchAll(/^\s*((?:"[a-z0-9_]+"\s*\|\s*)+"[a-z0-9_]+")\s*=>/gm))
        for (const n of m[1].matchAll(/"([a-z0-9_]+)"/g)) names.add(n[1]);
    const stub = stubs.slice(stubs.indexOf("fn is_stub_command"));
    for (const m of stub.slice(0, stub.indexOf("\n}\n")).matchAll(/"([a-z0-9_]+)"/g)) names.add(m[1]);
    return names;
}

describe("cef-api.ts commands have a host route", () => {
    const sent = sentCommands(read("frontend/util/cef-api.ts"));
    const routed = routedCommands(read("crates/cef/src/ipc.rs"), read("crates/cef/src/commands/stubs.rs"));

    it("finds the commands on both sides", () => {
        expect(sent.length).toBeGreaterThan(40);
        expect(sent).toContain("reveal_in_file_explorer");
        expect(routed.size).toBeGreaterThan(40);
        expect(routed.has("reveal_in_file_explorer")).toBe(true);
        expect(routed.has("install_update")).toBe(true); // a stub
    });

    it("sends no command the host doesn't route", () => {
        expect(sent.filter((c) => !routed.has(c))).toEqual([]);
    });

    it("no longer sends open_native_path", () => {
        expect(sent).not.toContain("open_native_path");
    });
});
