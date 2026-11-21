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

function mount(meta: Record<string, unknown>, contextTokens?: number, conversationImages?: () => number, compactThenSwitch?: (route: string, model: string) => Promise<void>) {
    render(() => (
        <AgentRuntimeDropup
            blockId="block-1"
            blockAtom={() => ({ meta }) as any}
            providerId="claude"
            contextTokens={contextTokens}
            conversationImages={conversationImages}
            compactThenSwitch={compactThenSwitch}
        />
    ));
}
const trigger = () => screen.getByRole("button", { name: /Runtime settings/i }) as HTMLButtonElement;
const openPanel = async () => {
    fireEvent.click(trigger());
    await flush();
};
const option = (re: RegExp) => screen.getAllByRole("option").find((o) => re.test(o.querySelector(".label")?.textContent ?? ""))!;
const routeRow = () => option(/^OpenRouter$/);
const openRoute = async () => {
    fireEvent.click(routeRow());
    await flush();
};
const optionLabels = () => screen.getAllByRole("option").map((o) => o.querySelector(".label")?.textContent ?? "");

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

    it("lists a route as one closed row at the very bottom, with who pays, and opens its models in place", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        expect(optionLabels().at(-1)).toBe("OpenRouter");
        expect(routeRow().textContent).toContain("billed to your key in the Vault");
        expect(routeRow().getAttribute("aria-expanded")).toBe("false");
        expect(screen.queryByText("DeepSeek V4 Pro")).toBeNull();
        await openRoute();
        expect(routeRow().getAttribute("aria-expanded")).toBe("true");
        expect(optionLabels().slice(-3)).toEqual(["OpenRouter", "DeepSeek V4.1 Flash", "DeepSeek V4 Pro"]);
        expect(option(/DeepSeek V4 Pro/).textContent).toContain("1M context");
        expect(option(/DeepSeek V4 Pro/).textContent).toContain("$0.15 in / $0.60 out per million");
        expect(option(/Sonnet/).getAttribute("aria-selected")).toBe("true");
        expect(panelCommand).toHaveBeenCalledWith({}, { blockid: "block-1", conversation_tokens: null, images: null });
        expect(switchCommand).not.toHaveBeenCalled();
    });

    it("keeps the harness's models and Effort together, and the routes after them", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        const labels = optionLabels();
        expect(labels.findLastIndex((l) => /^Opus/.test(l))).toBeLessThan(labels.indexOf("low"));
        expect(labels.indexOf("max")).toBeLessThan(labels.indexOf("OpenRouter"));
        expect(labels.at(-1)).toBe("OpenRouter");
    });

    it("marks the route model current and none of the harness's own, and says what runs", async () => {
        panelCommand.mockResolvedValue(panelOf({ current: { route: "openrouter", model: FLASH, label: "DeepSeek V4.1 Flash" } }, [model(FLASH, "DeepSeek V4.1 Flash", { current: true }), model(PRO, "DeepSeek V4 Pro")]));
        mount(onRoute);
        await openPanel();
        // Closed, the route's row says which of its models the agent is on.
        expect(routeRow().getAttribute("aria-selected")).toBe("true");
        expect(routeRow().textContent).toContain("DeepSeek V4.1 Flash");
        await openRoute();
        expect(routeRow().getAttribute("aria-selected")).toBe("false");
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
        await openRoute();
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
        await openRoute();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "openrouter", model: PRO, restart: true, fresh: false, handoff: null }, { timeout: 120000 });
        expect(patchRuntime).not.toHaveBeenCalled();
    });

    it("starts a fresh session on the model picked after the choice is turned on, and only for that change", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        expect(option(/Start a fresh session/).getAttribute("aria-selected")).toBe("false");
        expect(option(/Start a fresh session/).textContent).toContain("resumes the conversation");
        // A line above it sets the routes section apart from the Effort rows, so it does not read as one more effort level.
        expect(option(/Start a fresh session/).classList.contains("is-set-apart")).toBe(true);
        fireEvent.click(option(/Start a fresh session/));
        await flush();
        expect(option(/Start a fresh session/).getAttribute("aria-selected")).toBe("true");
        expect(switchCommand).not.toHaveBeenCalled();
        await openRoute();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(switchCommand).toHaveBeenLastCalledWith({}, { blockid: "block-1", route: "openrouter", model: PRO, restart: true, fresh: true, handoff: null }, { timeout: 120000 });
        // The choice was for that change: the next pick in the same opening resumes the conversation.
        expect(option(/Start a fresh session/).getAttribute("aria-selected")).toBe("false");
        fireEvent.click(option(/DeepSeek V4.1 Flash/));
        await flush();
        expect(switchCommand).toHaveBeenLastCalledWith({}, { blockid: "block-1", route: "openrouter", model: FLASH, restart: true, fresh: false, handoff: null }, { timeout: 120000 });
        // Closed and opened again, the choice is off.
        fireEvent.click(option(/Start a fresh session/));
        await flush();
        await openPanel();
        await openPanel();
        expect(option(/Start a fresh session/).getAttribute("aria-selected")).toBe("false");
    });

    it("does not start a fresh session when a harness model is picked from a route, and keeps the choice on", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(onRoute);
        await openPanel();
        fireEvent.click(option(/Start a fresh session/));
        await flush();
        fireEvent.click(option(/Opus/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "", model: "", restart: false, fresh: false, handoff: null }, { timeout: 120000 });
        expect(patchRuntime.mock.calls[0][2]).toEqual({ model: "opus" });
        expect(option(/Start a fresh session/).getAttribute("aria-selected")).toBe("true");
    });

    it("starts a fresh session instead of compacting first when the choice is on", async () => {
        const why = "the conversation (100K) is larger than this model's context (64K)";
        panelCommand.mockResolvedValue(panelOf({}, [model(PRO, "DeepSeek V4 Pro", { compact_first: why })]));
        const compactThenSwitch = vi.fn().mockResolvedValue(undefined);
        mount(plain, 100_000, undefined, compactThenSwitch);
        await openPanel();
        fireEvent.click(option(/Start a fresh session/));
        await flush();
        await openRoute();
        fireEvent.click(option(/Compact, then switch/));
        await flush();
        expect(compactThenSwitch).not.toHaveBeenCalled();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "openrouter", model: PRO, restart: true, fresh: true, handoff: null }, { timeout: 120000 });
    });

    it("shows srv's reason when a fresh session is refused and the model change was not made", async () => {
        panelCommand.mockResolvedValue(panelOf());
        switchCommand.mockRejectedValueOnce(new Error("FORBIDDEN: a fresh session is not built for container agents yet, so this model change was not made: compact the conversation first"));
        mount(plain);
        await openPanel();
        fireEvent.click(option(/Start a fresh session/));
        await flush();
        await openRoute();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(document.body.innerHTML).toContain("not built for container agents yet");
    });

    it("leaves the route first when a harness model is picked on a route, and lets the runtime change do the restart", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(onRoute);
        await openPanel();
        fireEvent.click(option(/Opus/));
        await flush();
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "", model: "", restart: false, fresh: false, handoff: null }, { timeout: 120000 });
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
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "openrouter", model: FLASH, restart: true, fresh: false, handoff: null }, { timeout: 120000 });
    });

    it("shows a failed switch on the trigger and leaves the agent where it was", async () => {
        panelCommand.mockResolvedValue(panelOf());
        switchCommand.mockRejectedValue(new Error("KEY_MISSING: route openrouter needs a Vault api-key entry"));
        mount(plain);
        await openPanel();
        await openRoute();
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
        expect(switchCommand).toHaveBeenCalledWith({}, { blockid: "block-1", route: "", model: "", restart: false, fresh: false, handoff: null }, { timeout: 120000 });
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
        await openRoute();
        fireEvent.click(option(/DeepSeek V4 Pro/));
        await flush();
        expect(trigger().title).toContain("did not switch");
        expect(trigger().title).toContain("the pane is gone");
    });

    it("is the harness's own panel when srv offers no routes", async () => {
        panelCommand.mockRejectedValue(new Error("unknown command"));
        mount(plain);
        await openPanel();
        expect(optionLabels()).not.toContain("OpenRouter");
        expect(screen.getAllByRole("option").length).toBeGreaterThan(5);
    });

    it("says why the other models are missing when the routes could not be read, and not when they could", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        panelCommand.mockRejectedValue(new Error("FORBIDDEN: only the AgentMux window reads the model panel (this connection isn't the AgentMux window)"));
        mount(plain);
        await openPanel();
        expect(document.body.textContent).toContain("Other models: couldn't be read: only the AgentMux window reads the model panel");
        expect(warn).toHaveBeenCalled();
        cleanup();
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        expect(document.body.textContent).not.toContain("couldn't be read");
        warn.mockRestore();
    });

    it("clears the note once a later open reads the routes", async () => {
        const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
        panelCommand.mockRejectedValueOnce(new Error("timeout waiting for response"));
        mount(plain);
        await openPanel();
        expect(document.body.textContent).toContain("couldn't be read: timeout waiting for response");
        panelCommand.mockResolvedValue(panelOf());
        fireEvent.click(trigger());
        await flush();
        await openPanel();
        expect(document.body.textContent).not.toContain("couldn't be read");
        expect(optionLabels()).toContain("OpenRouter");
        warn.mockRestore();
    });
});

describe("a route's list at the bottom of the panel", () => {
    const many = (n: number, currentAt = -1) =>
        Array.from({ length: n }, (_, i) => model(i === 0 ? FLASH : `vendor/model-${i}`, `Model ${i}`, { current: i === currentAt }));

    it("shows the route's first twelve models in the order srv lists them", async () => {
        panelCommand.mockResolvedValue(panelOf({}, many(15)));
        mount(plain);
        await openPanel();
        await openRoute();
        const labels = optionLabels();
        expect(labels.slice(labels.indexOf("OpenRouter") + 1)).toEqual(Array.from({ length: 12 }, (_, i) => `Model ${i}`));
    });

    it("still shows the model the agent is on when it is not among the twelve", async () => {
        panelCommand.mockResolvedValue(panelOf({ current: { route: "openrouter", model: "vendor/model-14", label: "Model 14" } }, many(15, 14)));
        mount({
            "agent:runtime": { model: "sonnet", permissionMode: "default", effort: "high" },
            "agent:route_model": { route: "openrouter", route_label: "OpenRouter", model: "vendor/model-14", label: "Model 14" },
        });
        await openPanel();
        expect(routeRow().textContent).toContain("Model 14");
        await openRoute();
        const labels = optionLabels();
        expect(labels.at(-1)).toBe("Model 14");
        expect(labels).not.toContain("Model 12");
        expect(option(/^Model 14$/).getAttribute("aria-selected")).toBe("true");
    });

    it("opens and closes with Enter on its row, and with Right and Left on the row or one of its models", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        (document.querySelector(".agent-runtime-dropup-trigger") as HTMLElement).focus();
        const key = async (k: string) => {
            fireEvent.keyDown(document, { key: k });
            await flush();
        };
        // Walk to the route's row, the last one: the keys stop on every row that can be picked.
        for (let i = 0; i < optionLabels().length && !routeRow().classList.contains("active"); i++) await key("ArrowDown");
        expect(routeRow().classList.contains("active")).toBe(true);
        await key("Enter");
        expect(routeRow().getAttribute("aria-expanded")).toBe("true");
        await key("Enter");
        expect(routeRow().getAttribute("aria-expanded")).toBe("false");
        await key("ArrowRight");
        expect(routeRow().getAttribute("aria-expanded")).toBe("true");
        await key("ArrowDown");
        expect(option(/DeepSeek V4.1 Flash/).classList.contains("active")).toBe(true);
        await key("ArrowLeft");
        expect(routeRow().getAttribute("aria-expanded")).toBe("false");
        expect(routeRow().classList.contains("active")).toBe(true);
        expect(switchCommand).not.toHaveBeenCalled();
    });

    it("starts closed each time the panel opens", async () => {
        panelCommand.mockResolvedValue(panelOf());
        mount(plain);
        await openPanel();
        await openRoute();
        expect(routeRow().getAttribute("aria-expanded")).toBe("true");
        fireEvent.click(trigger());
        await flush();
        await openPanel();
        expect(routeRow().getAttribute("aria-expanded")).toBe("false");
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
