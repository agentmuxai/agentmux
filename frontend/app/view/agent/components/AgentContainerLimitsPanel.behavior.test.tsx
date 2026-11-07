// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentContainerLimitsPanel (the "Container limits…" modal), pinned as a whole ahead of splitting it into a state
 * object and views: each scenario mounts the real panel over a stand-in for srv's two limit commands whose answers the
 * scenario controls, with stand-ins for the kit's button, field, select and text input and for the caches section,
 * drives the controls, and logs the panel as an indented tree and, in order, every call it makes (each command with its
 * request, each button pressed). The limit helpers (container-limits.ts) are the real ones.
 * The logs are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { type JSX } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const T: string[] = [];
const log = (line: string): void => {
    T.push(line);
};

const h = vi.hoisted(() => ({ get: null as any, set: null as any }));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        GetContainerLimitsCommand: (_c: unknown, req: unknown) => (log(`GetContainerLimitsCommand ${JSON.stringify(req)}`), h.get()),
        SetContainerLimitsCommand: (_c: unknown, req: unknown) => (log(`SetContainerLimitsCommand ${JSON.stringify(req)}`), h.set()),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/element/ui", () => ({
    Button: (p: { tone?: string; busy?: boolean; disabled?: boolean; onClick?: () => void; children?: JSX.Element }) => (
        <span
            data-stand="Button"
            data-tone={p.tone}
            data-busy={String(!!p.busy)}
            data-disabled={String(!!p.disabled)}
            onClick={() => (log(`Button ${JSON.stringify(p.children)} click`), p.disabled || p.onClick?.())}
        >
            {p.children}
        </span>
    ),
    Field: (p: { label: string; layout?: string; children?: JSX.Element }) => (
        <div data-stand="Field" data-label={p.label} data-layout={p.layout}>
            {p.children}
        </div>
    ),
    Select: (p: any) => (
        <select
            data-stand="Select"
            data-value={p.value}
            disabled={p.disabled}
            data-testid={p["data-testid"]}
            onChange={(e) => p.onChange(e.currentTarget.value)}
        >
            {p.children}
        </select>
    ),
    TextInput: (p: any) => (
        <input
            data-stand="TextInput"
            class={p.class}
            inputmode={p.inputmode}
            data-value={p.value}
            disabled={p.disabled}
            data-testid={p["data-testid"]}
            onInput={(e) => p.onInput(e)}
        />
    ),
}));
vi.mock("./AgentContainerCaches", () => ({
    AgentContainerCaches: (p: { agentId: string; agentName: string }) => (
        <div data-stand="AgentContainerCaches" data-agent-id={p.agentId} data-agent-name={p.agentName} />
    ),
}));

import { AgentContainerLimitsPanel } from "./AgentContainerLimitsPanel";

const SNAP = (name: string) => `./__snapshots__/AgentContainerLimitsPanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|style|disabled|type|id|inputmode)$/;

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
let unmount: () => void;
const look = (label: string) => {
    log(`-- ${label}`);
    log(dump(host).join("\n") || "-");
};
const flush = async () => {
    for (let i = 0; i < 12; i++) await Promise.resolve();
};
const defer = <T,>() => {
    let resolve!: (v: T) => void;
    let reject!: (e: unknown) => void;
    const promise = new Promise<T>((res, rej) => ((resolve = res), (reject = rej)));
    return { promise, resolve, reject };
};
const pick = (testid: string, value: string) => {
    const el = host.querySelector<HTMLSelectElement>(`[data-testid='${testid}']`)!;
    el.value = value;
    fireEvent.change(el);
};
const typeIn = (testid: string, value: string) => {
    const el = host.querySelector<HTMLInputElement>(`[data-testid='${testid}']`)!;
    el.value = value;
    fireEvent.input(el);
};
const press = (label: string) => {
    const found = [...host.querySelectorAll<HTMLElement>("[data-stand='Button']")].find((b) => b.textContent?.trim() === label)!;
    fireEvent.click(found);
};

const LIMITS = {
    memory_mb: "4096",
    cpus: "2",
    effective_memory_mb: 4096,
    effective_cpus: 2,
    daemon_memory_mb: 8192,
    daemon_cpus: 8,
};

function mount(props: { onClose?: () => void } = {}) {
    const view = render(() => <AgentContainerLimitsPanel agentId="agent-1" agentName="Claude" {...props} />, {
        container: document.body.appendChild(document.createElement("div")),
    });
    host = view.container;
    unmount = view.unmount;
}

beforeEach(() => {
    T.length = 0;
    h.get = async () => LIMITS;
    h.set = async () => LIMITS;
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
});

describe("AgentContainerLimitsPanel, pinned", () => {
    it("asks srv for the agent's limits on mount with the controls off until they arrive, then shows each stored value as its mode and amount (a limit in GB and CPUs, none, and automatic) with what is in force", async () => {
        const out = defer<any>();
        h.get = () => out.promise;
        mount();
        look("loading");
        out.resolve(LIMITS);
        await flush();
        look("a set limit");
        unmount();
        document.body.innerHTML = "";
        h.get = async () => ({ memory_mb: "0", cpus: "0" });
        mount();
        await flush();
        look("no limit");
        unmount();
        document.body.innerHTML = "";
        h.get = async () => ({ memory_mb: "", cpus: "", daemon_memory_mb: 2048, daemon_cpus: 4 });
        mount();
        await flush();
        look("automatic, a small Docker");
        return snapshot("load");
    });

    it("shows a failed load as an error (an error's message or plain text) with Retry, and Retry loads again and drops the error", async () => {
        h.get = async () => {
            throw new Error("srv is down");
        };
        mount();
        await flush();
        look("load failed");
        h.get = async () => {
            throw "plain refusal";
        };
        press("Retry");
        await flush();
        look("a plain refusal");
        h.get = async () => LIMITS;
        press("Retry");
        await flush();
        look("loaded");
        return snapshot("retry");
    });

    it("edits the drafts: another mode, an amount that is not a number or not above 0 (a problem and Save off), an amount over what Docker has (a note), and no change (Save off)", async () => {
        mount();
        await flush();
        look("loaded, Save off");
        typeIn("container-limits-memory-amount", "16");
        look("memory over Docker");
        typeIn("container-limits-memory-amount", "abc");
        look("not a number");
        typeIn("container-limits-memory-amount", "0");
        look("zero");
        typeIn("container-limits-memory-amount", "6");
        typeIn("container-limits-cpus-amount", "12");
        look("both changed, CPUs over Docker");
        pick("container-limits-cpus-mode", "none");
        look("CPUs no limit");
        pick("container-limits-memory-mode", "auto");
        pick("container-limits-cpus-mode", "auto");
        look("both automatic");
        pick("container-limits-memory-mode", "custom");
        typeIn("container-limits-memory-amount", "4");
        pick("container-limits-cpus-mode", "custom");
        typeIn("container-limits-cpus-amount", "2");
        look("back to what is stored");
        return snapshot("edit");
    });

    it("saves the drafts: busy with the controls off while the command is out, then the answer shown with a note that depends on whether the running container took it, an edit dropping the note, and a failure shown and dropped by the next save", async () => {
        mount();
        await flush();
        typeIn("container-limits-memory-amount", "6");
        pick("container-limits-cpus-mode", "none");
        const out = defer<any>();
        h.set = () => out.promise;
        press("Save");
        look("saving");
        press("Save");
        out.resolve({ ...LIMITS, memory_mb: "6144", cpus: "0", applied: true });
        await flush();
        look("applied");
        typeIn("container-limits-memory-amount", "5");
        look("edited after saving");
        h.set = async () => ({ ...LIMITS, memory_mb: "5120", cpus: "0", applied: false });
        press("Save");
        await flush();
        look("not applied");
        typeIn("container-limits-memory-amount", "3");
        h.set = async () => ({ ...LIMITS, memory_mb: "3072", cpus: "0" });
        press("Save");
        await flush();
        look("no container yet");
        typeIn("container-limits-memory-amount", "2");
        h.set = async () => {
            throw new Error("docker refused");
        };
        press("Save");
        await flush();
        look("save failed");
        typeIn("container-limits-memory-amount", "1");
        h.set = async () => {
            throw "plain refusal";
        };
        press("Save");
        await flush();
        look("a plain refusal");
        h.set = async () => ({ ...LIMITS, memory_mb: "1024", cpus: "0", applied: true });
        press("Save");
        await flush();
        look("saved again, the error gone");
        return snapshot("save");
    });

    it("shows a Close button only when hosted in a modal, and passes its click on", async () => {
        mount();
        await flush();
        look("no onClose");
        unmount();
        document.body.innerHTML = "";
        mount({ onClose: () => log("onClose") });
        await flush();
        press("Close");
        look("with onClose");
        return snapshot("close");
    });
});
