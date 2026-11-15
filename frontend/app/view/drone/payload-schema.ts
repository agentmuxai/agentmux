// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// What a drone can read of what starts it, and which of it is outside text
// (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.6). The schemas come from srv
// (`getdronepayloadschema`); this reads a node's text for `{{trigger.…}}` references and
// says which of them are outside text. It is a hint for the person: srv's rules decide.

import type { PayloadField } from "@/types/rpc/PayloadField";
import type { PayloadSchemaResp } from "@/types/rpc/PayloadSchemaResp";

/** A reference a node's text makes to the trigger, and whether the field is outside text. */
export interface TriggerRef {
    /** The path under `trigger`, as written (`item.title`). */
    path: string;
    outside: boolean;
}

const REF = /\{\{\s*trigger\.([A-Za-z0-9_.]+)/g;

/** Every `trigger.…` path the strings of `data` refer to, once each, in order. */
export function triggerPathsIn(data: Record<string, unknown>): string[] {
    const found: string[] = [];
    const visit = (v: unknown, depth: number): void => {
        if (typeof v === "string") {
            for (const m of v.matchAll(REF)) {
                const path = m[1].replace(/\.+$/, "");
                if (path && !found.includes(path)) found.push(path);
            }
        } else if (depth < 4 && v && typeof v === "object") {
            for (const x of Object.values(v as Record<string, unknown>)) visit(x, depth + 1);
        }
    };
    visit(data, 0);
    return found;
}

/** Whether `path` is outside text under `fields`: it is unless the nearest field that names it, or a parent of it, is clean. */
export function isOutsideText(fields: PayloadField[], path: string): boolean {
    const under = (p: string, of: string) => p === of || p.startsWith(`${of}.`);
    let best: PayloadField | undefined;
    for (const f of fields) {
        if (f.path === "*" || !under(path, f.path)) continue;
        if (!best || f.path.length > best.path.length) best = f;
    }
    return best ? !best.clean : true;
}

/**
 * The fields to judge a reference by. A drone with jobs is judged by their triggers (a field is clean only if every job's
 * trigger says so); a drone with no job has what a hand run has. A run started by hand or by an agent reads whatever it
 * was given: that is for srv's turn-on check, not this hint.
 */
export function judge(schema: PayloadSchemaResp | null, paths: string[]): TriggerRef[] {
    const sets: PayloadField[][] = schema && schema.jobs.length > 0 ? schema.jobs.map((j) => j.fields) : [schema?.by_hand ?? []];
    return paths.map((path) => ({ path, outside: sets.some((fields) => isOutsideText(fields, path)) }));
}
