// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** `term:awaiting_user` follows both of the agent's own waits, a question
 *  and a tool permission (awaiting-user.ts). */

import { afterEach, describe, expect, it, vi } from "vitest";

const writes: unknown[] = [];
vi.mock("@/app/store/services", () => ({
    ObjectService: {
        UpdateObjectMeta: async (_oref: string, meta: Record<string, unknown>) => {
            writes.push(meta["term:awaiting_user"]);
        },
    },
}));
vi.mock("@/app/store/global", () => ({ MOS: { getMuxObjectAtom: () => () => ({}) }, getSettingsKeyAtom: () => () => undefined }));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: {} }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));

import { __resetWaitingForYou, endWaitingForYou, startWaitingForYou } from "@/app/notification/waiting-for-you";
import { syncAwaitingUser } from "./awaiting-user";

afterEach(() => {
    __resetWaitingForYou();
    writes.length = 0;
});

describe("syncAwaitingUser", () => {
    it("is true while a question or a permission waits, and clears with the last", async () => {
        startWaitingForYou("b1", "permission", "Allow Bash?");
        syncAwaitingUser("b1");
        startWaitingForYou("b1", "question", "Which branch?");
        syncAwaitingUser("b1");
        endWaitingForYou("b1", "question");
        syncAwaitingUser("b1");
        endWaitingForYou("b1", "permission");
        syncAwaitingUser("b1");
        await Promise.resolve();
        expect(writes).toEqual([true, true, true, null]);
    });

    it("ignores waits that aren't the agent's own (a browser hand-off)", async () => {
        startWaitingForYou("b2", "srv:k1", "Lark needs you");
        syncAwaitingUser("b2");
        await Promise.resolve();
        expect(writes).toEqual([null]);
    });
});
