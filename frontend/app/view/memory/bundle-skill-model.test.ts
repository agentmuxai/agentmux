// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests for BundleSkillModel — the bundle-scoped Skills view model
// (composable model v2, docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md).
// Mirrors bundle-mcp-model.test.ts.

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const hub = vi.hoisted(() => ({
    handlers: new Map<string, (e: unknown) => void>(),
}));

vi.mock("@/app/store/wps", () => ({
    waveEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
        hub.handlers.set(sub.eventType, sub.handler);
        return () => hub.handlers.delete(sub.eventType);
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SkillCatalogListForBundleCommand: vi.fn(),
        SkillCatalogBindToBundleCommand: vi.fn(),
        SkillCatalogUnbindFromBundleCommand: vi.fn(),
        SkillCatalogUpsertForBundleCommand: vi.fn(),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { RpcApi } from "@/app/store/rpc-api";
import { BundleSkillModel } from "./bundle-skill-model";

const skill = (id: string, overrides: Partial<SkillBundleListItem> = {}): SkillBundleListItem => ({
    id,
    name: `skill-${id}`,
    trigger: "",
    skill_type: "prompt",
    description: "",
    content: "content",
    is_global: true,
    created_at: 0,
    updated_at: 0,
    bound_to_bundle: false,
    ...overrides,
});

let dispose: (() => void) | undefined;

beforeEach(() => {
    vi.clearAllMocks();
    hub.handlers.clear();
});

afterEach(() => {
    dispose?.();
    dispose = undefined;
});

function makeModel(bundleId = "bundle-1"): BundleSkillModel {
    let model!: BundleSkillModel;
    createRoot((d) => {
        dispose = d;
        model = new BundleSkillModel(bundleId);
    });
    return model;
}

describe("BundleSkillModel", () => {
    test("refresh loads the bundle-scoped list on construction", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([skill("s1")]);
        const model = makeModel("bundle-1");
        await model.refresh();

        expect(RpcApi.SkillCatalogListForBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1" },
        );
        expect(model.skillsAtom()).toHaveLength(1);
    });

    test("a refresh failure surfaces via errorAtom, not a thrown exception", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockRejectedValue(new Error("boom"));
        const model = makeModel();
        await model.refresh();

        expect(model.errorAtom()).toContain("boom");
        expect(model.skillsAtom()).toEqual([]);
    });

    test("bind calls SkillCatalogBindToBundleCommand with the bundle_id, then refreshes", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.SkillCatalogBindToBundleCommand).mockResolvedValue({ bound: true });
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([skill("s1", { bound_to_bundle: true })]);

        await model.bind("s1");

        expect(RpcApi.SkillCatalogBindToBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", skill_id: "s1" },
        );
        expect(model.skillsAtom()[0]?.bound_to_bundle).toBe(true);
    });

    test("unbind calls SkillCatalogUnbindFromBundleCommand with the bundle_id", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.SkillCatalogUnbindFromBundleCommand).mockResolvedValue({ unbound: true });
        const model = makeModel("bundle-1");
        await model.refresh();

        await model.unbind("s1");

        expect(RpcApi.SkillCatalogUnbindFromBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", skill_id: "s1" },
        );
    });

    test("a bind failure surfaces via errorAtom instead of throwing", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.SkillCatalogBindToBundleCommand).mockRejectedValue(new Error("FORBIDDEN"));
        const model = makeModel();
        await model.refresh();

        await model.bind("s1");

        expect(model.errorAtom()).toContain("FORBIDDEN");
    });

    test("addPrivate creates a new skill scoped to this bundle, then refreshes", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.SkillCatalogUpsertForBundleCommand).mockResolvedValue(
            skill("new-1", { is_global: false, bound_to_bundle: true }) as any,
        );
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([
            skill("new-1", { is_global: false, bound_to_bundle: true }),
        ]);

        const ok = await model.addPrivate("My Skill", "Do the thing");

        expect(ok).toBe(true);
        expect(RpcApi.SkillCatalogUpsertForBundleCommand).toHaveBeenCalledWith(
            {},
            { bundle_id: "bundle-1", name: "My Skill", content: "Do the thing" },
        );
        expect(model.addingAtom()).toBe(false);
        expect(model.skillsAtom()).toHaveLength(1);
    });

    test("addPrivate sets addingAtom while the call is in flight", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        let resolveUpsert!: (v: Skill) => void;
        vi.mocked(RpcApi.SkillCatalogUpsertForBundleCommand).mockReturnValue(
            new Promise((resolve) => { resolveUpsert = resolve; }) as any,
        );
        const model = makeModel();
        await model.refresh();

        const promise = model.addPrivate("name", "content");
        expect(model.addingAtom()).toBe(true);
        resolveUpsert(skill("x") as any);
        await promise;
        expect(model.addingAtom()).toBe(false);
    });

    test("a duplicate-name addPrivate failure surfaces via errorAtom and clears addingAtom", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        vi.mocked(RpcApi.SkillCatalogUpsertForBundleCommand).mockRejectedValue(
            new Error("skill name 'X' already bound to this bundle"),
        );
        const model = makeModel();
        await model.refresh();

        const ok = await model.addPrivate("X", "content");

        expect(ok).toBe(false);
        expect(model.errorAtom()).toContain("already bound to this bundle");
        expect(model.addingAtom()).toBe(false);
    });

    test("selectedAtom resolves the currently selected skill by id", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([skill("s1"), skill("s2")]);
        const model = makeModel();
        await model.refresh();

        model.handleSelect(model.skillsAtom()[1]!);
        expect(model.selectedAtom()?.id).toBe("s2");
    });

    test("a skills:changed event fired elsewhere triggers a refresh", async () => {
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([]);
        const model = makeModel("bundle-1");
        await model.refresh();
        vi.mocked(RpcApi.SkillCatalogListForBundleCommand).mockResolvedValue([skill("s1")]);

        hub.handlers.get("skills:changed")?.({});
        await vi.waitFor(() => expect(model.skillsAtom()).toHaveLength(1));
    });

    test("dispose unsubscribes from skills:changed", () => {
        const model = makeModel();
        expect(hub.handlers.has("skills:changed")).toBe(true);
        model.dispose();
        expect(hub.handlers.has("skills:changed")).toBe(false);
    });
});
