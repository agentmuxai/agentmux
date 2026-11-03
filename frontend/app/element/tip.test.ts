// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { tip } from "./tip";

describe("tip", () => {
    it("labels the tip as the element's description, never as its name, so it cannot replace the element's text", () => {
        expect(tip("Full name")).toEqual({ "data-hover-tip": "Full name", "aria-description": "Full name" });
    });

    it("names an element that has no text of its own when asked to", () => {
        expect(tip("Unseen", { label: true })).toEqual({ "data-hover-tip": "Unseen", "aria-label": "Unseen" });
    });

    it("gives no attributes for no text, so a label that is not set leaves no tooltip", () => {
        expect(tip(undefined)).toEqual({});
        expect(tip(null)).toEqual({});
        expect(tip("")).toEqual({});
        expect(tip("", { label: true })).toEqual({});
    });
});
