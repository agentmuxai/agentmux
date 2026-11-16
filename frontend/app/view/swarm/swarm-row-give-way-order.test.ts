// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The order a Swarm row's parts give way when the pane is too narrow for all of them (#47761): the age, the
 * context size, the "No pane" mark, the seat badge, the name, then the status chip. Who an agent is (name and
 * role) outlasts when it last spoke. jsdom has no layout, so this reads the SCSS source for each part's
 * `flex-shrink`: a bigger weight gives up its width first.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const scss = readFileSync(join(__dirname, "swarm-view.scss"), "utf8");

/** The `flex-shrink` of the first `selector { ... }` rule under `.swarm-agent-row`, as a number. */
function shrinkOf(selector: string): number {
    const rowStart = scss.indexOf("\n.swarm-agent-row {");
    if (rowStart < 0) throw new Error("`.swarm-agent-row` rule not found");
    const at = scss.indexOf(`    ${selector} {`, rowStart);
    if (at < 0) throw new Error(`rule not found under .swarm-agent-row: ${selector}`);
    const body = scss.slice(at, scss.indexOf("}", at));
    const longhand = /flex-shrink:\s*([0-9.e+]+)\s*;/.exec(body);
    // `flex: <grow> <shrink> <basis>`
    const shorthand = /(?:^|[\s;])flex:\s*[0-9.e+]+\s+([0-9.e+]+)\s/.exec(body);
    const m = longhand ?? shorthand;
    if (!m) throw new Error(`no flex-shrink in ${selector}`);
    return Number(m[1]);
}

describe("Swarm row: the order its parts give way", () => {
    it("gives up the age first, then the context size, the No pane mark, the seat badge and the name", () => {
        const order = [
            shrinkOf(".swarm-last-active"),
            shrinkOf(".swarm-ctx-size"),
            shrinkOf(".swarm-nopane-slot"),
            shrinkOf(".swarm-seat-slot"),
            shrinkOf(".swarm-agent-label"),
        ];
        for (let i = 1; i < order.length; i++) {
            // Far enough apart that the later part loses less than a layout unit while the earlier one still shrinks.
            expect(order[i - 1] / order[i]).toBeGreaterThanOrEqual(1e4);
        }
    });
});
