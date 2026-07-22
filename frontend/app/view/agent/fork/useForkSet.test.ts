// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { createRoot, createSignal } from "solid-js";
import { describe, expect, it } from "vitest";
import { useForkSet } from "./useForkSet";
import type { ForkDefinition } from "./fork-set";

const def = (id: string, over: Partial<ForkDefinition> = {}): ForkDefinition => ({
    id,
    name: id,
    created_at: 0,
    ...over,
});

/** A def with a real fork link — branch_label always set, as ForkAgentDefinitionCommand does. */
const fork = (id: string, parent_id: string, over: Partial<ForkDefinition> = {}): ForkDefinition =>
    def(id, { parent_id, branch_label: over.branch_label ?? `${id} branch`, ...over });

describe("useForkSet", () => {
    it("derives the fork set from its reactive sources", () => {
        createRoot((dispose) => {
            const [defs] = createSignal<ForkDefinition[]>([
                def("root"),
                fork("f1", "root", { branch_label: "side" }),
            ]);
            const [open] = createSignal(new Map<string, string>());
            const [active] = createSignal("root");
            const forks = useForkSet({ definitions: defs, openBlockByDef: open, activeDefinitionId: active });
            expect(forks().map((e) => e.definitionId)).toEqual(["root", "f1"]);
            expect(forks().find((e) => e.isActive)?.definitionId).toBe("root");
            dispose();
        });
    });

    it("recomputes when the active definition changes", () => {
        createRoot((dispose) => {
            const [defs] = createSignal<ForkDefinition[]>([
                def("root"),
                fork("f1", "root"),
            ]);
            const [open] = createSignal(new Map<string, string>());
            const [active, setActive] = createSignal("root");
            const forks = useForkSet({ definitions: defs, openBlockByDef: open, activeDefinitionId: active });
            expect(forks().find((e) => e.isActive)?.definitionId).toBe("root");
            setActive("f1");
            expect(forks().find((e) => e.isActive)?.definitionId).toBe("f1");
            dispose();
        });
    });

    it("recomputes when the open-block map changes", () => {
        createRoot((dispose) => {
            const [defs] = createSignal<ForkDefinition[]>([def("root"), fork("f1", "root")]);
            const [open, setOpen] = createSignal<ReadonlyMap<string, string>>(new Map());
            const [active] = createSignal("root");
            const forks = useForkSet({ definitions: defs, openBlockByDef: open, activeDefinitionId: active });
            expect(forks().find((e) => e.definitionId === "f1")?.blockId).toBeUndefined();
            setOpen(new Map([["f1", "block-1"]]));
            expect(forks().find((e) => e.definitionId === "f1")?.blockId).toBe("block-1");
            dispose();
        });
    });

    it("recomputes when a new fork is added to the definition list", () => {
        createRoot((dispose) => {
            const [defs, setDefs] = createSignal<ForkDefinition[]>([def("root")]);
            const [open] = createSignal(new Map<string, string>());
            const [active] = createSignal("root");
            const forks = useForkSet({ definitions: defs, openBlockByDef: open, activeDefinitionId: active });
            expect(forks()).toHaveLength(1);
            setDefs([def("root"), fork("f1", "root")]);
            expect(forks().map((e) => e.definitionId)).toEqual(["root", "f1"]);
            dispose();
        });
    });
});
