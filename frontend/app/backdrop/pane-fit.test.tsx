// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** A colour a view makes for itself, fitted to the pane's backdrop. */
import { cleanup, render } from "@solidjs/testing-library";
import { afterEach, describe, expect, it } from "vitest";
import { contrastRatio } from "../block/pane-color-scheme";
import { deriveBuiltin } from "./builtins";
import type { ResolvedBackdrop } from "./meta";
import { PaneBackdropContext, useFitToPane } from "./pane-fit";

afterEach(cleanup);

const resolved = (id: string, scheme: "dark" | "light"): ResolvedBackdrop => ({ id, derived: deriveBuiltin(id, scheme)! }) as unknown as ResolvedBackdrop;

function fitted(backdrop: ResolvedBackdrop | undefined, hex: string | undefined): string | undefined {
    let out: string | undefined;
    function Probe() {
        const fit = useFitToPane();
        out = fit(hex);
        return null;
    }
    render(() => (
        <PaneBackdropContext.Provider value={() => backdrop}>
            <Probe />
        </PaneBackdropContext.Provider>
    ));
    return out;
}

describe("useFitToPane", () => {
    it("returns the colour as it was outside a pane with a backdrop", () => {
        expect(fitted(undefined, "#ad9eff")).toBe("#ad9eff");
        let bare: string | undefined;
        render(() => {
            bare = useFitToPane()("#ad9eff");
            return null;
        });
        expect(bare).toBe("#ad9eff");
    });

    it("moves a near-white name colour to read on a pale backdrop, keeping a colour that already reads", () => {
        const pale = resolved("yellow", "light");
        const moved = fitted(pale, "#ffffff")!;
        for (const layer of Object.values(pale.derived.surfaces)) expect(contrastRatio(moved, layer)).toBeGreaterThanOrEqual(4.5);
        const dark = resolved("violet", "dark");
        expect(fitted(dark, "#ffffff")).toBe("#ffffff");
    });

    it("passes a missing colour through", () => {
        expect(fitted(resolved("yellow", "light"), undefined)).toBeUndefined();
    });
});
