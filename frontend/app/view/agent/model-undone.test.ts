// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";
import { MODEL_UNDONE_META_KEY, modelUndoneText, readModelUndone } from "./model-undone";

const card = {
    at: 1700000000000,
    tried: { route: "openrouter", model: "deepseek/deepseek-v4-pro-0813", label: "DeepSeek V4 Pro" },
    back_on: { route: "openrouter", model: "deepseek/deepseek-v4.1-flash-1017", label: "DeepSeek V4.1 Flash" },
    status: 400,
    reason: "API Error: 400 history refused",
};

describe("readModelUndone", () => {
    it("reads what srv writes", () => {
        const read = readModelUndone({ [MODEL_UNDONE_META_KEY]: card });
        expect(read?.tried.label).toBe("DeepSeek V4 Pro");
        expect(read?.backOn.model).toBe("deepseek/deepseek-v4.1-flash-1017");
        expect(read?.status).toBe(400);
        expect(read?.reason).toContain("history refused");
    });

    it("is null when absent, null or not the shape", () => {
        expect(readModelUndone(undefined)).toBeNull();
        expect(readModelUndone({})).toBeNull();
        expect(readModelUndone({ [MODEL_UNDONE_META_KEY]: null })).toBeNull();
        expect(readModelUndone({ [MODEL_UNDONE_META_KEY]: "x" })).toBeNull();
        expect(readModelUndone({ [MODEL_UNDONE_META_KEY]: { ...card, back_on: null } })).toBeNull();
    });

    it("names the harness's own sign-in by its label, which has no route or model", () => {
        const read = readModelUndone({
            [MODEL_UNDONE_META_KEY]: { ...card, back_on: { route: "", model: "", label: "your plan" } },
        });
        expect(read?.backOn).toEqual({ route: "", model: "", label: "your plan" });
    });
});

describe("modelUndoneText", () => {
    it("says what refused and where the agent is again", () => {
        expect(modelUndoneText(readModelUndone({ [MODEL_UNDONE_META_KEY]: card })!)).toBe(
            "DeepSeek V4 Pro refused this conversation (400); you are back on DeepSeek V4.1 Flash"
        );
    });

    it("leaves the status out when there is none", () => {
        const read = readModelUndone({ [MODEL_UNDONE_META_KEY]: { ...card, status: 0 } })!;
        expect(modelUndoneText(read)).toBe("DeepSeek V4 Pro refused this conversation; you are back on DeepSeek V4.1 Flash");
    });
});
