// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { TITLE_GIVEN_AT_META, titleMetaFor } from "./given-title";

const given = { "frame:title": "M365 Business Basic signup", [TITLE_GIVEN_AT_META]: "https://signup.microsoft.com/start" } as MetaType;

describe("a title the opener gave a browser pane", () => {
    it("stays while the page is on the site it was given for", () => {
        expect(titleMetaFor(given, "https://signup.microsoft.com/next?step=2")).toEqual({});
        expect(titleMetaFor(given, "https://signup.microsoft.com/")).toEqual({});
    });

    it("goes, with its marker, once the pane is on another site", () => {
        expect(titleMetaFor(given, "https://agentmux.ai/")).toEqual({ "frame:title": null, [TITLE_GIVEN_AT_META]: null });
        // A different port or scheme is another site, as everywhere else.
        expect(titleMetaFor(given, "http://signup.microsoft.com/")).toEqual({ "frame:title": null, [TITLE_GIVEN_AT_META]: null });
    });

    it("is not moved by a page with no site", () => {
        expect(titleMetaFor(given, "about:blank")).toEqual({});
        expect(titleMetaFor(given, "data:text/html,error")).toEqual({});
    });

    it("leaves a title with no marker alone (a name the person chose)", () => {
        expect(titleMetaFor({ "frame:title": "Mine" } as MetaType, "https://agentmux.ai/")).toEqual({});
        expect(titleMetaFor(undefined, "https://agentmux.ai/")).toEqual({});
    });
});
