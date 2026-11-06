// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * The agent identity links panel, pinned as a whole ahead of splitting it into a state object and view parts: each
 * scenario feeds it an agent, its links and the account cache (and changes them while it is mounted) and logs the
 * panel as a tree and, in order, every call it makes (the RPC, the subscriptions, the Claude login it opens). The logs
 * are snapshots recorded on the unsplit component; the split must leave them byte for byte.
 * (agent-identity-links-panel.test.tsx asserts what the person sees; this pins the markup and the calls.)
 */

import { cleanup, fireEvent, render } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    log: [] as string[],
    links: [] as any[],
    listError: null as any,
    listGate: null as Promise<void> | null,
    bundle: (async () => undefined) as (id: string) => Promise<any>,
    accounts: [] as any[],
    accountCb: null as null | ((a: any[]) => void),
    handlers: new Map<string, () => void>(),
    setAgents: (_: any[]) => {},
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ListAllAgentIdentitiesCommand: async () => {
            h.log.push("rpc ListAllAgentIdentities");
            await h.listGate;
            if (h.listError) throw h.listError;
            return h.links;
        },
        GetBundleCommand: async (_c: unknown, d: { id: string }) => {
            h.log.push(`rpc GetBundle ${d.id}`);
            return h.bundle(d.id);
        },
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({
    onMuxEvent: (eventType: string, handler: () => void) => {
        h.log.push(`subscribe ${eventType}`);
        h.handlers.set(eventType, handler);
        return () => {
            h.log.push(`unsubscribe ${eventType}`);
            h.handlers.delete(eventType);
        };
    },
}));
vi.mock("@/app/view/agent/components/AgentPicker", async () => {
    const { createSignal } = await import("solid-js");
    const [agents, setAgents] = createSignal<any[]>([]);
    h.setAgents = setAgents;
    return { useAgentDefinitions: () => [agents, () => false] };
});
vi.mock("./identity-model", () => ({
    loadAccounts: () => h.accounts,
    subscribeAccountChanges: (cb: (a: any[]) => void) => {
        h.log.push("subscribeAccountChanges");
        h.accountCb = cb;
        return () => h.log.push("unsubscribeAccountChanges");
    },
    PROVIDER_LABELS: { github: "GitHub", anthropic: "Anthropic" },
}));
vi.mock("@/app/view/accounts/ClaudeLoginPanel", () => ({
    ClaudeLoginPanel: (props: any) => {
        h.log.push(
            `ClaudeLoginPanel opened existingAccountId=${props.existingAccountId} linkTarget=${JSON.stringify(props.linkTarget)} staleAliasProvider=${props.staleAliasProvider} scope=${props.scope}`,
        );
        return (
            <div data-login-panel="">
                <button onClick={props.onClose}>close login</button>
            </div>
        );
    },
}));

import { AgentIdentityLinksPanel } from "./agent-identity-links-panel";

const T = h.log;
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/agent-identity-links-panel.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href)$/;
/** The panel as an indented tree: tags, classes, the attributes that carry state, and text. */
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
    for (let i = 0; i < 6; i++) await new Promise((r) => setTimeout(r, 0));
};
const gate = () => {
    let open!: () => void;
    const promise = new Promise<void>((r) => (open = r));
    return { promise, open };
};

const account = (id: string, extra: Record<string, unknown> = {}) => ({
    id,
    name: id,
    provider: "github",
    kind: "pat",
    display_name: id.toUpperCase(),
    status: "valid",
    ...extra,
});
const link = (agent_id: string, account_id: string, provider: string) => ({ agent_id, account_id, provider });
const AGENTS = [
    { id: "claude-agent", name: "Claude Agent", provider: "claude" },
    { id: "codex-agent", name: "Codex Agent", provider: "codex" },
    { id: "bundle-agent", name: "Bundle Agent", provider: "", memory_id: "mem-1" },
    { id: "nameless", name: undefined, provider: "claude" },
];

async function mount(agentId: string | undefined) {
    const [id, setId] = createSignal(agentId);
    const view = render(() => <AgentIdentityLinksPanel agentId={id()} />);
    host = view.container;
    await flush();
    return { setId, view };
}
const press = async (text: string) => {
    const b = [...host.querySelectorAll("button")].find((x) => x.textContent?.trim() === text);
    if (!b) throw new Error(`no button ${text}`);
    fireEvent.click(b);
    await flush();
};

beforeEach(() => {
    T.length = 0;
    h.links = [];
    h.listError = null;
    h.listGate = null;
    h.bundle = async () => undefined;
    h.accounts = [account("acc-gh"), account("acc-claude", { provider: "claude", kind: "oauth", display_name: "Claude Work" })];
    h.accountCb = null;
    h.handlers.clear();
    h.setAgents(AGENTS);
});
afterEach(() => cleanup());

describe("Agent identity links panel: the empty states", () => {
    it("without an agent it says so, and still reads the links but listens for no agent's changes", async () => {
        await mount(undefined);
        look("no agent");
        return snapshot("no-agent");
    });

    it("says nothing about links until they have loaded, then that there are none, with Connect for a Claude agent only", async () => {
        const g = gate();
        h.listGate = g.promise;
        await mount("claude-agent");
        look("loading");
        h.listGate = null;
        g.open();
        await flush();
        look("loaded, none");
        cleanup();
        T.length = 0;
        await mount("codex-agent");
        look("a Codex agent, none");
        cleanup();
        await mount("nameless");
        look("an agent without a name");
        cleanup();
        await mount("unknown-agent");
        look("an agent that is not defined");
        return snapshot("empty");
    });
});

describe("Agent identity links panel: the table", () => {
    it("lists only this agent's links: the provider, the account, its status and the button for a Claude row", async () => {
        h.accounts = [
            account("acc-gh"),
            account("acc-claude", { provider: "claude", kind: "oauth", display_name: "Claude Work" }),
            account("acc-nodisplay", { name: "claude-oauth", provider: "claude", display_name: "" }),
            account("acc-blank", { name: "plain-name", display_name: "   ", status: "expired" }),
            account("acc-reauth", { status: "needs_reauth" }),
            account("acc-unknown", { status: "unknown" }),
            account("acc-nostatus", { status: undefined }),
            account("acc-odd", { status: "weird" }),
        ];
        h.links = [
            link("claude-agent", "acc-gh", "github"),
            link("claude-agent", "acc-claude", "claude"),
            link("claude-agent", "acc-nodisplay", "claude-code"),
            link("claude-agent", "acc-blank", "github"),
            link("claude-agent", "acc-reauth", "github"),
            link("claude-agent", "acc-unknown", "github"),
            link("claude-agent", "acc-nostatus", "github"),
            link("claude-agent", "acc-odd", "github"),
            link("claude-agent", "acc-gone", "mystery-provider"),
            link("claude-agent", "acc-gone-claude", "claude"),
            link("codex-agent", "acc-gh", "github"),
        ];
        await mount("claude-agent");
        look("claude-agent");
        cleanup();
        await mount("codex-agent");
        look("codex-agent");
        return snapshot("table");
    });

    it("offers Connect Anthropic account to a Claude agent whose links have no Claude row, but not when one has", async () => {
        h.links = [link("claude-agent", "acc-gh", "github")];
        await mount("claude-agent");
        look("only github");
        cleanup();
        h.links = [link("claude-agent", "acc-gh", "github"), link("claude-agent", "acc-claude", "claude-code")];
        await mount("claude-agent");
        look("github and a legacy claude row");
        return snapshot("connect-cta");
    });
});

describe("Agent identity links panel: the Claude login", () => {
    it("opens for the row's own account id and provider, and for a fresh connect, and closes", async () => {
        h.links = [
            link("claude-agent", "acc-claude", "claude"),
            link("claude-agent", "acc-nodisplay", "claude-code"),
            link("claude-agent", "acc-gone-claude", "claude"),
        ];
        h.accounts = [account("acc-claude", { provider: "claude", display_name: "Claude Work" }), account("acc-nodisplay", { provider: "claude" })];
        await mount("claude-agent");
        const buttons = () => [...host.querySelectorAll("tbody button")] as HTMLButtonElement[];
        log(`row buttons: ${buttons().map((b) => b.textContent).join(", ")}`);
        fireEvent.click(buttons()[0]);
        await flush();
        look("canonical row");
        await press("close login");
        fireEvent.click(buttons()[1]);
        await flush();
        await press("close login");
        fireEvent.click(buttons()[2]);
        await flush();
        look("a row whose account is not cached");
        await press("close login");
        log(`login panel present: ${!!host.querySelector("[data-login-panel]")}`);
        return snapshot("login-row");
    });

    it("Connect Anthropic account opens it with no account, linked to the agent", async () => {
        h.links = [link("claude-agent", "acc-gh", "github")];
        await mount("claude-agent");
        await press("Connect Anthropic account");
        look("opened");
        await press("close login");
        return snapshot("login-connect");
    });

    it("a panel without an agent gives the login no link target", async () => {
        // No row can be pressed without an agent; the panel is only reachable through an agent id.
        await mount(undefined);
        log(`login panel present: ${!!host.querySelector("[data-login-panel]")}`);
        return snapshot("login-no-agent");
    });
});

describe("Agent identity links panel: the provider the agent really runs", () => {
    it("asks the bound bundle when the agent has no provider, and follows its answer", async () => {
        h.bundle = async () => ({ provider: "claude" });
        await mount("bundle-agent");
        look("bundle says claude");
        cleanup();
        h.bundle = async () => ({ provider: "codex" });
        await mount("bundle-agent");
        look("bundle says codex");
        cleanup();
        h.bundle = async () => ({});
        await mount("bundle-agent");
        look("bundle says nothing");
        cleanup();
        h.bundle = async () => Promise.reject(new Error("no such bundle"));
        await mount("bundle-agent");
        look("bundle fails");
        return snapshot("provider");
    });

    it("does not ask the bundle when the agent has its own provider", async () => {
        await mount("claude-agent");
        log(`GetBundle calls: ${T.filter((l) => l.startsWith("rpc GetBundle")).length}`);
        return snapshot("provider-own");
    });
});

describe("Agent identity links panel: errors", () => {
    it("shows why the links could not be read, and no empty hint or Connect while it fails", async () => {
        h.listError = new Error("srv is busy");
        await mount("claude-agent");
        look("failed with a message");
        cleanup();
        h.listError = {};
        await mount("claude-agent");
        look("failed without one");
        cleanup();
        h.listError = new Error("srv is busy");
        h.links = [link("claude-agent", "acc-gh", "github")];
        await mount("claude-agent");
        look("failed, with links from before");
        return snapshot("errors");
    });

    it("a change event reads the links again and clears the error", async () => {
        h.listError = new Error("srv is busy");
        await mount("claude-agent");
        h.listError = null;
        h.links = [link("claude-agent", "acc-gh", "github")];
        h.handlers.get("agentidentities:changed:claude-agent")!();
        await flush();
        look("recovered");
        return snapshot("errors-recovered");
    });
});

describe("Agent identity links panel: subscriptions and live changes", () => {
    it("listens for this agent's changes, follows a new agentId, and lets go on unmount", async () => {
        h.links = [link("claude-agent", "acc-gh", "github"), link("codex-agent", "acc-gh", "github")];
        const { setId, view } = await mount("claude-agent");
        log(`-- handlers: ${JSON.stringify([...h.handlers.keys()])}`);
        setId("codex-agent");
        await flush();
        log(`-- handlers: ${JSON.stringify([...h.handlers.keys()])}`);
        look("codex-agent");
        setId(undefined);
        await flush();
        log(`-- handlers: ${JSON.stringify([...h.handlers.keys()])}`);
        setId("claude-agent");
        await flush();
        view.unmount();
        log(`-- handlers: ${JSON.stringify([...h.handlers.keys()])}`);
        return snapshot("subscriptions");
    });

    it("an account change updates the labels, and a change event reads the links again", async () => {
        h.links = [link("claude-agent", "acc-gh", "github"), link("claude-agent", "acc-new", "github")];
        await mount("claude-agent");
        look("before");
        h.accountCb!([account("acc-gh", { display_name: "Renamed", status: "expired" }), account("acc-new", { display_name: "New one" })]);
        await flush();
        look("accounts changed");
        h.links = [link("claude-agent", "acc-new", "github")];
        h.handlers.get("agentidentities:changed:claude-agent")!();
        await flush();
        look("links changed");
        return snapshot("live");
    });
});
