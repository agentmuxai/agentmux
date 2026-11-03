// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { readdirSync, readFileSync } from "node:fs";
import { join, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";
import {
    isContainerAgentMode,
    isContainerAgentType,
    isContainerRuntime,
    parseRuntimeKind,
    runtimeKindOfAgentMode,
    runtimeKindOfAgentType,
    runtimeKindOfBlockMeta,
} from "./runtime-kind";

describe("runtime kind", () => {
    it('reads "container" as a container and everything else as the host, as srv does', () => {
        expect(runtimeKindOfAgentMode("container")).toBe("container");
        for (const v of ["host", "standalone", "", "Container", "docker", undefined, null, 3, {}]) {
            expect(runtimeKindOfAgentMode(v)).toBe("host");
        }
        expect(runtimeKindOfAgentType("container")).toBe("container");
        expect(runtimeKindOfAgentType(undefined)).toBe("host");
    });

    it("reads a pane's runtime from its block meta", () => {
        expect(runtimeKindOfBlockMeta({ agentMode: "container" })).toBe("container");
        expect(runtimeKindOfBlockMeta({ agentMode: "host" })).toBe("host");
        expect(runtimeKindOfBlockMeta({})).toBe("host");
        expect(runtimeKindOfBlockMeta(undefined)).toBe("host");
    });

    it('parses exactly "host" and "container", nothing else', () => {
        expect(parseRuntimeKind("host")).toBe("host");
        expect(parseRuntimeKind("container")).toBe("container");
        for (const v of ["standalone", "", "Host", undefined, null, 0]) expect(parseRuntimeKind(v)).toBeUndefined();
    });

    it("answers is-container for a kind, a mode and a type", () => {
        expect(isContainerRuntime("container")).toBe(true);
        expect(isContainerRuntime("host")).toBe(false);
        expect(isContainerAgentMode("container")).toBe(true);
        expect(isContainerAgentMode("host")).toBe(false);
        expect(isContainerAgentMode(undefined)).toBe(false);
        expect(isContainerAgentType("container")).toBe(true);
        expect(isContainerAgentType("standalone")).toBe(false);
    });
});

const repoRoot = resolve(__dirname, "../..");
const frontendDir = resolve(repoRoot, "frontend");

function walk(dir: string, out: string[] = []): string[] {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "node_modules") continue;
        const f = join(dir, e.name);
        if (e.isDirectory()) walk(f, out);
        else if (/\.tsx?$/.test(e.name) && !/\.(test|bench)\.tsx?$/.test(e.name)) out.push(f);
    }
    return out;
}

/** A line's code: the text before a `//` comment, and nothing for a line inside a block comment. */
function codeOf(line: string): string {
    const t = line.trim();
    if (t.startsWith("*") || t.startsWith("/*") || t.startsWith("//")) return "";
    return line.split("//")[0];
}

// Mirrors srv's `nothing_but_the_adapter_compares_a_mode_to_container`
// (crates/srv/src/backend/runtime_adapter.rs): "is this a container agent?" is answered by
// runtime-kind.ts, and a new hand-written comparison is a branch it should own.
const COMPARES_TO_CONTAINER = /(?:[!=]==?|\bcase)\s*["']container["']|["']container["']\s*[!=]==?/;

describe("nothing but runtime-kind.ts compares a mode to container", () => {
    // Files that still read the string, and why. The list only shrinks.
    const allowed: string[] = [
        "frontend/util/runtime-kind.ts", // the one derivation
    ];

    it("finds no hand-written comparison outside the allowed files", () => {
        const offenders: string[] = [];
        for (const file of walk(frontendDir)) {
            const rel = relative(repoRoot, file).split(sep).join("/");
            if (allowed.includes(rel)) continue;
            readFileSync(file, "utf8")
                .split("\n")
                .forEach((line, i) => {
                    if (COMPARES_TO_CONTAINER.test(codeOf(line))) offenders.push(`${rel}:${i + 1}`);
                });
        }
        expect(offenders, "compare through frontend/util/runtime-kind.ts, not the string").toEqual([]);
    });

    it("sees the comparisons it is meant to forbid", () => {
        for (const bad of [
            'if (agentMode === "container") {',
            "x.agent_type !== 'container'",
            'return "container" === mode;',
            'case "container":',
            'a == "container"',
        ]) {
            expect(COMPARES_TO_CONTAINER.test(codeOf(bad)), bad).toBe(true);
        }
        for (const ok of [
            'agentType: "host" | "container";',
            '// if (agentMode === "container")',
            ' * `agentMode === "container"` forces false',
            'pickRuntime("container")',
            'const kinds = ["host", "container"];',
        ]) {
            expect(COMPARES_TO_CONTAINER.test(codeOf(ok)), ok).toBe(false);
        }
    });
});
