// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
// @vitest-environment jsdom

/** A pane's colours on a panel it opens outside itself: which pane, what the panel carries, and for how long. */

import { afterEach, describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { blockOf, carryPaneScope, OVERLAY_BLOCK_ATTR } from "./overlay-scope";
import { overlayScopeVars, registerPaneScope, releasePaneScope, scopeVars, SURFACE_TOKEN_MAP } from "./scope";

const vars = scopeVars(deriveBuiltin(BUILTINS[0].id, "dark")!);
const other = scopeVars(deriveBuiltin(BUILTINS[1].id, "light")!);

afterEach(() => {
    releasePaneScope("pane-1");
    releasePaneScope("pane-2");
    document.body.replaceChildren();
});

/** A pane's frame with a button in it. */
function pane(blockId: string): { frame: HTMLElement; inside: HTMLElement } {
    const frame = document.body.appendChild(document.createElement("div"));
    frame.dataset.blockid = blockId;
    const inside = frame.appendChild(document.createElement("div")).appendChild(document.createElement("button"));
    return { frame, inside };
}

const panel = (): HTMLElement => document.body.appendChild(document.createElement("div"));

describe("overlayScopeVars", () => {
    it("puts every token that is the pane's surface on the overlay layer, and leaves the ink alone", () => {
        const overlay = overlayScopeVars(vars);
        const surfaceTokens = Object.keys(SURFACE_TOKEN_MAP).filter((t) => SURFACE_TOKEN_MAP[t].includes("var(--surface)"));
        expect(surfaceTokens).toContain("--main-bg-color");
        for (const token of surfaceTokens) {
            expect(overlay[token], token).toContain("var(--surface-overlay)");
            expect(overlay[token], token).not.toContain("var(--surface)");
        }
        // The window's opacity still applies to a menu's surface, as it does to the pane's.
        expect(overlay["--main-bg-color"]).toContain("--window-opacity");
        for (const [name, value] of Object.entries(vars)) {
            if (!surfaceTokens.includes(name)) expect(overlay[name], name).toBe(value);
        }
        // The pane's own set is not changed.
        expect(vars["--main-bg-color"]).toContain("var(--surface)");
    });
});

describe("blockOf", () => {
    it("is the pane an element is in, and nothing for an element outside every pane", () => {
        const { frame, inside } = pane("pane-1");
        expect(blockOf(inside)).toBe("pane-1");
        expect(blockOf(frame)).toBe("pane-1");
        expect(blockOf(panel())).toBeUndefined();
    });

    it("takes a block id as it is, and has no pane for a rect, a point or nothing", () => {
        expect(blockOf("pane-9")).toBe("pane-9");
        expect(blockOf({ x: 1, y: 2, width: 0, height: 0 } as DOMRect)).toBeUndefined();
        expect(blockOf(null)).toBeUndefined();
        expect(blockOf(undefined)).toBeUndefined();
    });

    it("finds the pane through a panel that pane opened: a submenu's row, a tooltip on a menu row", () => {
        registerPaneScope("pane-1", vars);
        const menu = panel();
        const stop = carryPaneScope(menu, pane("pane-1").inside);
        const row = menu.appendChild(document.createElement("div"));
        expect(blockOf(row)).toBe("pane-1");
        stop?.();
        expect(blockOf(row)).toBeUndefined();
    });
});

describe("carryPaneScope", () => {
    it("does nothing, and says so, for a source in no pane", () => {
        const el = panel();
        expect(carryPaneScope(el, panel())).toBeUndefined();
        expect(carryPaneScope(el, undefined)).toBeUndefined();
        expect(el.getAttribute("style") ?? "").toBe("");
        expect(el.hasAttribute("data-backdrop")).toBe(false);
    });

    it("leaves a panel of a pane with no backdrop as it was", () => {
        const el = panel();
        const stop = carryPaneScope(el, pane("pane-1").inside);
        expect(stop).toBeTypeOf("function");
        expect(el.getAttribute("style") ?? "").toBe("");
        expect(el.hasAttribute("data-backdrop")).toBe(false);
        expect(el.hasAttribute(OVERLAY_BLOCK_ATTR)).toBe(false);
        stop?.();
    });

    it("puts the pane's ink and its overlay surface on the panel, with the marker the aliases hang on", () => {
        registerPaneScope("pane-1", vars);
        const el = panel();
        el.style.left = "12px";
        carryPaneScope(el, pane("pane-1").inside);
        expect(el.getAttribute("data-backdrop")).toBe("");
        expect(el.getAttribute(OVERLAY_BLOCK_ATTR)).toBe("pane-1");
        expect(el.style.getPropertyValue("--ink-text")).toBe(vars["--ink-text"]);
        expect(el.style.getPropertyValue("--surface-overlay")).toBe(vars["--surface-overlay"]);
        expect(el.style.getPropertyValue("--main-text-color")).toBe("var(--ink-text)");
        expect(el.style.getPropertyValue("--modal-bg-color")).toBe("var(--surface-overlay)");
        expect(el.style.getPropertyValue("--main-bg-color")).toContain("var(--surface-overlay)");
        expect(el.style.getPropertyValue("color-scheme")).toBe("dark");
        // What the panel had of its own stays.
        expect(el.style.left).toBe("12px");
    });

    it("follows the pane while the panel is open: a backdrop set, changed and removed", () => {
        const el = panel();
        carryPaneScope(el, pane("pane-1").inside);
        registerPaneScope("pane-1", vars);
        expect(el.style.getPropertyValue("--ink-text")).toBe(vars["--ink-text"]);
        registerPaneScope("pane-1", other);
        expect(el.style.getPropertyValue("--ink-text")).toBe(other["--ink-text"]);
        expect(el.style.getPropertyValue("color-scheme")).toBe("light");
        registerPaneScope("pane-1", undefined);
        expect(el.style.getPropertyValue("--ink-text")).toBe("");
        expect(el.style.getPropertyValue("color-scheme")).toBe("");
        expect(el.hasAttribute("data-backdrop")).toBe(false);
        expect(el.hasAttribute(OVERLAY_BLOCK_ATTR)).toBe(false);
    });

    it("takes only its own pane's colours: another pane's backdrop does not reach it", () => {
        registerPaneScope("pane-2", other);
        const el = panel();
        carryPaneScope(el, pane("pane-1").inside);
        expect(el.hasAttribute("data-backdrop")).toBe(false);
        registerPaneScope("pane-1", vars);
        expect(el.style.getPropertyValue("--ink-text")).toBe(vars["--ink-text"]);
    });

    it("takes everything off again when stopped, keeps the panel's own style, and no longer follows", () => {
        registerPaneScope("pane-1", vars);
        const el = panel();
        el.style.left = "12px";
        const stop = carryPaneScope(el, "pane-1");
        expect(el.hasAttribute("data-backdrop")).toBe(true);
        stop?.();
        expect(el.hasAttribute("data-backdrop")).toBe(false);
        expect(el.hasAttribute(OVERLAY_BLOCK_ATTR)).toBe(false);
        expect(el.style.getPropertyValue("--ink-text")).toBe("");
        expect(el.style.getPropertyValue("--main-bg-color")).toBe("");
        expect(el.style.left).toBe("12px");
        registerPaneScope("pane-1", other);
        expect(el.hasAttribute("data-backdrop")).toBe(false);
    });
});
