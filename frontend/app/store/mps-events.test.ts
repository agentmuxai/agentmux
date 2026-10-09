// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `WpsEvent` is checked against the backend's source, so an event name can't
// drift between the two, and no subscription spells one by hand.

import { readdirSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { WpsEvent } from "./mps-events";

const repoRoot = resolve(__dirname, "../../..");

function walk(dir: string, keep: (file: string) => boolean, out: string[] = []): string[] {
    for (const e of readdirSync(dir, { withFileTypes: true })) {
        if (e.name === "target" || e.name === "node_modules") continue;
        const f = join(dir, e.name);
        if (e.isDirectory()) walk(f, keep, out);
        else if (keep(f)) out.push(f);
    }
    return out;
}

const names = new Set<string>(Object.values(WpsEvent));

/** Events the frontend publishes itself, through srv's broker. */
const PUBLISHED_BY_THE_FRONTEND = new Set<string>([WpsEvent.SingletonClaim]);

describe("WpsEvent", () => {
    it("has every EVENT_* name mps.rs declares", () => {
        const rust = readFileSync(resolve(repoRoot, "crates/srv/src/backend/mps.rs"), "utf8");
        const declared = [...rust.matchAll(/pub const EVENT_\w+: &str = "([^"]+)";/g)].map((m) => m[1]);
        expect(declared.length).toBeGreaterThan(20);
        expect(declared.filter((n) => !names.has(n))).toEqual([]);
    });

    it("names only events the backend publishes (srv, cef, bashwrap, …)", () => {
        const backend = walk(resolve(repoRoot, "crates"), (f) => f.endsWith(".rs"))
            .map((f) => readFileSync(f, "utf8"))
            .join("\n");
        expect([...names].filter((n) => !PUBLISHED_BY_THE_FRONTEND.has(n) && !backend.includes(`"${n}"`))).toEqual([]);
    });

    it("is where every event name is spelled: no subscription or event constant elsewhere has its own", () => {
        const files = walk(resolve(repoRoot, "frontend/app"), (f) => /\.tsx?$/.test(f) && !/\.test\.tsx?$/.test(f) && !f.endsWith("mps-events.ts"));
        // As `eventType`, or as a constant named for an event. A template string
        // naming one object's event (`dronerun:${id}`) is fine.
        const literal = /(?:eventType:|const \w*EVENT\w*\s*=)\s*(?:"([^"]+)"|'([^']+)'|`([^`$]+)`)/g;
        const raw = files.flatMap((f) =>
            [...readFileSync(f, "utf8").matchAll(literal)].map((m) => `${f.slice(repoRoot.length + 1)}: ${m[1] ?? m[2] ?? m[3]}`)
        );
        expect(raw).toEqual([]);
    });
});
