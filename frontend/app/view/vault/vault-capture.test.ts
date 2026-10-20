// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/** The host's page helper that finds what a pane screenshot must black out:
 *  the Vault, marked `data-agent-hidden`, where it overlaps the caller's pane
 *  (crates/cef/src/browser_api/scripts/query.js, `__amq_hidden_boxes`). */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { beforeAll, describe, expect, it } from "vitest";

const QUERY_JS = readFileSync(join(__dirname, "../../../../crates/cef/src/browser_api/scripts/query.js"), "utf8");

declare global {
    interface Window {
        __amq_hidden_boxes: (clip: { x: number; y: number; width: number; height: number }) => number[][];
    }
}

function box(el: HTMLElement, x: number, y: number, width: number, height: number) {
    el.getBoundingClientRect = () => ({ x, y, width, height, top: y, left: x, right: x + width, bottom: y + height, toJSON: () => ({}) }) as DOMRect;
}

describe("__amq_hidden_boxes", () => {
    beforeAll(() => {
        new Function(QUERY_JS)();
    });

    it("gives the part of the Vault inside the clip, from the clip's origin", () => {
        document.body.innerHTML = `<div id="vault" data-agent-hidden="vault"></div><div id="other"></div>`;
        box(document.getElementById("vault")!, 100, 50, 400, 300);
        box(document.getElementById("other")!, 0, 0, 1000, 1000);
        expect(window.__amq_hidden_boxes({ x: 200, y: 100, width: 600, height: 600 })).toEqual([[0, 0, 300, 250]]);
    });

    it("gives nothing when the Vault is elsewhere or not shown", () => {
        document.body.innerHTML = `<div id="vault" data-agent-hidden="vault"></div>`;
        box(document.getElementById("vault")!, 0, 0, 100, 100);
        expect(window.__amq_hidden_boxes({ x: 200, y: 200, width: 100, height: 100 })).toEqual([]);
        box(document.getElementById("vault")!, 0, 0, 0, 0);
        expect(window.__amq_hidden_boxes({ x: 0, y: 0, width: 100, height: 100 })).toEqual([]);
    });
});
