// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { isBangCommand, parseBangCommand } from "./bang-command";

describe("parseBangCommand", () => {
    it.each([
        ["!ls", "ls"],
        ["! ls -la", "ls -la"],
        ["  !pwd", "pwd"],
        ["\n!git status  ", "git status"],
        ["!", ""],
        ["!   ", ""],
    ])("%j → %j", (input, expected) => {
        expect(parseBangCommand(input)).toBe(expected);
    });

    it.each(["ls", "", "   ", "/help", "run !ls", "hello!"])("%j is not a bang command", (input) => {
        expect(parseBangCommand(input)).toBeNull();
        expect(isBangCommand(input)).toBe(false);
    });

    // The highlight used an untrimmed startsWith while the send path trimmed.
    it("treats leading whitespace the same for detection and parsing", () => {
        expect(isBangCommand("  !ls")).toBe(true);
    });
});
