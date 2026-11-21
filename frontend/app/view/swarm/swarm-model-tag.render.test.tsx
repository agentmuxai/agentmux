// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The model tag on a Swarm row (docs/specs/SPEC_SWARM_MODEL_AND_SEAT_TAGS_2026_11_20.md §4, §6.3, §8): a line of its own
 * between the header and the summary for an agent whose model the runtime knows, in its family's colour, with the
 * words first and a hidden lead word for a screen reader; muted for a model only asked for; no line at all, and no
 * placeholder, when the model is not known.
 */

import { cleanup, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@floating-ui/dom", () => ({
    autoUpdate: vi.fn(() => vi.fn()),
    computePosition: vi.fn(() => Promise.resolve({ x: 0, y: 0 })),
    flip: vi.fn(() => ({})),
    offset: vi.fn(() => ({})),
    shift: vi.fn(() => ({})),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: { DockNodeStatusCommand: () => Promise.resolve() } }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/util/reveal-block", () => ({ revealBlock: () => {} }));
const runtime = vi.hoisted(() => ({ read: (): Record<string, unknown>[] => [] }));
vi.mock("@/app/store/agent-runtime-live", () => ({
    openAgentInActiveTab: vi.fn(),
    sharedAgentRuntime: () => ({ rows: () => runtime.read() }),
}));

import { __resetAllSlots, registerPane } from "@/app/store/agent-document-store";
import { MOS } from "@/app/store/global";
import type { MuxObjUpdate } from "@/types/rpc/MuxObjUpdate";
import { NO_BACKGROUND_TASKS } from "./swarm-background";
import type { AgentTreeNode, SwarmViewModel } from "./swarm-model";
import { AgentRow } from "./swarm-view";

afterEach(() => {
    cleanup();
    __resetAllSlots();
    runtime.read = () => [];
});

let version = 0;
function seedBlockMeta(blockId: string, meta: Record<string, unknown>): void {
    MOS.updateMuxObject({
        updatetype: "update",
        otype: "block",
        oid: blockId,
        obj: { otype: "block", oid: blockId, version: ++version, meta } as unknown as Block,
    } as MuxObjUpdate);
}

const node = (blockId: string): AgentTreeNode =>
    ({
        blockId,
        agentName: "Quill",
        agentProvider: "claude",
        activitySummary: null,
        line: { text: "No activity yet", source: "status" },
        contextTokens: null,
        agentStatus: "running",
        agentToolRows: [],
        workflowRows: [],
        shellRows: [],
        cronRows: [],
        todoRows: [],
        todosTruncated: 0,
        todosPartial: false,
        currentTool: null,
        backgroundTasks: NO_BACKGROUND_TASKS,
    }) as AgentTreeNode;

const model = {
    isAgentCollapsed: () => false,
    toggleAgentCollapsed: () => {},
    isSelected: () => false,
    toggleSelected: () => {},
} as unknown as SwarmViewModel;

function renderRow(blockId: string, meta: Record<string, unknown>, rows: Record<string, unknown>[]) {
    runtime.read = () => rows;
    registerPane(blockId);
    seedBlockMeta(blockId, { view: "agent", agentName: "Quill", ...meta });
    return render(() => (
        <div class="swarm-view" tabIndex={-1}>
            <AgentRow node={() => node(blockId)} focusedBlockId={() => null} model={model} />
        </div>
    ));
}

const tagOf = (c: HTMLElement) => c.querySelector<HTMLElement>(".swarm-tags-line .swarm-model-tag");

describe("the model tag on a Swarm row", () => {
    it("shows a known model on a line between the header and the summary, in its family's colour", () => {
        const { container } = renderRow("b-m1", { agentId: "uid-1" }, [
            { agent_id: "uid-1", provider: "claude", model_label: "Sonnet 5.5", model_source: "effective", effort: "high" },
        ]);
        const card = container.querySelector(".swarm-agent-card")!;
        const parts = Array.from(card.children).map((c) => c.className.split(" ")[0]);
        expect(parts.slice(0, 3)).toEqual(["swarm-agent-row", "swarm-tags-line", "swarm-activity-summary"]);
        const tag = tagOf(container)!;
        expect(tag.classList.contains("ui-tag--model-sonnet")).toBe(true);
        expect(tag.classList.contains("ui-tag--muted")).toBe(false);
        // The hidden lead word, then the words; the glyph is decoration.
        expect(tag.textContent).toBe("Model Sonnet 5.5");
        expect(tag.querySelector(".ui-tag-sr")!.textContent).toBe("Model ");
        expect(tag.querySelector("i")!.getAttribute("aria-hidden")).toBe("true");
        // The tooltip's text is also in the page as the tag's description.
        const described = tag.getAttribute("aria-describedby")!;
        expect(container.querySelector(`#${described}`)!.textContent).toBe("Sonnet 5.5, running now (confirmed by its CLI). Effort high.");
    });

    it("gives each family its own colour class", () => {
        const classes: string[] = [];
        for (const [i, label] of ["Fable 5.2", "Opus 5.5", "Haiku 5.5", "GPT 6", "Gemini 3 Pro", "llama3.1:8b"].entries()) {
            const { container, unmount } = renderRow(`b-f${i}`, { agentId: `uid-f${i}` }, [
                { agent_id: `uid-f${i}`, model_label: label, model_source: "effective" },
            ]);
            classes.push([...tagOf(container)!.classList].find((c) => c.startsWith("ui-tag--model-"))!);
            unmount();
        }
        expect(classes).toEqual([
            "ui-tag--model-fable",
            "ui-tag--model-opus",
            "ui-tag--model-haiku",
            "ui-tag--model-gpt",
            "ui-tag--model-gemini",
            "ui-tag--model-other",
        ]);
    });

    it("mutes a model that was only asked for, and a switch that is pending", () => {
        const asked = renderRow("b-r1", { agentId: "uid-r1" }, [{ agent_id: "uid-r1", model_label: "Sonnet 5.5", model_source: "requested" }]);
        expect(tagOf(asked.container)!.classList.contains("ui-tag--muted")).toBe(true);
        asked.unmount();
        const switching = renderRow("b-r2", { agentId: "uid-r2" }, [
            { agent_id: "uid-r2", model_label: "Sonnet 5.5", model_source: "effective", pending_switch: { to_label: "Opus 5.5" } },
        ]);
        const tag = tagOf(switching.container)!;
        expect(tag.classList.contains("ui-tag--muted")).toBe(true);
        expect(tag.querySelector("i")!.className).toContain("fa-arrow-right");
    });

    it("draws no line and no placeholder when the model is not known", () => {
        // A runtime row without the fields (a build that does not send them), one marked unknown, a row of another
        // agent, and an agent with no id in its pane's meta.
        const cases: [Record<string, unknown>, Record<string, unknown>[]][] = [
            [{ agentId: "uid-u1" }, [{ agent_id: "uid-u1", state: "running" }]],
            [{ agentId: "uid-u2" }, [{ agent_id: "uid-u2", model_label: "Sonnet 5.5", model_source: "unknown" }]],
            [{ agentId: "uid-u3" }, [{ agent_id: "someone-else", model_label: "Opus 5.5", model_source: "effective" }]],
            [{}, [{ agent_id: "uid-u4", model_label: "Opus 5.5", model_source: "effective" }]],
        ];
        for (const [i, [meta, rows]] of cases.entries()) {
            const { container, unmount } = renderRow(`b-u${i}`, meta, rows);
            expect(container.querySelector(".swarm-tags-line"), `case ${i}`).toBeNull();
            expect(container.querySelector(".swarm-model-tag"), `case ${i}`).toBeNull();
            unmount();
        }
    });

    it("follows the runtime: a model that arrives makes the line, and one that goes unknown takes it away", () => {
        const [rows, setRows] = createSignal<Record<string, unknown>[]>([]);
        runtime.read = rows;
        registerPane("b-live");
        seedBlockMeta("b-live", { view: "agent", agentId: "uid-live", agentName: "Quill" });
        const { container } = render(() => (
            <div class="swarm-view" tabIndex={-1}>
                <AgentRow node={() => node("b-live")} focusedBlockId={() => null} model={model} />
            </div>
        ));
        expect(container.querySelector(".swarm-tags-line")).toBeNull();
        setRows([{ agent_id: "uid-live", model_label: "Opus 5.5", model_source: "effective" }]);
        expect(tagOf(container)!.textContent).toBe("Model Opus 5.5");
        setRows([{ agent_id: "uid-live", model_label: "Opus 5.5", model_source: "unknown" }]);
        expect(container.querySelector(".swarm-tags-line")).toBeNull();
    });

    it("sits with the seat tag, each in its own colour", () => {
        const { container } = renderRow(
            "b-s1",
            { agentId: "uid-s1", "agent:seat": { role: "leader", qualifier: "product", lead: null, area: null } },
            [{ agent_id: "uid-s1", model_label: "Opus 5.5", model_source: "effective" }]
        );
        const seat = container.querySelector(".swarm-agent-row .swarm-seat-badge")!;
        expect(seat.classList.contains("ui-tag--seat-leader")).toBe(true);
        expect(tagOf(container)!.classList.contains("ui-tag--model-opus")).toBe(true);
    });
});
