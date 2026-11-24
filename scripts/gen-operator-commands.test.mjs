// @vitest-environment node
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// gen-operator-commands.mjs: the window's commands are every decided command that is not open.
import { describe, expect, it } from "vitest";
import { constList, operatorCommands, render } from "./gen-operator-commands.mjs";

const commands = `
pub const ALWAYS: &[&str] = &[
    "agent.kill-tree", "setconfig", // a comment with "a quote"
];
pub const UNLESS_OPERATOR: &[&str] = &["setmeta"];
pub const LIVE_INPUT: &[&str] = &["eventsub", "controllerinput"];
pub const READ_ONLY: &[&str] = &["getmemory", "getfullconfig"];
`;
const access = `pub const OPEN: &[&str] = &["eventsub", "getfullconfig", "setconfig"];
pub const OWN_PANE: &[&str] = &["setmeta"];`;

describe("operatorCommands", () => {
    it("reads each list's strings and not a comment's", () => {
        expect(constList(commands, "ALWAYS")).toEqual(["agent.kill-tree", "setconfig"]);
    });

    it("is every decided command that is not open, sorted, the pane-checked ones included", () => {
        expect(operatorCommands(commands, access)).toEqual(["agent.kill-tree", "controllerinput", "getmemory", "setmeta"]);
    });

    it("refuses an open command that no table decides", () => {
        expect(() => operatorCommands(commands, 'pub const OPEN: &[&str] = &["nobody-decided"];')).toThrow(/nobody-decided/);
    });

    it("renders the set the app reads", () => {
        const src = render(["getmemory"]);
        expect(src).toContain("@generated");
        expect(src).toContain('    "getmemory",');
    });
});
