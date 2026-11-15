// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The element a pane's backdrop properties sit on. */

import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { BUILTINS, deriveBuiltin } from "./builtins";
import { PaneScope } from "./PaneScope";
import { registerPaneScope, releasePaneScope, scopeVars } from "./scope";

afterEach(() => {
    cleanup();
    releasePaneScope("pane-1");
});

const scopeEl = (c: HTMLElement) => c.querySelector<HTMLElement>(".pane-scope")!;

describe("PaneScope", () => {
    it("wraps its children and, with no scope registered, carries no properties and no data-backdrop", () => {
        const { container } = render(() => (
            <PaneScope blockId="pane-1">
                <span id="inside">hello</span>
            </PaneScope>
        ));
        expect(scopeEl(container).querySelector("#inside")?.textContent).toBe("hello");
        expect(scopeEl(container).hasAttribute("data-backdrop")).toBe(false);
        expect(scopeEl(container).getAttribute("style") ?? "").toBe("");
    });

    it("carries the block's vars inline, and the data-backdrop marker the aliases hang on", () => {
        const vars = scopeVars(deriveBuiltin(BUILTINS[0].id, "dark")!);
        registerPaneScope("pane-1", vars);
        const { container } = render(() => <PaneScope blockId="pane-1" />);
        const el = scopeEl(container);
        expect(el.hasAttribute("data-backdrop")).toBe(true);
        expect(el.style.getPropertyValue("--surface")).toBe(vars["--surface"]);
        expect(el.style.getPropertyValue("--ink-text")).toBe(vars["--ink-text"]);
        expect(el.style.getPropertyValue("--main-text-color")).toBe("var(--ink-text)");
        expect(el.style.getPropertyValue("color-scheme")).toBe("dark");
    });
});
