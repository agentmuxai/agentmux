// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { holdPaneContent, paneContentSettled, registerPaneMounted } from "./pane-content-holds";

describe("pane content holds", () => {
    it("is settled only once mounted and with no hold left", () => {
        expect(paneContentSettled("p1")).toBe(false);
        const unmount = registerPaneMounted("p1");
        expect(paneContentSettled("p1")).toBe(true);
        const a = holdPaneContent("p1");
        const b = holdPaneContent("p1");
        a();
        expect(paneContentSettled("p1")).toBe(false);
        b();
        expect(paneContentSettled("p1")).toBe(true);
        unmount();
        expect(paneContentSettled("p1")).toBe(false);
    });

    it("releases and unmounts idempotently", () => {
        const unmount = registerPaneMounted("p2");
        const other = registerPaneMounted("p2");
        const hold = holdPaneContent("p2");
        hold();
        hold();
        expect(paneContentSettled("p2")).toBe(true);
        unmount();
        unmount();
        expect(paneContentSettled("p2")).toBe(true);
        other();
        expect(paneContentSettled("p2")).toBe(false);
    });
});
