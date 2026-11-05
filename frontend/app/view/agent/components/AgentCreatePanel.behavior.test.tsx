// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * AgentCreatePanel, pinned as a whole before it was split into a state object, hook groups and view parts: each
 * scenario drives the panel through its steps (the tools check, the install of a CLI AgentMux manages or one it
 * doesn't, the free sign-up guide, the runtime choice that skips the host steps, the Create form's wiring) and
 * logs, in order, the whole panel as a tree and every call it made outward. The logs are snapshots recorded on
 * the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { createSignal } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from "vitest";

type AnyMock = Mock<(...args: any[]) => any>;

const { openExternal, pushNotification, getPlatform, clipboard } = vi.hoisted(() => ({
    openExternal: vi.fn(),
    pushNotification: vi.fn(),
    getPlatform: vi.fn(() => "linux"),
    clipboard: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        InstallStartCommand: vi.fn(),
        InstallCancelCommand: vi.fn(),
        InstallCheckCommand: vi.fn(),
        ToolchainResolveInstallCommandCommand: vi.fn(),
        ToolchainInstallSystemToolCommand: vi.fn(),
        ListIdentityAccountsCommand: vi.fn(),
        CheckCliAuthCommand: vi.fn(),
        ContainerImageCheckCommand: vi.fn(async () => ({ status: dockerForTest.image })),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", async () => {
    const { isNoPayloadMuxEvent } = await vi.importActual<typeof import("@/app/store/mps-events.generated")>(
        "@/app/store/mps-events.generated",
    );
    const mock = { muxEventSubscribe: vi.fn(() => () => {}) };
    const onMuxEvent = (eventType: string, handler: (data: unknown, event: unknown) => void, scope?: string) =>
        (mock.muxEventSubscribe as (sub: unknown) => () => void)({
            eventType,
            scope,
            handler: (e: any) => {
                if (e?.data != null) handler(e.data, e);
                else if (isNoPayloadMuxEvent(eventType)) handler(null, e);
            },
        } as any);
    return { ...mock, onMuxEvent };
});
vi.mock("@/app/store/global", () => ({
    getApi: () => ({ openExternal }),
    getSettingsKeyAtom: () => () => undefined,
    isDev: false,
}));
vi.mock("@/app/store/flash-notifications", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/app/store/flash-notifications")>()),
    pushNotification,
}));
vi.mock("@/app/store/contextmenu", () => ({ ContextMenuModel: { showContextMenu: vi.fn() } }));
vi.mock("@/util/clipboard", () => ({ writeText: clipboard }));
vi.mock("@/util/platformutil", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/util/platformutil")>()),
    getPlatform,
}));
vi.mock("../defaults/cli-catalog", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../defaults/cli-catalog")>()),
    getCliCatalogEntry: (id: string) =>
        ({
            codex: { displayName: "Codex", icon: "🤖", popoverMarkdown: "" },
            antigravity: { displayName: "Antigravity", icon: "✦", popoverMarkdown: "" },
        })[id],
}));
vi.mock("../providers", async (importOriginal) => ({
    ...(await importOriginal<typeof import("../providers")>()),
    getProvider: (id: string) =>
        ({
            codex: {
                id: "codex",
                displayName: "Codex",
                cliCommand: "codex",
                npmPackage: "@openai/codex",
                pinnedVersion: "0.116.0",
                docsUrl: "https://example.test/codex",
                windowsInstallCommand: "npm install -g @openai/codex",
                unixInstallCommand: "npm install -g @openai/codex",
            },
            antigravity: {
                id: "antigravity",
                displayName: "Antigravity",
                cliCommand: "agy",
                authType: "cli-managed",
                authCheckCommand: ["models"],
                npmPackage: "",
                pinnedVersion: "",
                docsUrl: "https://example.test/agy-docs",
                windowsInstallCommand: "irm https://example.test/install.ps1 | iex",
                unixInstallCommand: "curl -fsSL https://example.test/install.sh | bash",
            },
            nodocs: {
                id: "nodocs",
                displayName: "NoDocs",
                cliCommand: "nodocs",
                npmPackage: "",
                pinnedVersion: "",
                docsUrl: "",
                windowsInstallCommand: "",
                unixInstallCommand: "",
            },
        })[id],
}));
vi.mock("../agent-launch-env", () => ({
    resolveEffectiveLaunchProvider: vi.fn(async (t: { provider: string }) => t.provider),
}));

const accountsForTest = {
    list: [] as { provider: string }[],
    gate: null as null | Promise<void>,
    fail: false,
    cliSignedIn: false as boolean | "throws",
};

const formCalls = vi.hoisted(() => ({ log: [] as string[] }));
// The Create step's own behaviour is AgentCreateForm.test.tsx's: here it shows what the panel handed it.
vi.mock("./AgentCreateForm", async (importOriginal) => ({
    AgentCreateForm: (props: any) => (
        <div
            data-testid="agent-create-form"
            data-template={props.template.name}
            data-runtime={props.draft.runtime()}
            data-deciding={String(props.runtime.deciding())}
        >
            <button onClick={() => props.onCancel()}>form-cancel</button>
            <button onClick={() => props.onBusyChange(true)}>form-busy</button>
            <button
                onClick={() => {
                    formCalls.log.push("form onCreated");
                    void props.onCreated({ id: "a1" });
                }}
            >
                form-created
            </button>
        </div>
    ),
    createCreateDraft: (await importOriginal<typeof import("./AgentCreateForm")>()).createCreateDraft,
}));

type DockerStatus = "unknown" | "checking" | "available" | "unavailable";
const dockerForTest = vi.hoisted(() => ({
    image: "ok" as string,
    set: (_s: "unknown" | "checking" | "available" | "unavailable") => {},
}));
vi.mock("@/app/store/toolchain-capabilities", async () => {
    const { createSignal } = await import("solid-js");
    const [status, setStatus] = createSignal<DockerStatus>("unavailable");
    dockerForTest.set = setStatus;
    return {
        getCapability: () => ({ status: status() }),
        isAvailable: () => status() === "available",
        watchCapability: () => () => {},
    };
});
const lastRuntimeForTest = { value: null as "host" | "container" | null };
vi.mock("../defaults/runtime-choice", () => ({
    loadLastRuntime: () => lastRuntimeForTest.value,
    storeLastRuntime: vi.fn(),
}));

import { AgentCreatePanel, type MissingPrereq } from "./AgentCreatePanel";
import { createCreateDraft } from "./AgentCreateForm";
import { RpcApi } from "@/app/store/rpc-api";
import { muxEventSubscribe } from "@/app/store/mps";
import { resolveEffectiveLaunchProvider } from "../agent-launch-env";
import { invalidateInstallCheck } from "../install-check-cache";
import { __resetAgentInstallSessions } from "../install-sessions";
import { resetInstallDetailsPrefs } from "@/element/install/InstallProgress";
import type { AgentDefinition } from "@/app/store/rpc-api";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/AgentCreatePanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const baseTemplate = (over: Partial<AgentDefinition> = {}): AgentDefinition =>
    ({
        id: "tpl-1",
        slug: "tpl",
        name: "Codex",
        icon: "",
        provider: "codex",
        description: "",
        working_directory: "",
        shell: "",
        provider_flags: "",
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "host",
        environment: "local",
        agent_bus_id: "",
        is_seeded: 1,
        memory_id: "",
        ...over,
    }) as AgentDefinition;

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|href|open)$/;

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
    if (el.tagName === "svg" || el.tagName === "STYLE") return out;
    const attrs = [...el.attributes]
        .filter((a) => ATTRS.test(a.name))
        .map((a) => `${a.name}=${JSON.stringify(a.value.replace(/(-cl)-\d+/g, "$1-N"))}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const mocks = { probe: null as AnyMock | null, recheck: null as AnyMock | null, onClose: null as AnyMock | null, onCreated: null as AnyMock | null, onBusy: null as AnyMock | null };
const seen = new Map<string, number>();
const watched: [string, () => AnyMock | Mock][] = [
    ["probePrereqs", () => mocks.probe!],
    ["recheckInstalled", () => mocks.recheck!],
    ["onClose", () => mocks.onClose!],
    ["onCreated", () => mocks.onCreated!],
    ["onBusyChange", () => mocks.onBusy!],
    ["openExternal", () => openExternal],
    ["clipboard", () => clipboard],
    ["pushNotification", () => pushNotification],
    ["InstallStart", () => RpcApi.InstallStartCommand as Mock],
    ["InstallCancel", () => RpcApi.InstallCancelCommand as Mock],
    ["ToolchainResolve", () => RpcApi.ToolchainResolveInstallCommandCommand as Mock],
    ["ToolchainInstall", () => RpcApi.ToolchainInstallSystemToolCommand as Mock],
    ["ListIdentityAccounts", () => RpcApi.ListIdentityAccountsCommand as Mock],
    ["CheckCliAuth", () => RpcApi.CheckCliAuthCommand as Mock],
    ["InstallCheck", () => RpcApi.InstallCheckCommand as Mock],
];

/** The calls made outward since the last look. */
function calls() {
    for (const [name, get] of watched) {
        const made = get().mock.calls;
        for (let i = seen.get(name) ?? 0; i < made.length; i++) {
            log(`  call ${name} ${JSON.stringify(made[i].slice(name.startsWith("Install") || name.startsWith("Toolchain") || name.startsWith("List") || name.startsWith("Check") ? 1 : 0), (k, v) => (k === "timestamp" || k === "expiration" ? undefined : v))}`);
        }
        seen.set(name, made.length);
    }
    const form = formCalls.log;
    for (let i = seen.get("form") ?? 0; i < form.length; i++) log(`  ${form[i]}`);
    seen.set("form", form.length);
}

let container: HTMLElement;
function panel(label: string) {
    log(`== ${label}`);
    dump(container).forEach((l) => log(l));
    calls();
}

const flush = async () => {
    for (let i = 0; i < 8; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    for (let i = 0; i < 8; i++) await Promise.resolve();
};

interface RenderOpts {
    template?: AgentDefinition;
    installed?: boolean | undefined;
    missing?: MissingPrereq[];
    probePrereqs?: AnyMock;
    recheckInstalled?: AnyMock;
}

function renderPanel(opts: RenderOpts = {}) {
    const template = opts.template ?? baseTemplate();
    const [installed, setInstalled] = createSignal<boolean | undefined>("installed" in opts ? opts.installed : true);
    mocks.probe = opts.probePrereqs ?? vi.fn().mockResolvedValue(opts.missing ?? []);
    mocks.recheck =
        opts.recheckInstalled ??
        vi.fn(async () => {
            setInstalled(true);
        });
    mocks.onCreated = vi.fn().mockResolvedValue(undefined);
    mocks.onClose = vi.fn();
    mocks.onBusy = vi.fn();
    const draft = createCreateDraft(template);
    const result = render(() => (
        <AgentCreatePanel
            template={template}
            draft={draft}
            installed={installed()}
            probePrereqs={mocks.probe!}
            recheckInstalled={mocks.recheck!}
            onCreated={mocks.onCreated!}
            onClose={mocks.onClose!}
            onBusyChange={mocks.onBusy!}
        />
    ));
    container = result.container;
    return { ...result, setInstalled };
}

async function chunkSender(scope: string): Promise<(data: Record<string, unknown>) => void> {
    let handler!: (e: unknown) => void;
    await waitFor(() => {
        const sub = vi
            .mocked(muxEventSubscribe)
            .mock.calls.map(([s]) => s as unknown as { scope?: string; handler: (e: unknown) => void })
            .find((s) => s.scope === scope);
        expect(sub).toBeTruthy();
        handler = sub!.handler;
    });
    return (data) => handler({ data });
}

async function startAndSubscribe(): Promise<(data: Record<string, unknown>) => void> {
    fireEvent.click(await screen.findByTestId("agent-create-install-start"));
    const send = await chunkSender("install:sess-1");
    return (data) => send({ sessionId: "sess-1", ...data });
}

const git: MissingPrereq = { tool: "git", label: "Git", installUrl: "https://git-scm.com", installLinkText: "Install Git" };
const jq: MissingPrereq = { tool: "jq", label: "jq", installUrl: "https://jqlang.org", installLinkText: "Get jq" };
const gitResolved = {
    available: true,
    program: "winget",
    args: ["install", "--id", "Git.Git"],
    needsElevation: true,
    commandPreview: "winget install --id Git.Git",
    resolvedVersion: "2.47.1",
};

beforeEach(() => {
    T.length = 0;
    seen.clear();
    formCalls.log.length = 0;
    invalidateInstallCheck();
    vi.mocked(RpcApi.InstallStartCommand).mockResolvedValue({ sessionId: "sess-1" } as any);
    vi.mocked(RpcApi.InstallCancelCommand).mockResolvedValue({} as any);
    vi.mocked(resolveEffectiveLaunchProvider).mockImplementation(async (t) => t.provider);
    getPlatform.mockReturnValue("linux");
    clipboard.mockResolvedValue(undefined);
    dockerForTest.set("unavailable");
    dockerForTest.image = "ok";
    lastRuntimeForTest.value = null;
    accountsForTest.list = [];
    accountsForTest.gate = null;
    accountsForTest.fail = false;
    accountsForTest.cliSignedIn = false;
    vi.mocked(RpcApi.InstallCheckCommand).mockResolvedValue({ installed: true, version: null, path: "/bin/agy", source: "system_path" } as any);
    vi.mocked(RpcApi.CheckCliAuthCommand).mockImplementation((async () => {
        if (accountsForTest.cliSignedIn === "throws") throw new Error("check failed");
        return { authenticated: accountsForTest.cliSignedIn };
    }) as any);
    vi.mocked(RpcApi.ListIdentityAccountsCommand).mockImplementation((async () => {
        if (accountsForTest.gate) await accountsForTest.gate;
        if (accountsForTest.fail) throw new Error("accounts unavailable");
        return accountsForTest.list;
    }) as any);
});
afterEach(() => {
    cleanup();
    vi.clearAllMocks();
    resetInstallDetailsPrefs();
    __resetAgentInstallSessions();
});

describe("AgentCreatePanel behavior", () => {
    it("goes straight to Create when nothing is missing, and wires the form through", async () => {
        let answer!: (v: MissingPrereq[]) => void;
        renderPanel({ probePrereqs: vi.fn(() => new Promise<MissingPrereq[]>((r) => (answer = r))) });
        panel("probe not answered");
        answer([]);
        await flush();
        panel("create");
        fireEvent.click(screen.getByText("form-busy"));
        fireEvent.click(screen.getByText("form-created"));
        fireEvent.click(screen.getByText("form-cancel"));
        await flush();
        panel("form callbacks");
        await snapshot("create");
    });

    it("lists the missing tools, installs one inline, and keeps it as installed though the stale probe lists it", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockImplementation((async (_c: unknown, a: { toolId: string }) =>
            a.toolId === "git" ? gitResolved : { available: false }) as any);
        vi.mocked(RpcApi.ToolchainInstallSystemToolCommand).mockResolvedValue({ sessionId: "sysinstall-1" } as any);
        renderPanel({ probePrereqs: vi.fn().mockResolvedValue([git, jq]) });
        await flush();
        panel("two tools missing");
        fireEvent.click(screen.getByText("Get jq"));
        fireEvent.click(screen.getByText("Install v2.47.1 now ↓"));
        await flush();
        panel("git's inline install open");
        fireEvent.click(screen.getByText("Install v2.47.1 now"));
        const send = await chunkSender("install:sysinstall-1");
        await flush();
        send({ op: "done", ok: true });
        await flush();
        panel("git installed, restart needed");
        fireEvent.click(screen.getByRole("button", { name: "Check again" }));
        await flush();
        panel("checked again");
        fireEvent.click(screen.getByTestId("agent-create-tools-continue"));
        await flush();
        panel("continued anyway");
        await snapshot("tools");
    });

    it("moves on once every tool is installed inline, and closes from the tools step", async () => {
        vi.mocked(RpcApi.ToolchainResolveInstallCommandCommand).mockResolvedValue(gitResolved as any);
        vi.mocked(RpcApi.ToolchainInstallSystemToolCommand).mockResolvedValue({ sessionId: "sysinstall-1" } as any);
        renderPanel({ probePrereqs: vi.fn().mockResolvedValue([git]) });
        await flush();
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        fireEvent.click(await screen.findByText("Install v2.47.1 now ↓"));
        fireEvent.click(await screen.findByText("Install v2.47.1 now"));
        const send = await chunkSender("install:sysinstall-1");
        send({ op: "done", ok: true });
        await flush();
        panel("every tool installed: Create, Tools kept in the trail");
        await snapshot("tools-done");
    });

    it("installs the CLI AgentMux manages: start, progress, a failure and Retry", async () => {
        const recheckInstalled = vi.fn().mockResolvedValue(undefined);
        renderPanel({ installed: false, recheckInstalled });
        await flush();
        panel("not installed: install step");
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        const send = await startAndSubscribe();
        await flush();
        panel("running");
        send({ line: "npm http fetch GET 200 https://registry.npmjs.org/chalk/-/chalk-4.1.2.tgz 12ms", stream: "stderr" });
        await flush();
        panel("downloading");
        fireEvent.click(screen.getByTestId("agent-create-install-cancel"));
        await flush();
        send({ op: "done", ok: false, error: "npm exited Some(1)" });
        await flush();
        panel("failed");
        fireEvent.click(screen.getByRole("button", { name: "Close" }));
        fireEvent.click(screen.getByRole("button", { name: "Retry" }));
        await flush();
        panel("retrying");
        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await flush();
        panel("done: the re-check has not caught up");
        await snapshot("install-npm");
    });

    it("goes on to Create on Continue even when the re-check has not caught up", async () => {
        renderPanel({ installed: false, recheckInstalled: vi.fn().mockResolvedValue(undefined) });
        const send = await startAndSubscribe();
        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await flush();
        panel("done, installed still false");
        fireEvent.click(screen.getByTestId("agent-create-install-continue"));
        await flush();
        panel("continued to Create");
        await snapshot("install-npm-continue");
    });

    it("closes from a finished install whose re-check has not caught up", async () => {
        const { container: c } = renderPanel({ installed: false, recheckInstalled: vi.fn().mockResolvedValue(undefined) });
        container = c;
        const send = await startAndSubscribe();
        send({ op: "done", ok: true });
        await flush();
        fireEvent.click(screen.getByRole("button", { name: "Close" }));
        panel("closed from the done state");
        await snapshot("install-npm-close");
    });

    it("falls back to the template's own provider when the launch resolves none", async () => {
        vi.mocked(resolveEffectiveLaunchProvider).mockImplementation((async () => undefined) as any);
        renderPanel();
        await flush();
        panel("installed, no resolved provider: Create");
        cleanup();
        renderPanel({ installed: false });
        await flush();
        panel("not installed, no resolved provider");
        await snapshot("provider-fallback");
    });

    it("moves on by itself when the install ends here", async () => {
        renderPanel({ installed: false });
        const send = await startAndSubscribe();
        await flush();
        send({ line: "added 1 package in 1s", stream: "stdout" });
        send({ op: "done", ok: true });
        await flush();
        panel("done, the panel re-checked and moved on");
        await snapshot("install-npm-here");
    });

    it("keeps installing when the panel closes, shows the same run on reopening, and toasts the end once closed", async () => {
        const first = renderPanel({ installed: false });
        const send = await startAndSubscribe();
        await flush();
        first.unmount();
        calls();
        const second = renderPanel({ installed: false });
        await flush();
        panel("reopened: the same run");
        second.unmount();
        send({ op: "done", ok: true });
        await flush();
        log(`after unmount: ${document.body.children.length} body children`);
        calls();
        await snapshot("install-npm-away");
    });

    it("shows why the install could not start, and a start RPC that fails", async () => {
        vi.mocked(RpcApi.InstallStartCommand).mockRejectedValueOnce(new Error("provider codex install already in progress"));
        renderPanel({ installed: false });
        fireEvent.click(await screen.findByTestId("agent-create-install-start"));
        await flush();
        panel("start failed");
        await snapshot("install-npm-start-fails");
    });

    it("shows the vendor's installer for a CLI AgentMux does not install, per platform, with Check again", async () => {
        const vendor = baseTemplate({ name: "Antigravity", provider: "antigravity" });
        const recheck = vi.fn().mockResolvedValue(undefined);
        renderPanel({ template: vendor, installed: false, recheckInstalled: recheck });
        await flush();
        panel("unix");
        fireEvent.click(screen.getByText(/Antigravity's install instructions/));
        fireEvent.click(screen.getByText("Copy"));
        fireEvent.click(screen.getByTestId("agent-create-check-again"));
        panel("checking again");
        await flush();
        panel("still not found");
        cleanup();
        getPlatform.mockReturnValue("win32");
        renderPanel({ template: vendor, installed: false });
        await flush();
        panel("windows");
        fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
        cleanup();
        renderPanel({ template: vendor, installed: false });
        await flush();
        fireEvent.click(screen.getByTestId("agent-create-check-again"));
        await flush();
        panel("found: Create");
        cleanup();
        renderPanel({ template: baseTemplate({ name: "NoDocs", provider: "nodocs" }), installed: false });
        await flush();
        panel("a vendor CLI with no command and no docs");
        await snapshot("install-vendor");
    });

    it("shows the free sign-up guide only for someone with no account, after the list and the CLI check answer", async () => {
        const agy = () => baseTemplate({ provider: "antigravity", name: "Antigravity" });
        let release!: () => void;
        accountsForTest.gate = new Promise<void>((r) => (release = r));
        renderPanel({ template: agy() });
        await flush();
        panel("account list pending");
        release();
        await flush();
        panel("no account: guide");
        fireEvent.click(screen.getByTestId("agent-free-guide-open"));
        calls();
        cleanup();
        accountsForTest.gate = null;
        accountsForTest.list = [{ provider: "antigravity" }];
        renderPanel({ template: agy() });
        await flush();
        panel("has an account");
        cleanup();
        accountsForTest.list = [];
        accountsForTest.fail = true;
        renderPanel({ template: agy() });
        await flush();
        panel("accounts unreadable");
        cleanup();
        accountsForTest.fail = false;
        accountsForTest.cliSignedIn = true;
        renderPanel({ template: agy() });
        await flush();
        panel("CLI signed in");
        cleanup();
        accountsForTest.cliSignedIn = "throws";
        renderPanel({ template: agy() });
        await flush();
        panel("CLI check throws");
        cleanup();
        accountsForTest.cliSignedIn = false;
        vi.mocked(RpcApi.InstallCheckCommand).mockResolvedValue({ installed: false, version: null, path: null, source: null } as any);
        invalidateInstallCheck();
        renderPanel({ template: agy() });
        await flush();
        panel("CLI not installed");
        cleanup();
        renderPanel();
        await flush();
        panel("a provider whose CLI has no login of its own: account list decides");
        await snapshot("guide");
    });

    it("lets the runtime decide whether the host steps apply", async () => {
        dockerForTest.set("available");
        lastRuntimeForTest.value = "container";
        renderPanel({ installed: false, missing: [git] });
        await flush();
        panel("container chosen: straight to Create");
        cleanup();
        dockerForTest.set("checking");
        renderPanel({ installed: false });
        await flush();
        panel("docker checking: waits");
        dockerForTest.set("available");
        await flush();
        panel("docker answered");
        cleanup();
        dockerForTest.image = "denied";
        renderPanel({ installed: false });
        await flush();
        panel("image refused: native");
        cleanup();
        dockerForTest.image = "ok";
        lastRuntimeForTest.value = null;
        renderPanel({ installed: false });
        await flush();
        panel("container offered on the install step");
        fireEvent.click(screen.getByTestId("agent-create-use-container"));
        await flush();
        panel("container taken");
        cleanup();
        lastRuntimeForTest.value = "host";
        renderPanel({ installed: false, missing: [jq] });
        await flush();
        panel("native chosen: offer on the tools step");
        dockerForTest.set("checking");
        await flush();
        panel("docker re-probed");
        dockerForTest.set("unavailable");
        await flush();
        panel("no docker: no offer");
        await snapshot("runtime");
    });
});
