// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { LATE_PROMPT_LIMIT, POWER_LABELS, powerChecklist, sitesLabel } from "./agent-site-grants-model";

describe("sitesLabel", () => {
    it("names a row by its site without the scheme", () => {
        expect(sitesLabel(["https://shell.cloud.google.com"])).toBe("shell.cloud.google.com");
    });
    it("says which frames of other sites belong to it", () => {
        expect(sitesLabel(["https://shell.cloud.google.com", "https://ssh.cloud.google.com"])).toBe(
            "shell.cloud.google.com (and ssh.cloud.google.com)"
        );
    });
    it("is empty for a row with no site", () => {
        expect(sitesLabel([])).toBe("");
    });
});

describe("powerChecklist", () => {
    it("offers one checkbox per power beyond clicks, on when the row has it", () => {
        expect(powerChecklist(["terminal"], ["clicks", "terminal"])).toEqual([{ power: "terminal", on: true }]);
        expect(powerChecklist(["terminal"], ["clicks"])).toEqual([{ power: "terminal", on: false }]);
    });
    it("has a label for every power srv offers", () => {
        for (const { power } of powerChecklist(["terminal"], [])) expect(POWER_LABELS[power]).toBeTruthy();
    });
});

describe("LATE_PROMPT_LIMIT", () => {
    it("tells the person the 2.6 second limit where the terminal is granted", () => {
        expect(LATE_PROMPT_LIMIT).toMatch(/2\.6 seconds/);
        expect(LATE_PROMPT_LIMIT).toMatch(/password or code prompt/);
    });
});
