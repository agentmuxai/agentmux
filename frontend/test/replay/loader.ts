// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * NDJSON fixture loader for agent-pane session replay.
 *
 * Strict shape validation: a fixture must start with a `header`, end
 * optionally with a `trailer`, and have monotonically-increasing
 * `seq` numbers in between. Validation errors throw so the test
 * driver surfaces them as test failures rather than mysterious
 * downstream replay errors.
 */

import { readFileSync } from "node:fs";
import type {
    Fixture,
    FixtureEvent,
    FixtureHeader,
    FixtureLine,
    FixtureTrailer,
} from "./types";

export function loadFixture(absPath: string): Fixture {
    const raw = readFileSync(absPath, "utf8");
    const lines = raw.split(/\r?\n/).filter((l) => l.trim().length > 0);
    if (lines.length === 0) {
        throw new Error(`fixture is empty: ${absPath}`);
    }

    const parsed: FixtureLine[] = lines.map((line, i) => {
        try {
            return JSON.parse(line) as FixtureLine;
        } catch (e) {
            throw new Error(
                `fixture ${absPath} line ${i + 1}: invalid JSON — ${(e as Error).message}\n  line: ${line.slice(0, 200)}`,
            );
        }
    });

    // First line must be header.
    const first = parsed[0];
    if (!isHeader(first)) {
        throw new Error(`fixture ${absPath} line 1: must be a header (kind: "header")`);
    }
    if (first.version !== 1) {
        throw new Error(
            `fixture ${absPath}: unsupported version ${first.version} (expected 1)`,
        );
    }

    // Last line may be trailer.
    const last = parsed[parsed.length - 1];
    const trailer = isTrailer(last) ? last : null;
    const eventLines = trailer ? parsed.slice(1, -1) : parsed.slice(1);

    const events: FixtureEvent[] = [];
    let prevSeq = 0;
    for (let i = 0; i < eventLines.length; i++) {
        const ev = eventLines[i];
        if (isHeader(ev) || isTrailer(ev)) {
            throw new Error(
                `fixture ${absPath} line ${i + 2}: extra header/trailer not allowed`,
            );
        }
        const fev = ev as FixtureEvent;
        if (typeof fev.seq !== "number" || fev.seq <= prevSeq) {
            throw new Error(
                `fixture ${absPath} line ${i + 2}: seq must be strictly increasing (got ${fev.seq}, prev ${prevSeq})`,
            );
        }
        prevSeq = fev.seq;
        if (fev.src !== "stream-json" && fev.src !== "wps" && fev.src !== "dispatch") {
            throw new Error(
                `fixture ${absPath} line ${i + 2}: invalid src "${fev.src}" (expected stream-json / wps / dispatch)`,
            );
        }
        events.push(fev);
    }

    return { header: first, events, trailer };
}

function isHeader(line: unknown): line is FixtureHeader {
    return (
        typeof line === "object" &&
        line !== null &&
        (line as { kind?: unknown }).kind === "header"
    );
}

function isTrailer(line: unknown): line is FixtureTrailer {
    return (
        typeof line === "object" &&
        line !== null &&
        (line as { kind?: unknown }).kind === "trailer"
    );
}
