// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { buildAgentIndex } from "./indexes";
import { linkify } from "./linkify";
import { testContext } from "./test-context";
import type { AgentEntry } from "./types";

const ROSTER: AgentEntry[] = [
    { id: "loap", slug: "loap", name: "Loap" },
    { id: "loap-3", slug: "loap-3", name: "Loap #3" },
    { id: "naki", slug: "naki", name: "Naki" },
    { id: "naki-2", slug: "naki-2", name: "Naki #2" },
    { id: "agent2", slug: "agent2", name: "Agent2" },
    { id: "agent2-6", slug: "agent2-6", name: "Agent2 #6" },
    { id: "agent1", slug: "agent1", name: "Agent1" },
    { id: "agenty", slug: "agenty", name: "AgentY" },
    { id: "camper-0622h", slug: "camper", name: "Camper" },
    { id: "twin-a", slug: "twin-a", name: "Twin" },
    { id: "twin-b", slug: "twin-b", name: "Twin" },
    { id: "ada-1", slug: "ada-1", name: "Ada #1" },
    { id: "ada-2", slug: "ada-2", name: "Ada #2" },
    { id: "wren-1", slug: "wren", name: "Wren" },
    { id: "ops-x", slug: "ops-x", name: "Night Ops" },
    { id: "ops", slug: "ops", name: "Ops" },
];

const here = () => testContext({ agents: buildAgentIndex(ROSTER, { localComputer: "narko", version: 1 }) });

/** The links of `text` as `label=id`, in order. */
function mentions(text: string): string[] {
    return linkify(text, here()).flatMap((s) => (s.type === "link" && s.destination.kind === "agent" ? [`${s.label}=${s.destination.id}`] : []));
}

// [what is written, what it links to]
const CASES: [string, string[]][] = [
    // 1. A display name with a number sign is one name, whatever follows it.
    ["Loap #3 is done", ["Loap #3=loap-3"]],
    ["Naki #2 and Agent2 #6", ["Naki #2=naki-2", "Agent2 #6=agent2-6"]],
    ["Loap #3's turn", ["Loap #3=loap-3"]],
    ["Loap #3, then Naki", ["Loap #3=loap-3", "Naki=naki"]],
    ["(Loap #3)", ["Loap #3=loap-3"]],
    ["asked Loap #3.", ["Loap #3=loap-3"]],
    ["Loap and Loap #3 agree", ["Loap=loap", "Loap #3=loap-3"]],
    // The plain name with a number that no agent has is not the plain-name agent.
    ["Loap #4 is not here", []],
    ["Loap #35 and Loap #3x", ["Loap=loap"]],
    // 2. Route and id forms, and with a computer.
    ["Loap-3 and loap-3", ["Loap-3=loap-3", "loap-3=loap-3"]],
    ["Agent2-6 or naki-2", ["Agent2-6=agent2-6", "naki-2=naki-2"]],
    ["Loap #3@narko", ["Loap #3@narko=loap-3"]],
    ["AgentY@narko wrote", ["AgentY@narko=agenty"]],
    ["Loap #3@elsewhere", []],
    // 3. A name that is the start of another is not the other.
    ["Agent1 and Agent10 and Agent1-06309", ["Agent1=agent1"]],
    ["Loap-3 is loap-3's, not Loap", ["Loap-3=loap-3", "loap-3=loap-3", "Loap=loap"]],
    ["Loap-9 is nobody", []],
    ["Campers and Camperx", []],
    // 4. Inside a word, a path, an address or a branch name: not a mention (code spans are kept out by the layer above).
    ["see agents/loap-06183/notes.md", []],
    ["mail loap@example.com", []],
    ["on branch loap-3/quiet-urgent-watch", []],
    ["file Loap.md and Loap/x", []],
    ["https://example.com/Loap-3", []],
    // 5. A name that is also a word links as the agent only where it is written whole.
    ["Wren says Wren", []],
    ["Night Ops and Ops", ["Night Ops=ops-x", "Ops=ops"]],
    ["Night Opsy", []],
    // 6. Two agents answering to one name: no link, never the wrong one.
    ["Twin is here, twin-a@narko too", ["twin-a@narko=twin-a"]],
    ["Ada #1 and Ada #2 and Ada", ["Ada #1=ada-1", "Ada #2=ada-2"]],
];

describe("agent names are matched whole, longest first", () => {
    it.each(CASES)("%s", (text, expected) => {
        expect(mentions(text)).toEqual(expected);
    });
});
