// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import type { AgentDefinition } from "@/app/store/rpc-api";

import { paneIconInput, samePaneIconInput } from "./pane-icon-input";

const ROUTED = {
    route: "openrouter",
    route_label: "OpenRouter",
    model: "deepseek/deepseek-v4.1-flash-1017",
    label: "DeepSeek V4.1 Flash",
};

const def = (id: string, model_vendor_base_url = "") => ({ id, model_vendor_base_url }) as unknown as AgentDefinition;

describe("paneIconInput", () => {
    it("is null before an agent has launched into the pane", () => {
        expect(paneIconInput(undefined, [])).toBeNull();
        expect(paneIconInput({ agentIcon: "robot" }, [])).toBeNull();
    });

    it("shows the harness with the model answering on a route", () => {
        expect(paneIconInput({ agentProvider: "claude", "agent:route_model": ROUTED }, [])).toEqual({
            harness: "claude",
            vendor: "deepseek",
            modelText: "DeepSeek V4.1 Flash via OpenRouter",
        });
    });

    it("follows the model switched in place while the harness stays", () => {
        const before = paneIconInput({ agentProvider: "claude", "agent:route_model": ROUTED }, []);
        const after = paneIconInput(
            {
                agentProvider: "claude",
                "agent:route_model": { ...ROUTED, model: "moonshotai/kimi-k3", label: "Kimi K3" },
            },
            []
        );
        expect(after?.harness).toBe(before?.harness);
        expect(after?.vendor).toBe("moonshot");
        expect(samePaneIconInput(before, after)).toBe(false);
    });

    it("falls back to the vendor of today off a route (null or absent)", () => {
        const defs = [def("def-1")];
        const absent = paneIconInput({ agentProvider: "claude", agentId: "def-1" }, defs);
        const nulled = paneIconInput({ agentProvider: "claude", agentId: "def-1", "agent:route_model": null }, defs);
        expect(absent?.modelText).toBeUndefined();
        expect(nulled).toEqual(absent);
        expect(absent?.vendor).toBe("anthropic");
    });

    it("is a custom endpoint when the definition has a vendor base URL", () => {
        const defs = [def("def-2", "http://localhost:1234")];
        expect(paneIconInput({ agentProvider: "claude", agentId: "def-2" }, defs)?.vendor).toBe("custom");
    });

    it("resolves a provider alias, and a quick launch agent id as the provider", () => {
        expect(paneIconInput({ agentProvider: "kimi-cli" }, [])?.harness).toBe("kimi");
        expect(paneIconInput({ agentId: "codex" }, [])?.harness).toBe("codex");
    });
});

describe("samePaneIconInput", () => {
    it("compares by value, so a meta write that changes nothing builds no new icon", () => {
        const a = paneIconInput({ agentProvider: "claude", "agent:route_model": ROUTED }, []);
        const b = paneIconInput({ agentProvider: "claude", "agent:route_model": { ...ROUTED }, other: 1 }, []);
        expect(samePaneIconInput(a, b)).toBe(true);
        expect(samePaneIconInput(a, null)).toBe(false);
        expect(samePaneIconInput(null, null)).toBe(true);
    });
});
