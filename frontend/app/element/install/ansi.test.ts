// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { parseAnsi } from "./ansi";

const E = String.fromCharCode(27);

describe("parseAnsi", () => {
    it("passes plain text through untouched", () => {
        expect(parseAnsi("npm http fetch GET 200")).toEqual([{ text: "npm http fetch GET 200", cls: "" }]);
    });

    it("maps basic and bright colours plus bold to classes", () => {
        expect(parseAnsi(`${E}[31mError:${E}[0m no bottle ${E}[1;92mok${E}[22m!`)).toEqual([
            { text: "Error:", cls: "log-fg-red" },
            { text: " no bottle ", cls: "" },
            { text: "ok", cls: "log-fg-bright-green log-bold" },
            { text: "!", cls: "log-fg-bright-green" },
        ]);
    });

    it("drops non-colour escape sequences instead of printing them", () => {
        expect(parseAnsi(`${E}[2K${E}[1Gdownloading${E}[?25l`)).toEqual([{ text: "downloading", cls: "" }]);
    });

    it("treats an empty SGR as a reset", () => {
        expect(parseAnsi(`${E}[33mwarn${E}[m done`)).toEqual([
            { text: "warn", cls: "log-fg-yellow" },
            { text: " done", cls: "" },
        ]);
    });
});
