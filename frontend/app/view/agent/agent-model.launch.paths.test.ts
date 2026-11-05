// @vitest-environment jsdom
// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * What `AgentViewModel.launchAgentDefinition` decides that agent-model.launch.test.ts leaves
 * unpinned: which provider a launch is for when the agent has none of its own, whether a launch runs
 * in a container (the override wins over the definition), the slug and instance name, what the
 * remembered and carried-over runtime and the definition's flags do to the arguments, a Codex
 * account's own model list, and the catch-up request after a resume. Each test reads the RPCs and
 * the logs the method produced.
 */

import { createRoot } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type Call = { cmd: string; data: Record<string, unknown> };
type Meta = {
    agentName: string;
    agentMode: string;
    agentProvider: string;
    "cmd:args": string[];
    "cmd:env": Record<string, string>;
    "agent:runtime": { model: string };
    "agent:provider_flags": string;
};
const calls: Call[] = [];
const logs: string[] = [];
const recalled: { agentId: string; models: unknown }[] = [];
/** Per-command answers; a function answer may throw to fail that command. */
let answers: Record<string, unknown> = {};
let remembered: Record<string, unknown> | undefined;

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: new Proxy(
        {},
        {
            get: (_t, name: string) => (_client: unknown, data: unknown) => {
                calls.push({ cmd: name.replace(/Command$/, ""), data: data as Record<string, unknown> });
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
        info: (_c: string, m: string, d?: unknown) => logs.push(`info: ${m} ${JSON.stringify(d ?? {})}`),
        warn: (_c: string, m: string, d?: unknown) => logs.push(`warn: ${m} ${JSON.stringify(d ?? {})}`),
        error: (_c: string, m: string, d?: unknown) => logs.push(`error: ${m} ${JSON.stringify(d ?? {})}`),
        debug: () => {},
    },
}));
vi.mock("./components/AgentPicker", () => ({ useAgentDefinitions: () => [() => []] }));
vi.mock("./components/AgentPaneIcon", () => ({ buildAgentPaneIcon: () => null }));
vi.mock("./open-trace", () => ({ markAgentOpen: () => {} }));
vi.mock("./remembered-runtime", () => ({
    recallRuntime: async (agentId: string, models: unknown) => {
        recalled.push({ agentId, models });
        return remembered;
    },
}));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: async () => [{ id: "11111111-1111-4111-8111-111111111111" }],
}));
vi.mock("./composer-focus", () => ({}));
vi.mock("../../agent-model/composer-focus", () => ({
    cancelComposerFocusRequest: () => {},
    requestComposerFocus: () => {},
    focusComposer: () => {},
}));

import type { PaneTabHostContext } from "@/app/store/pane-tab-registry";
import type { AgentDefinition } from "@/app/store/rpc-api";
import { AgentViewModel } from "./agent-model";
import { CATCH_UP_META_KEY } from "./catch-up-request";
import type { LaunchOverrides } from "./launch-overrides";
import { getProvider, replaceProviderModels } from "./providers";

const ACCOUNT = "11111111-1111-4111-8111-111111111111";
const HOST: LaunchOverrides = {
    instanceName: "Korp",
    agentType: "host",
    environment: "local",
    accountId: "",
    bundleId: "",
};

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
    return {
        blockId: "block-1",
        meta: () => ({}),
        setMeta: async () => {},
        isFocused: () => true,
        visibility: () => "active",
    };
}

async function launch(agent: AgentDefinition, overrides?: LaunchOverrides, targetBlockId?: string) {
    let model!: AgentViewModel;
    const dispose = createRoot((d) => {
        model = new AgentViewModel(ctx());
        return d;
    });
    const ok = await model.launchAgentDefinition(agent, overrides, targetBlockId);
    dispose();
    return { ok, model };
}

const cmds = () => calls.map((c) => c.cmd);
const metaSets = () => calls.filter((c) => c.cmd === "SetMeta");
/** The meta the launch wrote to the block (the first SetMeta). */
const meta = () => metaSets()[0].data.meta as Meta;
const logged = (prefix: string) => logs.find((l) => l.startsWith(prefix));

beforeEach(() => {
    calls.length = 0;
    logs.length = 0;
    recalled.length = 0;
    remembered = undefined;
    answers = {
        ResolvePrereqsCommand: {
            results: [
                { tool: "node", found: true, path: "/usr/bin/node" },
                { tool: "npm", found: true, path: "/usr/bin/npm" },
            ],
        },
        ResolveCliCommand: { cli_path: "/cli/claude" },
        GetAllAgentContentCommand: [],
        EnsureProviderAuthDirCommand: { path: "/shared/providers/claude" },
        WriteAgentConfigCommand: (d: { working_dir: string }) => ({ working_dir: d.working_dir }),
        CreateAgentInstanceCommand: { id: "inst-1" },
        ListAgentIdentitiesCommand: [],
    };
});
afterEach(() => vi.useRealTimers());

describe("which provider a launch is for", () => {
    it("an agent with no provider of its own launches on its bundle's", async () => {
        answers.GetBundleCommand = { provider: "claude" };
        const { ok } = await launch(def({ provider: "", memory_id: "bundle-1" } as Partial<AgentDefinition>), {
            ...HOST,
            accountId: ACCOUNT,
            bundleId: "bundle-1",
        });
        expect(ok).toBe(true);
        expect(meta().agentProvider).toBe("claude");
        expect(calls.find((c) => c.cmd === "ResolveCli")!.data.provider_id).toBe("claude");
        expect(calls.find((c) => c.cmd === "LinkAgentIdentity")!.data.provider).toBe("claude");
        expect(logged("info: Launching agent definition")).toContain('"provider":"claude"');
        expect(logged("info: Launching agent definition")).toContain("(claude)");
    });

    it("names the provider it could not find, which is the bundle's when the agent has none", async () => {
        answers.GetBundleCommand = { provider: "nope" };
        const { ok } = await launch(def({ provider: "", memory_id: "bundle-1" } as Partial<AgentDefinition>), HOST);
        expect(ok).toBe(false);
        expect(logged("error: Unknown provider in agent definition")).toContain('"provider":"nope"');
        expect(cmds()).toEqual(["GetBundle"]);
    });
});

describe("whether it runs in a container", () => {
    it("the launch's own choice wins over the definition's: a host agent launched into a container needs no Node.js and no CLI", async () => {
        const { ok } = await launch(def(), { ...HOST, agentType: "container", environment: "docker" });
        expect(ok).toBe(true);
        expect(cmds()).not.toContain("ResolvePrereqs");
        expect(cmds()).not.toContain("ResolveCli");
        expect(meta().agentMode).toBe("container");
    });

    it("and the other way round: a container agent launched on the host needs both", async () => {
        const { ok } = await launch(def({ agent_type: "container" } as Partial<AgentDefinition>), HOST);
        expect(ok).toBe(true);
        expect(cmds()).toContain("ResolvePrereqs");
        expect(cmds()).toContain("ResolveCli");
        expect(meta().agentMode).toBe("host");
    });

    it("a container agent launched with no choice of its own runs in its container: no Node.js and no CLI", async () => {
        const { ok } = await launch(def({ agent_type: "container" } as Partial<AgentDefinition>));
        expect(ok).toBe(true);
        expect(cmds()).not.toContain("ResolvePrereqs");
        expect(cmds()).not.toContain("ResolveCli");
        expect(meta().agentMode).toBe("container");
    });

    it("a definition that says nothing runs on the host", async () => {
        const { ok } = await launch(def({ agent_type: undefined } as Partial<AgentDefinition>));
        expect(ok).toBe(true);
        expect(cmds()).toContain("ResolveCli");
        expect(meta().agentMode).toBe("host");
    });

    it("a harness that runs on this computer cannot be put in a container, whichever kind it is", async () => {
        const { ok, model } = await launch(def({ provider: "muse" } as Partial<AgentDefinition>), {
            ...HOST,
            agentType: "container",
            environment: "docker",
        });
        expect(ok).toBe(false);
        expect(model.launchError).toBe(
            `${getProvider("muse")!.displayName} runs on this computer, not in a container.`
        );
        expect(cmds()).not.toContain("SetMeta");
    });

    it("a container-capable harness is still launched into one", async () => {
        const { ok } = await launch(def(), { ...HOST, agentType: "container", environment: "docker" });
        expect(ok).toBe(true);
        expect(metaSets()).toHaveLength(1);
    });
});

describe("the agent's names", () => {
    it("takes the slug it was given", async () => {
        await launch(def({ slug: "custom-slug", name: "Korp" }));
        expect(meta()["cmd:env"].AGENTMUX_AGENT_SLUG).toBe("custom-slug");
    });

    it("derives one from the name when it has none", async () => {
        await launch(def({ slug: "", name: "Korp Prime" }));
        expect(meta()["cmd:env"].AGENTMUX_AGENT_SLUG).toBe("korp-prime");
    });

    it("trims the instance name the launch modal gave", async () => {
        await launch(def(), { ...HOST, instanceName: "  Korp 2  " });
        expect(meta().agentName).toBe("Korp 2");
    });

    it("takes the definition's own name when the launch gives none", async () => {
        await launch(def({ name: "Korp Prime", slug: "korp" }));
        expect(meta().agentName).toBe("Korp Prime");
    });
});

describe("the runtime and the arguments", () => {
    it("asks for what the agent remembers by the models its harness lists, and takes it", async () => {
        remembered = { model: "haiku" };
        await launch(def());
        expect(recalled).toEqual([{ agentId: "agent-1", models: getProvider("claude")!.models!.map((m) => m.value) }]);
        expect(meta()["agent:runtime"].model).toBe("haiku");
        const args = meta()["cmd:args"];
        expect(args[args.indexOf("--model") + 1]).toBe("haiku");
    });

    it("a fork's carried-over runtime wins over what the agent remembers", async () => {
        remembered = { model: "haiku" };
        await launch(def(), { ...HOST, carryOverRuntime: { model: "opus" } });
        expect(meta()["agent:runtime"].model).toBe("opus");
    });

    it("a model in the definition's flags is the runtime's, and its other flags reach the arguments", async () => {
        await launch(def({ provider_flags: "--model haiku --add-dir /x" } as Partial<AgentDefinition>));
        expect(meta()["agent:runtime"].model).toBe("haiku");
        const args = meta()["cmd:args"];
        expect(args).toContain("--add-dir");
        expect(args).toContain("/x");
        expect(meta()["agent:provider_flags"]).toBe("--model haiku --add-dir /x");
    });

    it("a model picked in the launch replaces the one in the flags, and only that flag goes", async () => {
        await launch(def({ provider_flags: "--model haiku --add-dir /x" } as Partial<AgentDefinition>), {
            ...HOST,
            model: "opus",
        });
        const args = meta()["cmd:args"];
        expect(args).not.toContain("haiku");
        expect(args).toContain("--add-dir");
        expect(meta()["agent:provider_flags"]).toBe("--add-dir /x");
        expect(meta()["agent:runtime"].model).toBe("opus");
    });

    it("a Codex agent has no model until its account's own list is known, then takes the list's first", async () => {
        replaceProviderModels("codex", []);
        await launch(def({ provider: "codex" } as Partial<AgentDefinition>));
        expect(meta()["agent:runtime"].model).toBe("");
        calls.length = 0;
        replaceProviderModels("codex", [{ value: "gpt-test", label: "Test" }]);
        try {
            await launch(def({ provider: "codex" } as Partial<AgentDefinition>));
            expect(meta()["agent:runtime"].model).toBe("gpt-test");
        } finally {
            replaceProviderModels("codex", []);
        }
    });
});

describe("the catch-up after a resume", () => {
    const resume = { ...HOST, continueOfInstanceId: "inst-0", continueSessionId: "sess-9" };

    it("asks the block the launch went into to catch up, and only then", async () => {
        const { ok } = await launch(def(), resume, "block-other");
        expect(ok).toBe(true);
        const sets = metaSets();
        expect(sets).toHaveLength(2);
        expect(sets[0].data.oref).toBe("block:block-other");
        expect(sets[1].data.oref).toBe("block:block-other");
        expect(String((sets[1].data.meta as Record<string, unknown>)[CATCH_UP_META_KEY])).toMatch(/^resumed:\d+$/);
    });

    it("is not asked for a fresh launch", async () => {
        await launch(def(), HOST);
        expect(metaSets()).toHaveLength(1);
    });

    it("is not asked for a fork, which is a new branch", async () => {
        await launch(def(), { ...resume, forkSession: true });
        expect(metaSets()).toHaveLength(1);
    });

    it("does not fail a launch that is already running when the request cannot be written", async () => {
        let n = 0;
        answers.SetMetaCommand = () => {
            if (++n === 2) return Promise.reject(new Error("meta down"));
        };
        const { ok } = await launch(def(), resume);
        expect(ok).toBe(true);
        expect(n).toBe(2);
    });
});
