// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Why an instance the lab started did not open a page. The instance writes its own incidents under its data
// folder (`channels/<channel>/versions/<version>/data/incidents.jsonl`): a window process that keeps crashing
// ends with a `host_give_up` incident, and the launcher may stay running after it. Without this the lab waited two
// minutes and said only that no page had opened.

import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

/** Every incident the instance in `home` has recorded, oldest first. */
export function readIncidents(home) {
    const found = [];
    const channels = join(home, "channels");
    if (!existsSync(channels)) return found;
    for (const channel of readdirSync(channels)) {
        const versions = join(channels, channel, "versions");
        if (!existsSync(versions)) continue;
        for (const version of readdirSync(versions)) {
            const file = join(versions, version, "data", "incidents.jsonl");
            if (!existsSync(file)) continue;
            for (const line of readFileSync(file, "utf8").split("\n")) {
                if (!line.trim()) continue;
                try {
                    found.push(JSON.parse(line));
                } catch {
                    // a line being written
                }
            }
        }
    }
    return found.sort((a, b) => (a.at_ms ?? 0) - (b.at_ms ?? 0));
}

/** True once the instance has stopped restarting its window process: no page will open. */
export const hostGaveUp = (home) => readIncidents(home).some((i) => i.kind === "host_give_up");

/** A clause for the launch error naming the instance's last incident, or "". */
export function incidentNote(home) {
    const last = readIncidents(home).at(-1);
    return last ? `; the instance recorded "${last.what}" (${last.cause})` : "";
}
