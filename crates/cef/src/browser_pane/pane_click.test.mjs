// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The click listener of a Linux browser pane (`pane_click.js`, added by `side_button_cdp.rs`): installed on a
// stand-in window, sent the events the person's input makes, and what it reports is read back. A page's own
// events (`isTrusted` false) report nothing, the side (history) buttons are not a click, and a report carries
// nothing the page could choose.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const here = dirname(fileURLToPath(import.meta.url));
const SCRIPT = readFileSync(join(here, "pane_click.js"), "utf8");

// The listener on a stand-in window: its handlers, the options it registered them with, and what it reports.
function install({ withBinding = true } = {}) {
    const handlers = {};
    const options = {};
    const reports = [];
    const win = {
        addEventListener: (type, fn, opts) => {
            (handlers[type] ||= []).push(fn);
            options[type] = opts;
        },
    };
    if (withBinding) win.agentmuxPaneClick = (payload) => reports.push(payload);
    new Function("window", SCRIPT)(win);
    return { win, handlers, options, reports };
}

const down = (button, isTrusted = true) => ({ type: "pointerdown", button, isTrusted });

describe("pane_click.js", () => {
    it("reports a trusted button going down, once per press, with an empty payload", () => {
        const { handlers, reports } = install();
        expect(handlers.pointerdown).toHaveLength(1);
        handlers.pointerdown[0](down(0));
        handlers.pointerdown[0](down(2));
        handlers.pointerdown[0](down(1));
        expect(reports).toEqual(["", "", ""]);
    });

    it("listens in the capture phase and never cancels the click", () => {
        const { options } = install();
        expect(options.pointerdown).toEqual({ capture: true, passive: true });
    });

    it("ignores a click the page's own script makes", () => {
        const { handlers, reports } = install();
        handlers.pointerdown[0](down(0, false));
        expect(reports).toEqual([]);
    });

    it("leaves the side buttons to the history listener", () => {
        const { handlers, reports } = install();
        handlers.pointerdown[0](down(3));
        handlers.pointerdown[0](down(4));
        handlers.pointerdown[0](down(-1));
        expect(reports).toEqual([]);
    });

    it("says nothing before the host has added the binding, and is installed once", () => {
        const { win, handlers, reports } = install({ withBinding: false });
        expect(() => handlers.pointerdown[0](down(0))).not.toThrow();
        new Function("window", SCRIPT)(win);
        expect(handlers.pointerdown).toHaveLength(1);
        expect(reports).toEqual([]);
    });
});
