// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { computeForkSet, type ForkDefinition } from "./fork-set";

const def = (id: string, over: Partial<ForkDefinition> = {}): ForkDefinition => ({
    id,
    name: id,
    created_at: 0,
    ...over,
});

/** A def with a real fork link — branch_label always set, as ForkAgentDefinitionCommand does. */
const fork = (id: string, parent_id: string, over: Partial<ForkDefinition> = {}): ForkDefinition =>
    def(id, { parent_id, branch_label: over.branch_label ?? `${id} branch`, ...over });

describe("computeForkSet", () => {
    it("returns [] when the active definition is unknown", () => {
        expect(computeForkSet([def("a")], new Map(), "missing")).toEqual([]);
    });

    it("a lone root yields a single root entry", () => {
        const r = computeForkSet([def("root")], new Map(), "root");
        expect(r).toHaveLength(1);
        expect(r[0]).toMatchObject({ definitionId: "root", isRoot: true, isActive: true, depth: 0 });
    });

    it("collects root + forks, root first, oldest sibling first", () => {
        const defs = [
            def("root", { created_at: 1 }),
            def("f2", { parent_id: "root", created_at: 30, branch_label: "second" }),
            def("f1", { parent_id: "root", created_at: 20, branch_label: "first" }),
        ];
        const r = computeForkSet(defs, new Map(), "f1");
        expect(r.map((e) => e.definitionId)).toEqual(["root", "f1", "f2"]);
        // Active is the one we asked about; root is flagged; titles use branch_label.
        expect(r[0]).toMatchObject({ isRoot: true, isActive: false, title: "root" });
        expect(r[1]).toMatchObject({ definitionId: "f1", isActive: true, title: "first", depth: 1 });
        expect(r[2]).toMatchObject({ definitionId: "f2", isActive: false, title: "second", depth: 1 });
    });

    it("walks up from a deep fork to the lineage root", () => {
        const defs = [
            def("root"),
            fork("mid", "root"),
            fork("leaf", "mid"),
        ];
        const r = computeForkSet(defs, new Map(), "leaf");
        expect(r.map((e) => e.definitionId)).toEqual(["root", "mid", "leaf"]);
        expect(r.map((e) => e.depth)).toEqual([0, 1, 2]);
        expect(r.find((e) => e.definitionId === "leaf")?.isActive).toBe(true);
    });

    it("treats a parent_id that isn't in the set as a root (orphaned fork)", () => {
        // The parent definition was deleted; the fork becomes its own root.
        const defs = [def("orphan", { parent_id: "gone" })];
        const r = computeForkSet(defs, new Map(), "orphan");
        expect(r).toHaveLength(1);
        expect(r[0]).toMatchObject({ definitionId: "orphan", isRoot: true });
    });

    it("attaches the open blockId for currently-open forks only", () => {
        const defs = [def("root"), fork("f1", "root")];
        const open = new Map([["f1", "block-xyz"]]);
        const r = computeForkSet(defs, open, "root");
        expect(r.find((e) => e.definitionId === "root")?.blockId).toBeUndefined();
        expect(r.find((e) => e.definitionId === "f1")?.blockId).toBe("block-xyz");
    });

    it("falls back to the definition name when branch_label is blank", () => {
        const defs = [def("root", { name: "Claude Code", branch_label: "   " })];
        expect(computeForkSet(defs, new Map(), "root")[0].title).toBe("Claude Code");
    });

    it("does not loop forever on a parent_id cycle", () => {
        // a → b → a (corrupt data). Must terminate and include both once.
        const defs = [
            fork("a", "b"),
            fork("b", "a"),
        ];
        const r = computeForkSet(defs, new Map(), "a");
        const ids = r.map((e) => e.definitionId).sort();
        expect(ids).toEqual(["a", "b"]);
        // No duplicates from the cycle.
        expect(new Set(ids).size).toBe(2);
    });

    it("the active fork is always present and flagged exactly once", () => {
        const defs = [
            def("root"),
            fork("f1", "root"),
            fork("f2", "root"),
        ];
        const r = computeForkSet(defs, new Map(), "f2");
        expect(r.filter((e) => e.isActive)).toHaveLength(1);
        expect(r.find((e) => e.isActive)?.definitionId).toBe("f2");
    });

    it("does not treat two unrelated agents cloned from the same template as forks of each other", () => {
        // agentdefcreatefromtemplate stamps parent_id = template.id on every
        // clone for template-provenance, leaving branch_label empty — unlike
        // a real fork. AgentX and AgentY below share no fork relationship,
        // only the same template parent.
        const defs = [
            def("tpl-claude", { name: "Claude Code" }),
            def("agent-x", { name: "AgentX", parent_id: "tpl-claude" }),
            def("agent-y", { name: "AgentY", parent_id: "tpl-claude" }),
        ];
        const rX = computeForkSet(defs, new Map(), "agent-x");
        expect(rX.map((e) => e.definitionId)).toEqual(["agent-x"]);
        const rY = computeForkSet(defs, new Map(), "agent-y");
        expect(rY.map((e) => e.definitionId)).toEqual(["agent-y"]);
    });

    it("still walks a real fork lineage rooted at a user-owned (non-template) definition", () => {
        const defs = [
            def("tpl-claude", { name: "Claude Code" }),
            def("agent-x", { name: "AgentX", parent_id: "tpl-claude" }),
            fork("agent-x-fork", "agent-x", { name: "AgentX #2" }),
        ];
        const r = computeForkSet(defs, new Map(), "agent-x-fork");
        expect(r.map((e) => e.definitionId)).toEqual(["agent-x", "agent-x-fork"]);
        expect(r.find((e) => e.definitionId === "agent-x")?.isRoot).toBe(true);
    });

    it("keeps the lineage link for a genuine fork whose source is itself a template row", () => {
        // forkagentdefinition doesn't reject a template as source_id; the
        // resulting fork gets branch_label set like any other fork, so its
        // parent_id (the template) must still count as a real link.
        const defs = [
            def("tpl-claude", { name: "Claude Code" }),
            fork("direct-fork", "tpl-claude", { name: "Claude Code #2" }),
        ];
        const r = computeForkSet(defs, new Map(), "direct-fork");
        expect(r.map((e) => e.definitionId)).toEqual(["tpl-claude", "direct-fork"]);
        expect(r.find((e) => e.definitionId === "tpl-claude")?.isRoot).toBe(true);
    });
});
