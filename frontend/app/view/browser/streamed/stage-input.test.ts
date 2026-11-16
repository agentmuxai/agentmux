// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// @vitest-environment jsdom

import { afterEach, describe, expect, it } from "vitest";
import { attachStageInput } from "./stage-input";

let unwire: (() => void) | null = null;
afterEach(() => unwire?.());

function setup() {
    const stage = document.createElement("div");
    stage.tabIndex = 0;
    const canvas = document.createElement("canvas");
    const button = document.createElement("button");
    stage.append(canvas, button);
    document.body.append(stage);
    const sent: Record<string, unknown>[] = [];
    unwire = attachStageInput(stage, () => canvas, (m) => sent.push(m as Record<string, unknown>));
    return { stage, canvas, button, sent };
}

const key = (target: Element, type: string, init: KeyboardEventInit) => {
    const e = new KeyboardEvent(type, { bubbles: true, cancelable: true, ...init });
    target.dispatchEvent(e);
    return e;
};

describe("a streamed pane's stage", () => {
    it("sends the keys pressed on the page and keeps them from the browser", () => {
        const { stage, sent } = setup();
        const e = key(stage, "keydown", { key: "a", code: "KeyA" });
        expect(sent).toHaveLength(1);
        expect(sent[0]).toMatchObject({ t: "key", type: "down", key: "a", text: "a" });
        expect(e.defaultPrevented).toBe(true);
    });

    it("leaves a control drawn over the page (Reconnect) its own keys", () => {
        const { button, sent } = setup();
        const e = key(button, "keydown", { key: "Enter", code: "Enter" });
        expect(sent).toHaveLength(0);
        expect(e.defaultPrevented).toBe(false);
        const tab = key(button, "keydown", { key: "Tab", code: "Tab" });
        expect(tab.defaultPrevented).toBe(false);
    });

    it("sends a press on the canvas but not one on a control over it", () => {
        const { canvas, button, sent } = setup();
        const press = (el: Element) => el.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, clientX: 5, clientY: 6, button: 0, detail: 1 }));
        press(button);
        expect(sent).toHaveLength(0);
        press(canvas);
        expect(sent[0]).toMatchObject({ t: "mouse", type: "down", button: "left" });
    });
});
