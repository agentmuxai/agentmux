// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentSeatPanel (the "Seat…" form), pinned as a whole ahead of splitting it into a state object and view parts: each
 * scenario mounts the real panel over a stand-in agent list whose answers the scenario controls, with stand-ins for
 * the kit's fields, selects, text inputs and buttons, for the roster import and for the seat RPC, drives the fields
 * and buttons, and logs the panel as an indented tree and, in order, every call it makes (the seat it saves, the roster
 * import it opens). The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (AgentSeatPanel.test.tsx asserts what the person sees against the real kit; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal, type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const [agents, setAgents] = createSignal<any[] | undefined>(undefined);
const [loadError, setLoadError] = createSignal<string | null>(null);
const h = vi.hoisted(() => ({
    setSeat: null as any,
    selects: [] as any[],
    roster: null as any,
}));

vi.mock("../use-agent-list", () => ({
    useAgentList: () => [() => agents(), () => loadError()],
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        AgentSetSeatCommand: (_client: unknown, req: unknown) => (
            log(`AgentSetSeatCommand ${JSON.stringify(req)}`),
            h.setSeat()
        ),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/element/ui", () => ({
    Button: (p: {
        tone?: string;
        icon?: string;
        busy?: boolean;
        disabled?: boolean;
        onClick?: () => void;
        children?: JSX.Element;
    }) => (
        <span
            data-stand="Button"
            data-tone={p.tone}
            data-icon={p.icon}
            data-busy={String(p.busy)}
            data-disabled={String(p.disabled)}
            onClick={() => {
                log(`Button ${JSON.stringify(p.children)} click`);
                if (!p.disabled) p.onClick?.();
            }}
        >
            {p.children}
        </span>
    ),
    Field: (p: { label?: string; layout?: string; hint?: string; children?: JSX.Element }) => (
        <div data-stand="Field" data-label={p.label} data-layout={p.layout} data-hint={p.hint}>
            {p.children}
        </div>
    ),
    Select: (p: {
        options: { value: string; label: string; disabled?: boolean }[];
        value?: string;
        onChange?: (v: string) => void;
    }) => {
        h.selects.push(p);
        return (
            <div data-stand="Select" data-value={p.value}>
                {p.options.map((o) => (
                    <span data-option={o.value} data-disabled={String(o.disabled ?? false)}>
                        {o.label}
                    </span>
                ))}
            </div>
        );
    },
    TextInput: (p: { value?: string; maxLength?: number; placeholder?: string; onInput?: (e: any) => void }) => (
        <input
            data-stand="TextInput"
            data-value={p.value}
            value={p.value}
            maxlength={p.maxLength}
            placeholder={p.placeholder}
            onInput={p.onInput}
        />
    ),
}));
vi.mock("./SeatRosterImport", () => ({
    SeatRosterImport: (p: { agents: { id: string; name: string }[]; onDone: () => void }) => {
        h.roster = p;
        log(`SeatRosterImport agents=${JSON.stringify(p.agents)}`);
        return <div data-stand="SeatRosterImport" />;
    },
}));
vi.mock("./AgentPrimitiveModal.scss", () => ({}));
vi.mock("./AgentSeatPanel.scss", () => ({}));

import { AgentSeatPanel, createSeatForm } from "./AgentSeatPanel";

const SNAP = (name: string) => `./__snapshots__/AgentSeatPanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|value|placeholder|maxlength)$/;

/** The panel as an indented tree: tags, classes, the attributes that carry state, and text. */
function dump(node: Node, depth = 0, out: string[] = []): string[] {
    const pad = "  ".repeat(depth);
    if (node.nodeType === Node.TEXT_NODE) {
        const t = (node.textContent ?? "").trim();
        if (t) out.push(`${pad}"${t}"`);
        return out;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return out;
    const el = node as HTMLElement;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const click = (text: string) => {
    const b = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find(
        (x) => x.textContent?.trim() === text
    );
    fireEvent.click(b!);
};
/** The Nth select shown, newest props (the panel remounts nothing, but a remount adds new ones). */
const select = (i: number) => h.selects[h.selects.length - 2 + i];
const pick = (i: number, value: string) => select(i).onChange(value);
const type = (placeholder: string, value: string) => {
    const el = host.querySelector<HTMLInputElement>(`input[placeholder='${placeholder}']`)!;
    fireEvent.input(el, { target: { value } });
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};

const def = (id: string, name: string, extra: Record<string, unknown> = {}) => ({ id, name, is_seeded: 0, ...extra });
const ME = "uid-x";
const AGENTS = (seat?: unknown) => [
    def(ME, "AgentX", { seat }),
    def("uid-c", "Camper", { updated_at: 10 }),
    def("uid-o", "Opaz", { last_active_at: 500 }),
    def("uid-a", "AgentA"),
    def("tpl-claude", "Claude Code", { is_seeded: 1 }),
];
const LEADER = { role: "leader", qualifier: "product", lead: "", area: "agent pane", set_at_ms: 5 };

const onClose = vi.fn(() => log("onClose"));

function mount(props: Partial<Parameters<typeof AgentSeatPanel>[0]> = {}) {
    h.selects.length = 0;
    const view = render(() => <AgentSeatPanel agentId={ME} agentName="AgentX" {...props} />);
    host = view.container;
    return view;
}

beforeEach(() => {
    T.length = 0;
    h.selects.length = 0;
    h.roster = null;
    setAgents(undefined);
    setLoadError(null);
    h.setSeat = async () => ({ seat: null });
    onClose.mockClear();
});
afterEach(() => cleanup());

describe("AgentSeatPanel, pinned", () => {
    it("says what it has to say before the list arrives, when the list fails, and when the pane has no saved agent", async () => {
        mount();
        look("before the list");
        setLoadError("srv is down");
        look("the list failed");
        setLoadError(null);
        setAgents([def("uid-other", "Other")]);
        look("no saved agent");
        return snapshot("states");
    });

    it("fills the form from the seat as listed, offering the operator and the other agents (newest use first, not itself, no templates, and a lead this machine can't name), and marks a seat from spawn", async () => {
        setAgents(AGENTS(LEADER));
        mount({ onClose });
        look("a leader");
        cleanup();
        T.push("-- a lead this machine can't name");
        setAgents(AGENTS({ ...LEADER, lead: "uid-elsewhere" }));
        mount();
        look("an unknown lead");
        cleanup();
        T.push("-- from spawn");
        setAgents(AGENTS({ role: "worker", set_at_ms: 0 }));
        mount();
        look("from spawn");
        cleanup();
        T.push("-- no seat");
        setAgents(AGENTS(undefined));
        mount();
        look("no seat yet");
        return snapshot("fill");
    });

    it("saves what is typed, trimmed, and says so; clears the seat; and shows busy while either is out", async () => {
        setAgents(AGENTS(undefined));
        mount();
        click("Save");
        log("-- (a role is needed first)");
        pick(0, "specialist");
        type("product", "  mobile ");
        pick(1, "uid-c");
        type("panes, layout, tabs", " the app  ");
        look("filled in");
        const out = defer<any>();
        h.setSeat = () => out.promise;
        click("Save");
        look("saving");
        click("Clear");
        out.resolve({ seat: null });
        await flush();
        look("saved");
        h.setSeat = async () => ({ seat: null });
        pick(0, "leader");
        click("Save");
        await flush();
        log("-- clearing, with a seat in the list");
        setAgents(AGENTS(LEADER));
        await flush();
        click("Clear");
        look("clearing, and busy");
        await flush();
        look("cleared");
        return snapshot("save");
    });

    it("shows srv's refusals: the text of an error, the channel-local note for a missing shared definition, and a thrown non-error; and drops them on the next try", async () => {
        setAgents(AGENTS(LEADER));
        mount();
        h.setSeat = async () => {
            throw new Error("the lead is not an agent");
        };
        click("Save");
        await flush();
        look("an error");
        h.setSeat = async () => {
            throw new Error("agent has no shared definition");
        };
        click("Save");
        await flush();
        look("no shared definition");
        h.setSeat = async () => {
            throw "plain text";
        };
        click("Save");
        await flush();
        look("a thrown string");
        h.setSeat = async () => ({ seat: null });
        click("Save");
        look("the next try");
        await flush();
        look("saved after failing");
        return snapshot("refusals");
    });

    it("fills again when the listed seat changes, but not over edits when the same seat is listed again, and a form given by the list keeps its edits across a remount", async () => {
        setAgents(AGENTS(LEADER));
        mount();
        type("product", "edited");
        setAgents(AGENTS(LEADER));
        look("the same seat listed again: the edit stays");
        setAgents(AGENTS({ ...LEADER, qualifier: "platform", area: "relay" }));
        look("the seat changed: filled again");
        cleanup();
        T.push("-- a form given by the list");
        const form = createSeatForm();
        setAgents(AGENTS(LEADER));
        mount({ form });
        type("product", "kept");
        log(`form: role=${form.role()} qualifier=${form.qualifier()} filledFrom=${form.filledFrom() !== undefined}`);
        cleanup();
        setAgents(AGENTS(LEADER));
        mount({ form });
        look("remounted with the same form");
        log(`form: qualifier=${form.qualifier()}`);
        return snapshot("refill");
    });

    it("opens the roster import with the agents it can name (not the templates), returns to the button when it is done, and closes from the Close button", async () => {
        setAgents(AGENTS(LEADER));
        mount({ onClose });
        click("Import roster…");
        look("importing");
        h.roster.onDone();
        look("done importing");
        click("Close");
        cleanup();
        T.push("-- the import without a saved agent");
        setAgents([def("uid-other", "Other")]);
        mount();
        click("Import roster…");
        look("importing, no saved agent");
        return snapshot("roster");
    });
});
