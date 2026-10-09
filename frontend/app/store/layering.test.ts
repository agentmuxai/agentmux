// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// The store layer doesn't import the view layer: a module the stores need
// lives in the store (or below). Type-only imports are fine, as they leave no
// runtime dependency. The imports listed in REMAINING predate this check; the
// list only shrinks (docs/specs/PLAN_CI_TEST_SPEED_AND_DRY_FOLLOWUPS_2026_10_09.md
// step 15).

import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { describe, expect, it } from "vitest";

const repoRoot = resolve(__dirname, "../../..");
const storeDir = resolve(repoRoot, "frontend/app/store");

/** store file → the view module it imports, as `file -> module`. */
const REMAINING = [
    "frontend/app/store/agent-document-store.ts -> frontend/app/view/agent/virtualization/perf-probe",
    "frontend/app/store/agent-document/reducer.ts -> frontend/app/view/agent/activity/tool-adapter",
    "frontend/app/store/agent-document/reducer.ts -> frontend/app/view/agent/live-feed",
    "frontend/app/store/agent-document/reducer.ts -> frontend/app/view/agent/tool-result-unload",
    "frontend/app/store/agent-pane-layout-store.ts -> frontend/app/view/agent/virtualization/perf-probe",
    "frontend/app/store/agent-pane-registration.ts -> frontend/app/view/agent/activity/task-outcomes",
    "frontend/app/store/command-registry.ts -> frontend/app/view/section-pane/panes",
    "frontend/app/store/keymodel.ts -> frontend/app/view/agent/composer-focus",
    "frontend/app/store/keymodel.ts -> frontend/app/view/term/term-models",
];

function walk(dir: string, out: string[] = []): string[] {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        const f = join(dir, e.name);
        if (e.isDirectory()) walk(f, out);
        else if (/\.tsx?$/.test(e.name) && !/\.test\.tsx?$/.test(e.name)) out.push(f);
    }
    return out;
}

const repoPath = (f: string) => relative(repoRoot, f).split(sep).join("/");

/** The repo path (no extension) a specifier in `file` names, or null for a package. */
function target(file: string, spec: string): string | null {
    if (spec.startsWith("@/app/")) return "frontend/app/" + spec.slice("@/app/".length);
    if (spec.startsWith("@/store/")) return "frontend/app/store/" + spec.slice("@/store/".length);
    if (spec.startsWith("@/view/")) return "frontend/app/view/" + spec.slice("@/view/".length);
    if (spec.startsWith(".")) return posix.normalize(posix.join(repoPath(dirname(file)), spec));
    return null;
}

describe("store layering", () => {
    it("imports nothing from the view layer but what REMAINING lists", () => {
        const found = new Set<string>();
        for (const file of walk(storeDir)) {
            const text = readFileSync(file, "utf8");
            for (const m of text.matchAll(/^import\s+(type\s+)?[^;]*?from\s+["']([^"']+)["']/gm)) {
                if (m[1]) continue;
                const t = target(file, m[2]);
                if (t?.startsWith("frontend/app/view/")) found.add(`${repoPath(file)} -> ${t.replace(/\.tsx?$/, "")}`);
            }
        }
        expect([...found].filter((x) => !REMAINING.includes(x)).sort(), "new store → view imports").toEqual([]);
        expect(REMAINING.filter((x) => !found.has(x)), "gone: delete them from REMAINING").toEqual([]);
    });
});
