// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { parseEnvText } from "./agent-env";

describe("parseEnvText", () => {
    it("reads trimmed NAME=VALUE lines, skipping blanks and comments", () => {
        const parsed = parseEnvText("# tools\n\n  EDITOR=vim  \nGREETING=hello = world\nEMPTY=\n");
        expect(parsed.vars).toEqual({ EDITOR: "vim", GREETING: "hello = world", EMPTY: "" });
        expect(parsed.ignored).toEqual([]);
    });

    it("gives a name given twice its last value", () => {
        expect(parseEnvText("A=1\nA=2").vars).toEqual({ A: "2" });
    });

    it("lists the lines the launch skips, with their numbers", () => {
        const parsed = parseEnvText("OK=1\nnot a setting\n=novalue\nKEY = spaced\n1ST=digit\nA-B=dash");
        expect(parsed.vars).toEqual({ OK: "1" });
        expect(parsed.ignored.map((l) => [l.number, l.text])).toEqual([
            [2, "not a setting"],
            [3, "=novalue"],
            [4, "KEY = spaced"],
            [5, "1ST=digit"],
            [6, "A-B=dash"],
        ]);
        expect(parsed.ignored[0].reason).toContain("NAME=");
        expect(parsed.ignored[2].reason).toContain("KEY ");
    });

    it("flags reserved AGENTMUX_ names without ignoring them", () => {
        const parsed = parseEnvText("AGENTMUX_AGENT_ID=x\nagentmux_other=y\nAGENTMUX_AGENT_ID=z");
        expect(parsed.reserved).toEqual(["AGENTMUX_AGENT_ID", "agentmux_other"]);
        expect(parsed.ignored).toEqual([]);
    });

    it("treats nothing as empty and tolerates CRLF", () => {
        expect(parseEnvText(undefined)).toEqual({ vars: {}, ignored: [], reserved: [] });
        expect(parseEnvText("A=1\r\nB=2\r\n").vars).toEqual({ A: "1", B: "2" });
    });
});
