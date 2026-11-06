// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent picker, pinned as a whole ahead of splitting it into a state object and view parts: each scenario mounts
 * the real component with stand-ins for the hooks that feed it (the definitions, the install checks, the templates,
 * the launch, the readiness) and for the cards, lists and panels it draws, and logs the picker as an indented tree
 * and, in order, every hook it builds and every callback its children press. The logs are snapshots recorded on the
 * unsplit component; the split must leave them byte for byte. (AgentPicker.test.tsx asserts what the person sees
 * through the RPCs; this pins the markup, the wiring and the order the hooks are built in.) The stand-in builder is
 * the one AgentBottomPanels.behavior.test.tsx uses.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, For, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { calls as T, stand } from "./agent-bottom-panels.behavior.stub";

const h = vi.hoisted(() => ({ fake: null as any }));
const log = (line: string): void => {
    T.push(line);
};

vi.mock("@/app/element/PaneLoadingCover", () => ({ PaneLoadingCover: (p: { phase: string }) => <div data-stand="PaneLoadingCover">{p.phase}</div> }));
vi.mock("@/app/errors/CopyErrorButton", () => ({
    CopyErrorButton: (p: { variant: string; className: string; report: () => string }) => (
        <div data-stand="CopyErrorButton" data-variant={p.variant} data-class={p.className}>
            {p.report()}
        </div>
    ),
}));
// The report carries the time and the build; the snapshots must not.
vi.mock("@/app/errors/error-report", () => ({ formatErrorReport: (r: { title: string; message: string }) => `${r.title}|${r.message}` }));
vi.mock("@/app/store/zoom-factor", () => ({ readZoom: (meta?: Record<string, number>) => meta?.["term:zoom"] ?? 1 }));
vi.mock("../install-sessions", () => ({ isAgentInstalling: (p: string) => p === "busy-provider" }));
vi.mock("./agent-definition-hooks", () => ({
    useAgentDefinitions: () => (log("useAgentDefinitions"), [h.fake.agents, h.fake.definitionsLoading]),
    useOpenDefinitionMap: () => ({}),
    useInstanceOpenDefinitions: () => (log("useInstanceOpenDefinitions"), [h.fake.openDefinitions, () => log("refreshOpenDefinitions"), h.fake.openLocations, (...a: unknown[]) => log(`resolveOpenDefinitions ${JSON.stringify(a)}`)]),
}));
vi.mock("./agent-picker-sort", () => ({ loadStoredSort: () => (log("loadStoredSort"), "recent"), storeSort: (s: string) => log(`storeSort ${s}`) }));
vi.mock("./agent-picker-readiness", () => ({
    usePickerReadiness: (blockId: string) => {
        log(`usePickerReadiness ${blockId}`);
        return { pickerReadiness: { get phase() { return h.fake.phase(); } }, setMyAgentsLoaded: (v: boolean) => log(`setMyAgentsLoaded ${v}`) };
    },
}));
vi.mock("./agent-picker-install", () => ({ useInstallStates: () => (log("useInstallStates"), h.fake.install) }));
vi.mock("./agent-picker-prereqs", () => ({ createPrereqs: () => (log("createPrereqs"), { prereqsFor: (a: { id: string }, fresh: boolean) => (log(`prereqsFor ${a.id} ${fresh}`), Promise.resolve([])) }) }));
vi.mock("./agent-picker-templates", () => ({
    createTemplates: (d: { clearNodejsError: () => void }) => {
        log("createTemplates");
        h.fake.templates.clearNodejsError = d.clearNodejsError;
        return h.fake.templates;
    },
}));
vi.mock("./agent-picker-launch", () => ({
    createPickerLaunch: (props: { model: { blockId: string } }, d: { forgetTemplate: (id: string) => void }) => {
        log(`createPickerLaunch ${props.model.blockId}`);
        h.fake.launch.forgetTemplate = d.forgetTemplate;
        return h.fake.launch;
    },
}));
vi.mock("./AgentPickerFilterBar", () => ({
    AgentPickerFilterBar: stand("FilterBar", { read: ["value", "sortBy"], call: { onInput: ["jo"], onClear: [], onSortChange: ["name"] } }),
}));
vi.mock("./MyAgentsList", () => ({
    MyAgentsList: (props: Record<string, any>) => {
        const Std = stand("MyAgentsList", {
            read: ["nameFilter", "sortBy", "openDefinitions", "openLocations"],
            call: { onReattach: ["a1"], onAttach: ["a1"], onFork: ["a1"], onSwitchToExisting: ["a1"], onFirstLoad: [], onViewHistory: ["a1"] },
        });
        return (
            <>
                <Std {...props} />
                <span data-stand="runtimeOf">{JSON.stringify(props.runtimeOf("a1"))}</span>
            </>
        );
    },
}));
vi.mock("./AgentCardFilters", () => ({ AgentCardFilters: stand("AgentCardFilters", { call: { onChange: [["x"]] } }) }));
vi.mock("./HiddenTemplatesSection", () => ({ HiddenTemplatesSection: stand("HiddenTemplatesSection") }));
vi.mock("./AgentCard", () => ({
    AgentCard: (props: Record<string, any>) => {
        const shown = () => JSON.stringify({ ...props, agent: props.agent.id, onLaunch: "<fn>", onContextMenu: "<fn>" });
        return (
            <div data-stand="AgentCard" data-props={shown()}>
                <span data-cb="onLaunch" onClick={() => props.onLaunch(props.agent)}>
                    launch
                </span>
                <span data-cb="onContextMenu" onClick={() => props.onContextMenu({ clientX: 3 }, props.agent)}>
                    menu
                </span>
            </div>
        );
    },
}));
vi.mock("./AgentCreatePanel", () => ({
    AgentCreatePanel: (props: Record<string, any>) => (
        <div data-stand="AgentCreatePanel" data-template={props.template.id} data-draft={JSON.stringify(props.draft)} data-installed={String(props.installed)}>
            <span data-cb="probePrereqs" onClick={() => props.probePrereqs(true)}>
                probe
            </span>
            <span data-cb="recheckInstalled" onClick={() => props.recheckInstalled()}>
                recheck
            </span>
            <span data-cb="onCreated" onClick={() => props.onCreated({ id: "new-agent" })}>
                created
            </span>
            <span data-cb="onClose" onClick={() => props.onClose()}>
                close
            </span>
            <span data-cb="onBusyChange" onClick={() => props.onBusyChange(true)}>
                busy
            </span>
        </div>
    ),
}));
vi.mock("@/app/element/ui/CardGrid", () => ({
    CardGrid: (props: Record<string, any>) => (
        <div data-stand="CardGrid" data-class={props.class} data-label={props.ariaLabel} data-min={props.minCardWidth} data-selected={String(props.selectedKey)}>
            <span data-cb="onClose" onClick={() => props.onClose()}>
                close
            </span>
            <For each={props.items}>
                {(item: any) => (
                    <div data-key={props.keyOf(item)}>
                        {props.renderCard(item)}
                        {props.selectedKey === props.keyOf(item) ? props.renderDetail(item) : null}
                    </div>
                )}
            </For>
        </div>
    ),
}));
vi.mock("@/app/element/ui", async (orig) => {
    const actual = await orig<typeof import("@/app/element/ui")>();
    return {
        ...actual,
        Menu: (props: Record<string, any>) => (
            <div data-stand="Menu" data-anchor={JSON.stringify(props.anchor)} data-label={props["aria-label"]}>
                <For each={props.items}>
                    {(it: any) => (
                        <span data-cb="select" data-item-label={it.label} data-icon={it.icon} onClick={() => it.onSelect()}>
                            {it.label}
                        </span>
                    )}
                </For>
                <span data-cb="onClose" onClick={() => props.onClose()}>
                    close
                </span>
            </div>
        ),
    };
});

import { AgentPicker } from "./AgentPicker";

const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/AgentPicker.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|style)$/;
/** The picker as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim().replace(/\s+/g, " ");
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 4; i++) await new Promise((r) => setTimeout(r, 0));
};
const agent = (id: string, over: Record<string, unknown> = {}) => ({ id, name: `Agent ${id}`, slug: id, is_seeded: 1, ...over });

/** Hooks made of signals. */
function fake(init: { agents?: any[]; loading?: boolean; zoom?: number } = {}) {
    const [agents, setAgents] = createSignal<any[]>(init.agents ?? []);
    const [definitionsLoading, setLoading] = createSignal(init.loading ?? false);
    const [phase, setPhase] = createSignal("ready");
    const [openTemplate, setOpenTemplate] = createSignal<string | null>(null);
    const [templateMenu, setTemplateMenu] = createSignal<any>(null);
    const [shown, setShown] = createSignal<any[]>(init.agents?.filter((a) => a.is_seeded) ?? []);
    const [launching, setLaunching] = createSignal<string | null>(null);
    const [busy, setBusy] = createSignal(false);
    const [launchError, setLaunchError] = createSignal<string | null>(null);
    const [nodejsError, setNodejsError] = createSignal<string | null>(null);
    const [installState, setInstallState] = createSignal<Record<string, boolean | undefined>>({ c1: true, c2: false });
    const [block, setBlock] = createSignal<any>({ meta: { "term:zoom": init.zoom ?? 1 } });
    const press = (name: string) => (...a: unknown[]) => log(`${name} ${JSON.stringify(a)}`);
    const f = {
        agents, definitionsLoading, phase,
        openDefinitions: () => ({ c1: "blk-x" }),
        openLocations: () => ({ c1: { tab: "t1" } }),
        install: {
            installState,
            driftState: () => ({ c1: "drift" }),
            providerOfAgent: new Map([["c2", "busy-provider"]]),
            recheckInstalled: (a: { id: string }) => log(`recheckInstalled ${a.id}`),
        },
        templates: {
            filterableCards: () => ["fc"],
            cardFilters: () => ["f1"],
            setCardFilters: press("setCardFilters"),
            shownTemplates: shown,
            installedKnown: () => true,
            openTemplate,
            closePanel: () => (log("closePanel"), setOpenTemplate(null)),
            handleTemplateSelect: (a: { id: string }) => (log(`handleTemplateSelect ${a.id}`), setOpenTemplate(a.id)),
            handleTemplateContextMenu: press("handleTemplateContextMenu"),
            draftFor: (a: { id: string }) => ({ draftOf: a.id }),
            setPanelBusy: press("setPanelBusy"),
            templateMenu,
            setTemplateMenu: (v: unknown) => (log(`setTemplateMenu ${JSON.stringify(v)}`), setTemplateMenu(v)),
            hideTemplate: (a: { id: string }) => log(`hideTemplate ${a.id}`),
            forgetTemplate: press("forgetTemplate"),
            clearNodejsError: undefined as undefined | (() => void),
        },
        launch: {
            launching, busy, launchError, nodejsError,
            setNodejsError: (v: unknown) => (log(`setNodejsError ${v}`), setNodejsError(v as null)),
            runtimeById: () => new Map([["a1", "runtime-a1"]]),
            handleReattach: press("handleReattach"), handleAttach: press("handleAttach"), handleFork: press("handleFork"),
            handleSwitchToExisting: press("handleSwitchToExisting"), handleViewHistory: press("handleViewHistory"),
            launchCreated: (a: { id: string }, c: { id: string }) => log(`launchCreated ${a.id} ${c.id}`),
            forgetTemplate: undefined as undefined | ((id: string) => void),
        },
        set: { setAgents, setLoading, setPhase, setShown, setLaunching, setBusy, setLaunchError, setNodejsError, setInstallState, setBlock, setOpenTemplate },
        block,
    };
    h.fake = f;
    return f;
}

function mount(f: ReturnType<typeof fake>) {
    const view = render(() => <AgentPicker model={{ blockId: "blk-1", blockAtom: f.block } as any} />);
    host = view.container;
}
const press = async (stand: string, cb: string, nth = 0) => {
    fireEvent.click(host.querySelectorAll(`[data-stand="${stand}"] [data-cb="${cb}"]`)[nth]);
    await flush();
};

beforeEach(() => {
    T.length = 0;
});
afterEach(() => {
    cleanup();
});

describe("AgentPicker, pinned", () => {
    it("builds its hooks in order and lays out the filter bar, the agents, the templates and the notes", async () => {
        const f = fake({ agents: [agent("a1", { is_seeded: 0 }), agent("c1"), agent("c2")], zoom: 1.25 });
        mount(f);
        await flush();
        look("laid out");
        await snapshot("layout");
    });

    it("says there are no definitions once they have loaded, and shows only the loading cover while they have not", async () => {
        const f = fake({ agents: [], loading: true });
        mount(f);
        await flush();
        look("loading");
        f.set.setLoading(false);
        await flush();
        look("none configured");
        f.set.setAgents([agent("c1")]);
        f.set.setShown([agent("c1")]);
        await flush();
        look("one arrives");
        await snapshot("empty");
    });

    it("hands the cards what the hooks know: launching, installed, installing, drift, expanded and the first card's focus", async () => {
        const f = fake({ agents: [agent("c1"), agent("c2"), agent("c3")] });
        mount(f);
        await flush();
        f.set.setLaunching("c2");
        f.set.setBusy(true);
        f.set.setInstallState({ c1: true, c2: false, c3: undefined });
        await flush();
        look("busy launching c2");
        await press("AgentCard", "onLaunch", 0);
        look("c1 open with its panel");
        await snapshot("cards");
    });

    it("presses every callback of the cards, the panel, the filters, the list and the grid", async () => {
        const f = fake({ agents: [agent("a1", { is_seeded: 0 }), agent("c1")] });
        mount(f);
        await flush();
        await press("AgentCard", "onContextMenu");
        await press("AgentCard", "onLaunch");
        for (const cb of ["probePrereqs", "recheckInstalled", "onCreated", "onBusyChange", "onClose"]) await press("AgentCreatePanel", cb);
        await press("AgentCardFilters", "onChange");
        for (const cb of ["onReattach", "onAttach", "onFork", "onSwitchToExisting", "onFirstLoad", "onViewHistory"]) await press("MyAgentsList", cb);
        for (const cb of ["onInput", "onClear", "onSortChange"]) await press("FilterBar", cb);
        await press("CardGrid", "onClose");
        h.fake.templates.clearNodejsError?.();
        h.fake.launch.forgetTemplate?.("c1");
        look("after the presses");
        await snapshot("callbacks");
    });

    it("shows the launch error and the Node.js notice, each with its text, and removes them", async () => {
        const f = fake({ agents: [agent("c1")] });
        mount(f);
        await flush();
        f.set.setLaunchError("The row could not be recorded");
        f.set.setNodejsError("Node 20 is needed");
        await flush();
        look("both notices");
        f.set.setLaunchError(null);
        f.set.setNodejsError(null);
        await flush();
        look("none");
        await snapshot("notices");
    });

    it("opens the template menu where the pointer was, hides the template, and closes", async () => {
        const f = fake({ agents: [agent("c1")] });
        mount(f);
        await flush();
        h.fake.templates.templateMenu();
        f.set.setLaunching(null);
        (f as any).templates.setTemplateMenu({ x: 10, y: 20, agent: agent("c1", { name: "" }) });
        await flush();
        look("menu with the slug as the name");
        await press("Menu", "select");
        (f as any).templates.setTemplateMenu({ x: 1, y: 2, agent: agent("c1", { name: "", slug: "" }) });
        await flush();
        look("menu with the id as the name");
        await press("Menu", "onClose");
        look("menu closed");
        await snapshot("menu");
    });

    it("shows the cover's phase on both branches and applies the pane's zoom", async () => {
        const f = fake({ agents: [], zoom: 2 });
        mount(f);
        await flush();
        f.set.setPhase("loading");
        await flush();
        look("empty, loading");
        f.set.setAgents([agent("c1")]);
        f.set.setBlock({ meta: { "term:zoom": 0.8 } });
        await flush();
        look("with agents, zoomed out");
        await snapshot("cover-zoom");
    });
});
