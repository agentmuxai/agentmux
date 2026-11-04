// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What `AgentViewModel.launchAgentDefinition` does, pinned as the ordered log of everything it asks
 * srv for (every RPC and its arguments), what it logs, what it sets on the model and what it
 * returns, for each kind of launch. A refactor of the method must leave the committed snapshots as
 * they are; regenerate with `vitest -u` only for a change meant to behave differently.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const calls: { cmd: string; data: unknown; opts?: unknown }[] = [];
const logs: string[] = [];
/** Per-command answers; a function answer may throw to fail that command. */
let answers: Record<string, unknown> = {};

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        {},
        {
            get: (_t, name: string) => (_client: unknown, data: unknown, opts?: unknown) => {
                calls.push({ cmd: name.replace(/Command$/, ""), data, ...(opts ? { opts } : {}) });
                const answer = answers[name];
                if (typeof answer === "function") return Promise.resolve((answer as (d: unknown) => unknown)(data));
                return Promise.resolve(answer);
            },
        }
    ),
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/srv-info", () => ({ srvInfo: () => ({ homeDir: "/srv/home" }) }));
vi.mock("@/app/store/global", () => ({
    atoms: { staticTabId: () => "tab-static" },
    getApi: () => ({}),
    MOS: { makeORef: (t: string, id: string) => `${t}:${id}`, getMuxObjectAtom: () => () => undefined },
    pushNotification: () => {},
}));
vi.mock("@/util/logger", () => ({
    Logger: {
        info: (_c: string, m: string) => logs.push(`info: ${m}`),
        warn: (_c: string, m: string) => logs.push(`warn: ${m}`),
        error: (_c: string, m: string) => logs.push(`error: ${m}`),
        debug: () => {},
    },
}));
vi.mock("./components/AgentPicker", () => ({ useAgentDefinitions: () => [() => []] }));
vi.mock("./components/AgentPaneIcon", () => ({ buildAgentPaneIcon: () => null }));
vi.mock("./open-trace", () => ({ markAgentOpen: (_b: string, stage: string) => logs.push(`trace: ${stage}`) }));
vi.mock("./remembered-runtime", () => ({ recallRuntime: async () => undefined }));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: async () => [{ id: "11111111-1111-4111-8111-111111111111" }],
}));
vi.mock("./composer-focus", () => ({}));
vi.mock("../../agent-model/composer-focus", () => ({
    cancelComposerFocusRequest: (b: string) => logs.push(`focus cancelled: ${b}`),
    requestComposerFocus: (b: string) => logs.push(`focus requested: ${b}`),
    focusComposer: () => {},
}));

import type { AgentDefinition } from "@/app/store/rpc-api";
import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import { AgentViewModel } from "./agent-model";
import type { LaunchOverrides } from "./launch-overrides";

const ACCOUNT = "11111111-1111-4111-8111-111111111111";

function def(over: Partial<AgentDefinition> = {}): AgentDefinition {
    return {
        id: "agent-1",
        name: "Korp",
        slug: "korp",
        provider: "claude",
        icon: "robot",
        agent_type: "host",
        working_directory: "~/.agentmux/agents/korp",
        provider_flags: "",
        is_seeded: 0,
        ...over,
    } as AgentDefinition;
}

function ctx(): PaneTabHostContext {
    return { blockId: "block-1", meta: () => ({}), setMeta: async () => {}, isFocused: () => true, visibility: () => "active" };
}

/** Runs one launch and returns everything observable about it. */
async function launch(
    agent: AgentDefinition,
    overrides?: LaunchOverrides,
    targetBlockId?: string,
    targetTabId?: string
): Promise<string> {
    let model!: AgentViewModel;
    const dispose = createRoot((d) => {
        model = new AgentViewModel(ctx());
        return d;
    });
    const ok = await model.launchAgentDefinition(agent, overrides, targetBlockId, targetTabId);
    dispose();
    // The catch-up request is stamped with the epoch time, which the zone the clock was built in moves.
    return (
        JSON.stringify({ ok, nodejsError: model.nodejsError, launchError: model.launchError, calls, logs }, null, 2).replace(
            /resumed:\d+/g,
            "resumed:<now>"
        ) + "\n"
    );
}

const snap = (name: string) => `./__snapshots__/agent-model.launch.${name}.json`;

beforeEach(() => {
    calls.length = 0;
    logs.length = 0;
    vi.useFakeTimers({ toFake: ["Date"] });
    vi.setSystemTime(new Date(2026, 10, 3, 12, 0, 0));
    answers = {
        ResolvePrereqsCommand: {
            results: [
                { tool: "node", found: true, path: "/usr/bin/node" },
                { tool: "npm", found: true, path: "/usr/bin/npm" },
            ],
        },
        ResolveCliCommand: { cli_path: "/cli/claude" },
        GetAllAgentContentCommand: [
            { content_type: "ui:color", content: "#336699" },
            { content_type: "ui:zoom", content: "1.25" },
        ],
        EnsureProviderAuthDirCommand: { path: "/shared/providers/claude" },
        WriteAgentConfigCommand: (d: { working_dir: string }) => ({ working_dir: d.working_dir }),
        CreateAgentInstanceCommand: { id: "inst-1" },
        ListAgentIdentitiesCommand: [],
    };
});
afterEach(() => vi.useRealTimers());

describe("launchAgentDefinition", () => {
    it("a plain launch of a seeded host agent with a saved colour and zoom", async () => {
        await expect(await launch(def())).toMatchFileSnapshot(snap("plain"));
    });

    it("a modal launch: instance name, account, bundle and a model pick", async () => {
        const out = await launch(def(), {
            instanceName: "Korp 2",
            agentType: "host",
            environment: "local",
            accountId: ACCOUNT,
            bundleId: "bundle-1",
            model: "opus",
        });
        await expect(out).toMatchFileSnapshot(snap("modal"));
    });

    it("a continuation: its folder, its session and its account link already in place", async () => {
        answers.ListAgentIdentitiesCommand = [{ provider: "claude", account_id: ACCOUNT }];
        const out = await launch(def(), {
            instanceName: "Korp",
            agentType: "host",
            environment: "local",
            accountId: ACCOUNT,
            bundleId: "",
            continueOfInstanceId: "inst-0",
            workDirOverride: "/srv/home/agents/korp-old",
            continueSessionId: "sess-9",
        });
        await expect(out).toMatchFileSnapshot(snap("continuation"));
    });

    it("a fork of a Claude session into another block and tab", async () => {
        const out = await launch(
            def(),
            { instanceName: "Korp fork", agentType: "host", environment: "local", accountId: "", bundleId: "", continueOfInstanceId: "inst-0", forkSession: true, continueSessionId: "sess-9" },
            "block-fork",
            "tab-fork"
        );
        await expect(out).toMatchFileSnapshot(snap("fork"));
    });

    it("a container agent: no Node.js check, no host CLI, an image in the meta", async () => {
        const out = await launch(def({ agent_type: "container", container_image: "img:1" } as Partial<AgentDefinition>), {
            instanceName: "Korp c",
            agentType: "container",
            environment: "docker",
            containerImage: "img:2",
            accountId: "",
            bundleId: "",
        });
        await expect(out).toMatchFileSnapshot(snap("container"));
    });

    it("a terminal-tier harness becomes a terminal and is not resynced", async () => {
        const out = await launch(def({ provider: "aider", name: "Ada", slug: "ada" } as Partial<AgentDefinition>));
        await expect(out).toMatchFileSnapshot(snap("terminal-tier"));
    });

    it("a vendor override, a literal ~ working directory, a user folder kept verbatim and no saved colour", async () => {
        answers.GetAllAgentContentCommand = [];
        const a = await launch(def({ model_vendor_base_url: "https://proxy.example/v1", working_directory: "/work/mine" } as Partial<AgentDefinition>));
        calls.length = 0;
        logs.length = 0;
        const b = await launch(def({ working_directory: "" }));
        await expect(a + "----\n" + b).toMatchFileSnapshot(snap("vendor-and-folders"));
    });

    it("a template-backed launch whose instance row cannot be recorded is not started", async () => {
        answers.CreateAgentInstanceCommand = () => {
            throw new Error("db down");
        };
        const out = await launch(def({ is_seeded: 1 } as Partial<AgentDefinition>));
        await expect(out).toMatchFileSnapshot(snap("template-commit-failed"));
    });

    it("a user agent whose instance row cannot be recorded still starts", async () => {
        answers.CreateAgentInstanceCommand = () => {
            throw new Error("db down");
        };
        await expect(await launch(def())).toMatchFileSnapshot(snap("user-commit-failed"));
    });

    it("failures: an unknown provider, no Node.js, no CLI, a failed config write, a failed link write", async () => {
        const out: string[] = [];
        out.push(await launch(def({ provider: "nope" } as Partial<AgentDefinition>)));
        calls.length = 0;
        logs.length = 0;
        answers.ResolvePrereqsCommand = { results: [{ tool: "node", found: false, path: null }, { tool: "npm", found: false, path: null }] };
        out.push(await launch(def()));
        calls.length = 0;
        logs.length = 0;
        answers.ResolvePrereqsCommand = { results: [{ tool: "node", found: true, path: "n" }, { tool: "npm", found: true, path: "n" }] };
        answers.ResolveCliCommand = () => {
            throw new Error("npm install failed");
        };
        out.push(await launch(def()));
        calls.length = 0;
        logs.length = 0;
        answers.ResolveCliCommand = { cli_path: "/cli/claude" };
        answers.WriteAgentConfigCommand = () => {
            throw new Error("disk full");
        };
        out.push(await launch(def()));
        calls.length = 0;
        logs.length = 0;
        answers.WriteAgentConfigCommand = (d: { working_dir: string }) => ({ working_dir: d.working_dir });
        answers.LinkAgentIdentityCommand = () => {
            throw new Error("fk failed");
        };
        out.push(await launch(def(), { instanceName: "K", agentType: "host", environment: "local", accountId: ACCOUNT, bundleId: "" }));
        await expect(out.join("----\n")).toMatchFileSnapshot(snap("failures"));
    });

    it("a terminal-tier harness cannot run in a container", async () => {
        const out = await launch(def({ provider: "aider" } as Partial<AgentDefinition>), {
            instanceName: "Ada c",
            agentType: "container",
            environment: "docker",
            accountId: "",
            bundleId: "",
        });
        expect(JSON.parse(out).ok).toBe(false);
        await expect(out).toMatchFileSnapshot(snap("terminal-in-container"));
    });
});
