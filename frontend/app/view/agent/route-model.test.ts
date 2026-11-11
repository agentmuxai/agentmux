// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

import { describe, expect, it } from "vitest";

import { modelMakerLogo, readRouteModel, routeModelText } from "./route-model";

const DEEPSEEK = {
    route: "openrouter",
    route_label: "OpenRouter",
    model: "deepseek/deepseek-v4.1-flash-1017",
    label: "DeepSeek V4.1 Flash",
};

describe("readRouteModel", () => {
    it("reads what srv writes", () => {
        expect(readRouteModel({ "agent:route_model": DEEPSEEK })).toEqual({
            route: "openrouter",
            routeLabel: "OpenRouter",
            model: "deepseek/deepseek-v4.1-flash-1017",
            label: "DeepSeek V4.1 Flash",
        });
    });

    it("is null off a route: absent, null, or not the shape", () => {
        expect(readRouteModel(undefined)).toBeNull();
        expect(readRouteModel({})).toBeNull();
        expect(readRouteModel({ "agent:route_model": null })).toBeNull();
        expect(readRouteModel({ "agent:route_model": "deepseek" })).toBeNull();
        expect(readRouteModel({ "agent:route_model": { route: "openrouter" } })).toBeNull();
    });

    it("names the model by its id when srv sent no label", () => {
        const m = readRouteModel({ "agent:route_model": { route: "openrouter", model: "qwen/qwen3.8-max-0902" } });
        expect(m?.label).toBe("qwen/qwen3.8-max-0902");
        expect(m?.routeLabel).toBe("openrouter");
    });
});

describe("modelMakerLogo", () => {
    const logo = (model: string, route = "openrouter") =>
        modelMakerLogo({ route, routeLabel: route, model, label: model });

    it("is the maker named before the slash", () => {
        expect(logo("deepseek/deepseek-v4-pro-0813")).toBe("deepseek");
        expect(logo("qwen/qwen3.8-max-0902")).toBe("qwen");
    });

    it("maps a maker whose mark is filed under another name", () => {
        expect(logo("moonshotai/kimi-k3")).toBe("moonshot");
        expect(logo("z-ai/glm-5.3")).toBe("zhipu");
        expect(logo("x-ai/grok-4")).toBe("xai");
        expect(logo("meta-llama/llama-4-maverick")).toBe("meta");
    });

    it("falls back to the route when the id names no maker", () => {
        expect(logo("some-model")).toBe("openrouter");
    });
});

describe("routeModelText", () => {
    it("says the model and where it is served from", () => {
        const m = readRouteModel({ "agent:route_model": DEEPSEEK })!;
        expect(routeModelText(m)).toBe("DeepSeek V4.1 Flash via OpenRouter");
    });
});
