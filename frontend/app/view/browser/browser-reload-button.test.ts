// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { reloadButtonState } from "./browser-reload-button";

describe("reloadButtonState", () => {
    it("is Reload when idle", () => {
        expect(reloadButtonState(false)).toEqual({ action: "reload", glyph: "↻", label: "Reload" });
    });

    it("is Stop while loading, with a name that says what a press does", () => {
        expect(reloadButtonState(true)).toEqual({ action: "stop", glyph: "✕", label: "Stop loading" });
    });
});
