// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { WIDGET_FRAME_SANDBOX } from "./widget-sandbox";

describe("a sandboxed widget's frame", () => {
    const flags = WIDGET_FRAME_SANDBOX.split(/\s+/);

    it("runs scripts and handles its own forms and downloads, and nothing more", () => {
        expect(flags.sort()).toEqual(["allow-downloads", "allow-forms", "allow-scripts"]);
    });

    it("can't show alert, confirm or prompt", () => {
        expect(flags).not.toContain("allow-modals");
    });

    it("can't reach the app's window, open a window or share the app's origin", () => {
        for (const flag of flags) {
            expect(flag).not.toMatch(
                /^allow-(top-navigation|popups|same-origin|pointer-lock|presentation|orientation-lock)/
            );
        }
    });
});
