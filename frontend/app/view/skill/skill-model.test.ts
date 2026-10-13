// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// SkillCatalogModel — the standalone Armory "Skills" tab's model. Scoped to
// the reactive-updates wiring added by
// docs/specs/SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md; this model had no
// prior test coverage, and full CRUD coverage is out of scope here.

import { beforeEach, describe, expect, test, vi } from "vitest";

const skillCatalogListMock = vi.fn().mockResolvedValue([]);
const listAgentDefinitionsMock = vi.fn().mockResolvedValue([]);
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        SkillCatalogListCommand: (...args: unknown[]) => skillCatalogListMock(...args),
        ListAgentDefinitionsCommand: (...args: unknown[]) => listAgentDefinitionsMock(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

// Same hub pattern bundle-skill-model.test.ts / global-bundle-model.test.ts use.
const mpsHub = vi.hoisted(() => ({ handlers: new Map<string, (e: unknown) => void>() }));
vi.mock("@/app/store/mps", async () => {
    const { NO_PAYLOAD_MUX_EVENTS } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>("@/app/store/mps-events.generated");
    const mock = {
        muxEventSubscribe: vi.fn((sub: { eventType: string; handler: (e: unknown) => void }) => {
            mpsHub.handlers.set(sub.eventType, sub.handler);
            return () => mpsHub.handlers.delete(sub.eventType);
        }),
    };
    // As the real onMuxEvent: a typed subscription is a muxEventSubscribe whose handler gets the data.
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({ eventType, scope, handler: (e: any) => { if (e?.data != null) handler(e.data, e); else if (NO_PAYLOAD_MUX_EVENTS.has(eventType)) handler(null, e); } } as any);
    return { ...mock, onMuxEvent };
});

import { SkillCatalogModel } from "./skill-model";

beforeEach(() => {
    skillCatalogListMock.mockClear();
    skillCatalogListMock.mockResolvedValue([]);
    listAgentDefinitionsMock.mockClear();
    listAgentDefinitionsMock.mockResolvedValue([]);
    mpsHub.handlers.clear();
});

describe("SkillCatalogModel — reactive updates", () => {
    test("subscribes to skills:changed and refreshes on it", async () => {
        const model = new SkillCatalogModel();
        await Promise.resolve();
        skillCatalogListMock.mockClear();

        mpsHub.handlers.get("skills:changed")?.({});
        await Promise.resolve();

        expect(skillCatalogListMock).toHaveBeenCalledTimes(1);
    });

    test("an unrelated event type does not trigger a refresh", async () => {
        new SkillCatalogModel();
        await Promise.resolve();
        skillCatalogListMock.mockClear();

        mpsHub.handlers.get("mcp:changed")?.({});
        await Promise.resolve();

        expect(skillCatalogListMock).not.toHaveBeenCalled();
    });

    test("unsubscribes on dispose", async () => {
        const model = new SkillCatalogModel();
        await Promise.resolve();
        expect(mpsHub.handlers.has("skills:changed")).toBe(true);

        model.dispose();

        expect(mpsHub.handlers.has("skills:changed")).toBe(false);
    });
});
