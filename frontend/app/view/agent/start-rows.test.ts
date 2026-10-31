// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { dropSupersededStartRows } from "./start-rows";
import type { CliNoticeNode, DocumentNode } from "./types";

const ready = (id: string): CliNoticeNode => ({ type: "cli_notice", id, kind: "container_start", provider: "claude", startState: "ready", timestamp: 1 });
const waiting = (id: string): CliNoticeNode => ({ type: "cli_notice", id, kind: "agent_ready", provider: "claude", timestamp: 1 });
const starting = (id: string): CliNoticeNode => ({ type: "cli_notice", id, kind: "container_start", provider: "claude", startState: "starting", timestamp: 1 });
const failed = (id: string): CliNoticeNode => ({ type: "cli_notice", id, kind: "container_start", provider: "claude", startState: "failed", timestamp: 1 });
const said = (id: string): DocumentNode => ({ type: "user_message", id, text: "hi", timestamp: 1 }) as unknown as DocumentNode;
const ids = (nodes: readonly DocumentNode[]) => nodes.map((n) => n.id);

describe("dropSupersededStartRows", () => {
    it("keeps only the newest Container ready and Ready pair of a quiet stretch", () => {
        const nodes = [ready("r1"), waiting("w1"), ready("r2"), waiting("w2"), ready("r3"), waiting("w3")];
        expect(ids(dropSupersededStartRows(nodes))).toEqual(["r3", "w3"]);
    });

    it("keeps a pair on each side of a message: a restart in the middle of a conversation says so", () => {
        const nodes = [ready("r1"), waiting("w1"), said("m1"), ready("r2"), waiting("w2")];
        expect(ids(dropSupersededStartRows(nodes))).toEqual(["r1", "w1", "m1", "r2", "w2"]);
    });

    it("leaves a start in progress and a failed start alone", () => {
        const nodes = [failed("f1"), starting("s1"), failed("f2"), ready("r1")];
        expect(ids(dropSupersededStartRows(nodes))).toEqual(["f1", "s1", "f2", "r1"]);
    });

    it("returns the same array when nothing is superseded", () => {
        const nodes = [ready("r1"), waiting("w1")];
        expect(dropSupersededStartRows(nodes)).toBe(nodes);
    });
});
