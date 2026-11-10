// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it, vi } from "vitest";

const open = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/block-component-registry", () => ({ openOrFocusPaneByView: open }));
vi.mock("@/app/store/global", () => ({ getSettingsKeyAtom: () => () => undefined }));

import { openAuditAtSound } from "./open-audit";
import { parseSoundRef } from "./audit-sound";

describe("openAuditAtSound", () => {
    it("opens or switches the Audit with the sound in its meta, new at every ask", async () => {
        const sound = { blockId: "blk-korp", event: "tool" as const, tool: "Bash", at: 1234 };
        await openAuditAtSound(sound);
        const [view, create, existing] = open.mock.calls[0];
        expect(view).toBe("audit");
        expect(create.meta.view).toBe("audit");
        expect(existing).toEqual({ "audit:sound": create.meta["audit:sound"] });
        expect(parseSoundRef(existing["audit:sound"])).toMatchObject({ blockId: "blk-korp", event: "tool", at: 1234 });
    });
});
