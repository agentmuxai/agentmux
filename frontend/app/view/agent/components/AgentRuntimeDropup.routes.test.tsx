// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The model panel with model routes: the groups, the greyed reasons, the Back row, and the pick that moves the agent. */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentModelPanel } from "@/types/rpc/AgentModelPanel";
import { AgentRuntimeDropup } from "./AgentRuntimeDropup";
import { backRow, compactValue, countImages, parseCompactValue, parseRouteValue, routeModelNote, routeModelOf, routeRows, routeValue } from "./runtime-dropup-routes";

const panelCommand = vi.fn();
const switchCommand = vi.fn();
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentModelPanelCommand: (...a: unknown[]) => panelCommand(...a),
        AgentModelSwitchCommand: (...a: unknown[]) => switchCommand(...a),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
const patchRuntime = vi.fn();
vi.mock("../runtime-apply", () => ({
    applyRuntimeChange: vi.fn().mockResolvedValue(undefined),
    patchRuntime: (...args: unknown[]) => patchRuntime(...args),
}));

const FLASH = "deepseek/deepseek-v4.1-flash-1017";
const PRO = "deepseek/deepseek-v4-pro-0813";
const model = (id: string, label: string, extra: Record<string, unknown> = {}) => ({
    id,
    label,
    context: 1_048_576,
    price_in: 0.15,
    price_out: 0.6,
    tools: true,
    thinking: true,
    images: true,
    default: id === FLASH,
    current: false,
    unavailable: null,
    compact_first: null,
    reread_cost_usd: null,
    ...extra,
});
const panelOf = (over: Partial<AgentModelPanel> = {}, models = [model(FLASH, "DeepSeek V4.1 Flash"), model(PRO, "DeepSeek V4 Pro")]): AgentModelPanel => ({
    harness: "claude",
    current: null,
    previous: null,
    routes: [{ id: "openrouter", label: "OpenRouter", key_present: true, key_label: "OpenRouter API key", models }],
    ...over,
});

const flush = async () => {
    for (let i = 0; i < 6; i++) await Promise.resolve();
};
const onRoute = {
    "agent:runtime": { model: "sonnet", permissionMode: "default", effort: "high" },
    "agent:route_model": { route: "openrouter", route_label: "OpenRouter", model: FLASH, label: "DeepSeek V4.1 Flash" },
};
const plain = { "agent:runtime": { model: "sonnet", permissionMode: "default", effort: "high" } };

function mount(meta: Record<string, unknown>, contextTokens?: number, conversationImages?: () => number) {
    render(() => (
        <AgentRuntimeDropup
            blockId="block-1"
            blockAtom={() => ({ meta }) as any}
            providerId="claude"
            contextTokens={contextTokens}
            conversationImages={conversationImages}
        />
    ));
}
const trigger = () => screen.getByRole("button", { name: /Runtime settings/i }) as HTMLButtonElement;
const openPanel = async () => {
    fireEvent.click(trigger());
    await flush();
};
const option = (re: RegExp) => screen.getAllByRole("option").find((o) => re.test(o.querySelector(".label")?.textContent ?? ""))!;
const groups = () => Array.from(document.querySelectorAll(".agent-runtime-dropup-group")).map((g) => g.textContent);

beforeEach(() => {
    panelCommand.mockReset();
    switchCommand.mockReset().mockResolvedValue({ changed: true, restarted: 1, failed: [] });
    patchRuntime.mockReset().mockResolvedValue({});
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("the model panel's routes", () => {
    it("shows the model that really answers in the trigger, not the harness alias", () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(onRoute);
        expect(trigger().textContent).toContain("DeepSeek V4.1 Flash");
        expect(trigger().textContent).not.toContain("Sonnet");
    });

    it("groups the harness's own models and each route's, with who pays", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        expect(groups()).toEqual(["Your planthe harness's own sign-in", "OpenRouterbilled to your key in the Vault"]);
        expect(option(/DeepSeek V4 Pro/).textContent).toContain("1M context");
        expect(option(/DeepSeek V4 Pro/).textContent).toContain("$0.15 in / $0.60 out per million");
        expect(option(/Sonnet/).getAttribute("aria-selected")).toBe("true");
        expect(panelCommand).toHaveBeenCalledWith({}, { blockid: "block-1", conversation_tokens: null, images: null });
    });

    it("marks the route model current and none of the harness's own, and says what runs", async () => {
        panelCommand.mockResolvedValue(panelOf({ current: { route: "openrouter", model: FLASH, label: "DeepSeek V4.1 Flash" } }, [model(FLASH, "DeepSeek V4.1 Flash", { current: true }), model(PRO, "DeepSeek V4 Pro")]));
        mount(onRoute);
        await openPanel();
        expect(option(/DeepSeek V4.1 Flash/).getAttribute("aria-selected")).toBe("true");
        expect(option(/Sonnet/).getAttribute("aria-selected")).toBe("false");
        expect(screen.getByText("Running: DeepSeek V4.1 Flash on OpenRouter")).toBeTruthy();
    });

    it("greys a model srv says cannot be picked, with its reason, and does not pick it", async () => {
        panelCommand.mockResolvedValue(
            panelOf({}, [model(FLASH, "DeepSeek V4.1 Flash"), model(PRO, "DeepSeek V4 Pro", { unavailable: "can't read images (3 in this conversation)" })])
        );
        mount(plain);
        await openPanel();
        const pro = option(/DeepSeek V4 Pro/);
        expect(pro.getAttribute("aria-disabled")).toBe("true");
        expect(pro.textContent).toContain("can't read images (3 in this conversation)");
        fireEvent.click(pro);
        await flush();
        expect(switchCommand).not.toHaveBeenCalled();
    });

    it("moves the agent to a route model through srv, and restarts it there", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "openrouter", model: PRO, restart: true });
        expect(patchRuntime).not.toHaveBeenCalled();
    });

    it("leaves the route first when a harness model is picked on a route, and lets the runtime change do the restart", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(onRoute);
        await openPanel();
        fireEvent.click(option(/Opus/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "", model: "", restart: false });
        expect(patchRuntime.mock.calls[0][2]).toEqual({ model: "opus" });
    });

    it("holds Back to the same checks as the model it goes back to", async () => {
        panelCommand.mockResolvedValue(
            panelOf({ previous: { route: "openrouter", model: PRO, label: "DeepSeek V4 Pro" } }, [
                model(FLASH, "DeepSeek V4.1 Flash"),
                model(PRO, "DeepSeek V4 Pro", { unavailable: "can't read images (2 in this conversation)" }),
            ])
        );
        mount(onRoute);
        await openPanel();
        const back = option(/Back to DeepSeek V4 Pro/);
        expect(back.getAttribute("aria-disabled")).toBe("true");
        expect(back.textContent).toContain("can't read images (2 in this conversation)");
        fireEvent.click(back);
        await flush();
        expect(switchCommand).not.toHaveBeenCalled();
    });

    it("scrolls the row the keys are on into view", async () => {
        const scrolled: string[] = [];
        (Element.prototype as any).scrollIntoView = function (this: HTMLElement) {
            scrolled.push(this.textContent ?? "");
        };
        try {
            panelCommand.mockResolvedValue(panelOf());
            mount(plain);
            await openPanel();
            (document.querySelector(".agent-runtime-dropup-trigger") as HTMLElement).focus();
            scrolled.length = 0;
            fireEvent.keyDown(document, { key: "ArrowDown" });
            await flush();
            expect(scrolled.length).toBe(1);
        } finally {
            delete (Element.prototype as any).scrollIntoView;
        }
    });

    it("offers Back to where the agent was, first in the Model section", async () => {
        panelCommand.mockResolvedValue(panelOf({ previous: { route: "openrouter", model: FLASH, label: "DeepSeek V4.1 Flash" } }));
        mount(plain);
        await openPanel();
        const back = option(/Back to DeepSeek V4.1 Flash/);
        fireEvent.click(back);
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "openrouter", model: FLASH, restart: true });
    });

    it("shows a failed switch on the trigger and leaves the agent where it was", async () => {
        panelCommand.mockResolvedValue(panelOf());
        switchCommand.mockRejectedValue(new Error("KEY_MISSING: route openrouter needs a Vault api-key entry"));
        mount(plain);
        await openPanel();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(trigger().title).toContain("KEY_MISSING");
    });

    it("leaves the route a first pick saved even while the running pane has not adopted it yet", async () => {
        // srv has the choice (current), but the pane's meta is still the old launch's: no route model on it.
        panelCommand.mockResolvedValue(panelOf({}, [model(FLASH, "DeepSeek V4.1 Flash"), model(PRO, "DeepSeek V4 Pro", { current: true })]));
        mount(plain);
        await openPanel();
        fireEvent.click(option(/Opus/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "", model: "", restart: false });
        expect(patchRuntime.mock.calls[0][2]).toEqual({ model: "opus" });
    });

    it("tells srv how many images the conversation holds, so a text-only model is greyed", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain, undefined, () => 3);
        await openPanel();
        expect(panelCommand).toHaveBeenCalledWith({}, { blockid: "block-1", conversation_tokens: null, images: 3 });
    });

    it("says so when a pane could not restart, so the saved pick is not taken for an applied one", async () => {
        panelCommand.mockResolvedValue(panelOf());
        switchCommand.mockResolvedValue({ changed: true, restarted: 0, failed: ["block-1: the pane is gone"] });
        mount(plain);
        await openPanel();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(trigger().title).toContain("did not switch");
        expect(trigger().title).toContain("the pane is gone");
    });

    it("is the harness's own panel when srv offers no routes", async () => {
        panelCommand.mockRejectedValue(new Error("unknown command"));
        mount(plain);
        await openPanel();
        expect(groups()).toEqual([]);
        expect(screen.getAllByRole("option").length).toBeGreaterThan(5);
    });
});

describe("the route rows' words", () => {
    it("counts the images in user messages, jekts and earlier-message notes, not other files", () => {
        const nodes = [
            { type: "user_message", attachments: [{ image: true }, { image: true }] },
            { type: "assistant_message", attachments: [{ image: true }] },
            { type: "user_message" },
            { type: "user_message", attachments: [{ image: true }, { name: "notes.txt" }] },
            { type: "jekt_message", attachments: [{ image: true }] },
            { type: "system_note", attachments: [{ image: true }, { image: true }] },
        ];
        expect(countImages(nodes)).toBe(6);
        expect(countImages([])).toBe(0);
    });

    it("round-trips a route model id that has slashes and dots", () => {
        expect(parseRouteValue(routeValue("openrouter", FLASH))).toEqual({ route: "openrouter", model: FLASH });
        expect(parseRouteValue("sonnet")).toBeNull();
        expect(parseRouteValue("back")).toBeNull();
    });

    it("reads the pane's route model, and nothing off a route", () => {
        expect(routeModelOf(onRoute)?.label).toBe("DeepSeek V4.1 Flash");
        expect(routeModelOf(plain)).toBeNull();
        expect(routeModelOf({ "agent:route_model": null })).toBeNull();
    });

    it("says what re-reading a big conversation costs, except on the model already running", () => {
        const m = model(PRO, "DeepSeek V4 Pro", { reread_cost_usd: 0.03 });
        expect(routeModelNote(m, 200_000)).toContain("about $0.030 to re-read 200K on this model");
        expect(routeModelNote(model(PRO, "x", { reread_cost_usd: 0.03, current: true }), 200_000)).not.toContain("re-read");
        expect(routeModelNote(m, null)).not.toContain("re-read");
    });

    it("offers a model too small for the conversation as Compact, then switch, and keeps one that cannot fit greyed", () => {
        const why = "the conversation (200K) is larger than this model's context (128K)";
        const small = model(PRO, "DeepSeek V4 Pro", { compact_first: why });
        const none = model(FLASH, "Tiny", { unavailable: `${why}, even compacted` });
        const rows = routeRows(panelOf({}, [small, none]), 200_000)[0].rows;
        expect(rows[0]).toMatchObject({ value: compactValue("openrouter", PRO), label: "DeepSeek V4 Pro · Compact, then switch", disabled: false });
        expect(rows[0].description).toContain("compacts it first, then switches");
        expect(rows[1]).toMatchObject({ value: routeValue("openrouter", FLASH), disabled: true });
        expect(parseCompactValue(rows[0].value)).toEqual({ route: "openrouter", model: PRO });
        expect(parseRouteValue(rows[0].value)).toBeNull();
        expect(parseCompactValue(routeValue("openrouter", PRO))).toBeNull();
    });

    it("names the harness's own model when Back goes off a route", () => {
        expect(backRow(panelOf({ previous: { route: "", model: "", label: "" } }))?.label).toBe("Back to the harness's own model");
        expect(backRow(panelOf())).toBeNull();
    });
});
