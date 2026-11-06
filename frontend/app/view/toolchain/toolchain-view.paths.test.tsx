// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ToolchainView, beyond the happy path of toolchain-view.test.tsx: the probes behind each row
 * and what they ask for, the shared-capability rows (Docker), the latest-version lookup and its
 * cache, Refresh, the install panel, the cards' wording and details, the filter, the widget
 * cards and their ports, and the environment section. The catalogs are the real ones; the
 * RPC, the install-check cache, the capability store, the platform and the clipboard are fakes.
 */

import { getProviderList } from "@/app/view/agent/providers";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@solidjs/testing-library";
import { createSignal, type Accessor, type Setter } from "solid-js";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

type CliHit = { version?: string; cli_path: string; source?: string };
type Installed = { installed: boolean; version?: string | null; path?: string | null; source?: string | null };
type Cap = { status: string; version?: string; path?: string; source?: string };
type InstallProps = { toolId: string; onInstalled: () => void; onUnavailable: () => void };

const h = vi.hoisted(() => ({
    platform: "darwin",
    cli: {} as Record<string, CliHit>,
    installed: {} as Record<string, Installed>,
    health: {} as Record<number, { healthy: boolean; status_code?: number }>,
    versions: {} as Record<string, string | null>,
    available: new Set<string>(),
    invalidated: null as null | ((id?: string | null) => void),
    unsubInstall: vi.fn(),
    unwatch: vi.fn(),
    cap: null as unknown as Accessor<Record<string, Cap>>,
    setCap: null as unknown as Setter<Record<string, Cap>>,
    install: null as unknown as InstallProps,
    openExternal: vi.fn(),
    createBlock: vi.fn(),
    clipboard: vi.fn(),
    clipboardFails: false,
    rpc: {
        ResolveCliCommand: vi.fn(),
        ToolchainEnvCommand: vi.fn(),
        ToolchainVersionsCommand: vi.fn(),
        WidgetHealthCommand: vi.fn(),
    },
    checkProviderInstalled: vi.fn(),
    invalidateInstallCheck: vi.fn(),
    ensureCapability: vi.fn(),
    watchCapability: vi.fn(),
}));

vi.mock("@/app/store/rpc-api", () => ({ RpcApi: h.rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/store/global", () => ({ getApi: () => ({ openExternal: h.openExternal }) }));
vi.mock("@/store/block-layout-actions", () => ({ createBlock: h.createBlock }));
vi.mock("@/util/platformutil", async (importOriginal) => ({
    ...(await importOriginal<typeof import("@/util/platformutil")>()),
    getPlatform: () => h.platform,
}));
vi.mock("@/app/view/agent/install-check-cache", () => ({
    checkProviderInstalled: h.checkProviderInstalled,
    invalidateInstallCheck: h.invalidateInstallCheck,
    onInstallCheckInvalidated: (cb: (id?: string | null) => void) => {
        h.invalidated = cb;
        return h.unsubInstall;
    },
}));
vi.mock("@/app/store/toolchain-capabilities", () => ({
    ensureCapability: h.ensureCapability,
    getCapability: (id: string) => h.cap()[id] ?? { status: "unknown" },
    isAvailable: (id: string) => h.available.has(id),
    watchCapability: h.watchCapability,
}));
// A plain promise, not a mock's: a mock's result is watched, and a rejection nobody handles would go unseen.
vi.mock("@/util/clipboard", () => ({
    writeText: (cmd: string) => {
        h.clipboard(cmd);
        return h.clipboardFails ? Promise.reject(new Error("denied")) : Promise.resolve();
    },
}));
vi.mock("./SystemToolInstallInline", () => ({
    SystemToolInstallInline: (p: InstallProps) => {
        h.install = p;
        return <div data-testid="install-inline" data-tool={p.toolId} />;
    },
}));
vi.mock("./CliDiskSpace", () => ({ CliDiskSpace: () => <div data-testid="cli-disk-space" /> }));
vi.mock("./ProviderUpdateInline", () => ({
    ProviderUpdateInline: (p: { providerId: string; pinned: string }) => (
        <div data-testid="provider-update">{`${p.providerId} -> ${p.pinned}`}</div>
    ),
}));

import { ToolchainView } from "./toolchain-view";

[h.cap, h.setCap] = createSignal<Record<string, Cap>>({});

const HOUR = 60 * 60 * 1000;
let clock = Date.UTC(2026, 0, 1);

const card = (id: string) => document.querySelector<HTMLElement>(`[data-tool="${id}"]`)!;
const wcard = (id: string) => document.querySelector<HTMLElement>(`[data-widget="${id}"]`)!;
const dot = (el: HTMLElement) => within(el).getByRole("img");
const open = (el: HTMLElement) => fireEvent.click(el);
const button = (name: string | RegExp) => screen.getByRole("button", { name });
const filterBox = () => screen.getByPlaceholderText("Filter tools");

/** The view, mounted, with every row's first probe answered. */
async function mount(props: { zoom?: () => number } = {}) {
    const view = render(() => <ToolchainView {...props} />);
    await waitFor(() => expect(within(card("git")).queryByRole("img", { name: "Checking" })).toBeNull());
    await waitFor(() => expect(within(card("claude")).queryByRole("img", { name: "Checking" })).toBeNull());
    await waitFor(() => expect(within(wcard("comfyui")).queryByRole("img", { name: "Checking" })).toBeNull());
    return view;
}

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["Date"] });
    clock += 7 * HOUR;
    vi.setSystemTime(clock);
    localStorage.clear();
    h.platform = "darwin";
    h.cli = {
        git: { version: "2.40.0", cli_path: "/usr/bin/git", source: "system_path" },
        node: { version: "unknown", cli_path: "/usr/bin/node", source: "local_install" },
    };
    h.installed = {};
    h.health = {};
    h.versions = {};
    h.available = new Set();
    h.invalidated = null;
    h.setCap({});
    for (const f of [
        h.unsubInstall,
        h.unwatch,
        h.openExternal,
        h.createBlock,
        h.clipboard,
        h.checkProviderInstalled,
        h.invalidateInstallCheck,
        h.ensureCapability,
        h.watchCapability,
        ...Object.values(h.rpc),
    ]) {
        f.mockReset();
    }
    h.rpc.ResolveCliCommand.mockImplementation((_c: unknown, data: { provider_id: string }) =>
        h.cli[data.provider_id] ? Promise.resolve(h.cli[data.provider_id]) : Promise.reject(new Error("not found"))
    );
    h.rpc.ToolchainEnvCommand.mockResolvedValue({
        path: "/bin:/usr/bin",
        pathSource: "login-shell",
        os: "linux",
        arch: "x64",
    });
    h.rpc.ToolchainVersionsCommand.mockImplementation(() => Promise.resolve(h.versions));
    h.rpc.WidgetHealthCommand.mockImplementation((_c: unknown, data: { port: number }) =>
        Promise.resolve(h.health[data.port] ?? { healthy: false })
    );
    h.checkProviderInstalled.mockImplementation((a: { providerId: string }) =>
        Promise.resolve(h.installed[a.providerId] ?? { installed: false })
    );
    h.ensureCapability.mockResolvedValue(undefined);
    h.watchCapability.mockReturnValue(h.unwatch);
    h.clipboardFails = false;
});

afterEach(() => {
    cleanup();
    vi.useRealTimers();
});

describe("what each row asks for", () => {
    it("looks up each core and local tool by its command, with the tool's id", async () => {
        await mount();
        const asked = h.rpc.ResolveCliCommand.mock.calls.map(([, d, o]) => [d.provider_id, d.cli_command, o?.timeout]);
        expect(asked).toContainEqual(["git", "git", 12000]);
        expect(asked).toContainEqual(["node", "node", 12000]);
        expect(asked).toContainEqual(["ollama", "ollama", 12000]);
        expect(asked.map(([id]) => id)).not.toContain("docker");
        expect(asked.map(([id]) => id)).not.toContain("claude");
        expect(h.rpc.ResolveCliCommand.mock.calls[0][1]).toMatchObject({
            npm_package: "",
            pinned_version: "",
            windows_install_command: "",
            unix_install_command: "",
        });
    });

    it("asks the install check about each agent CLI, naming its package only when it has one", async () => {
        await mount();
        const byId = Object.fromEntries(h.checkProviderInstalled.mock.calls.map(([a]) => [a.providerId, a]));
        expect(byId.claude).toEqual({
            providerId: "claude",
            cliCommand: "claude",
            npmPackage: "@anthropic-ai/claude-code",
        });
        expect(byId.kimi).toEqual({ providerId: "kimi", cliCommand: "kimi", npmPackage: undefined });
        expect(h.invalidateInstallCheck).not.toHaveBeenCalled();
    });

    it("reads an agent CLI's version, path and source from the install check", async () => {
        h.installed.claude = { installed: true, version: "2.1.288", path: "/opt/claude", source: "system_path" };
        await mount();
        expect(within(card("claude")).getByRole("img", { name: "Installed" })).toBeInTheDocument();
        expect(card("claude").textContent).toContain("2.1.288");
        expect(card("claude").textContent).toContain("your install");
        open(card("claude"));
        expect(await screen.findByText("/opt/claude")).toBeInTheDocument();
        expect(dot(card("codex"))).toHaveAccessibleName("Not found");
    });

    it("shows no path for an installed agent CLI that gave none", async () => {
        h.installed.claude = { installed: true, version: "2.1.288", path: null, source: null };
        await mount();
        open(card("claude"));
        await screen.findByText("up to date");
        expect(screen.queryByText("Path")).toBeNull();
        expect(card("claude").textContent).not.toContain("your install");
    });

    it("shows an agent CLI whose check throws as not found", async () => {
        h.checkProviderInstalled.mockRejectedValue(new Error("boom"));
        await mount();
        expect(dot(card("claude"))).toHaveAccessibleName("Not found");
    });

    it("reads a core tool's version, path and source, and calls an unknown version just installed", async () => {
        await mount();
        expect(card("git").textContent).toContain("2.40.0");
        expect(dot(card("git"))).toHaveAccessibleName("Installed");
        expect(card("node").textContent).toContain("managed");
        expect(within(card("node")).getByText("Installed")).toBeInTheDocument();
        open(card("git"));
        expect(await screen.findByText("/usr/bin/git")).toBeInTheDocument();
    });

    it("shows a card as checking until its first answer, with a line saying what it looks for", async () => {
        let finish: (v: CliHit) => void = () => {};
        h.rpc.ResolveCliCommand.mockImplementation(() => new Promise<CliHit>((resolve) => (finish = resolve)));
        render(() => <ToolchainView />);
        expect(dot(card("git"))).toHaveAccessibleName("Checking");
        expect(card("git").textContent).toContain("Checking…");
        open(card("git"));
        expect(await screen.findByText("Looking for Git on this machine…")).toBeInTheDocument();
        expect(screen.queryByText(/Not found on this machine/)).toBeNull();
        expect(within(card("git")).queryByRole("button", { name: "Install" })).toBeNull();
        finish({ version: "1.0.0", cli_path: "/x", source: "system_path" });
    });

    it("shows a core tool whose lookup fails as not found, and an optional one as optional", async () => {
        await mount();
        expect(dot(card("python"))).toHaveAccessibleName("Not found");
        expect(dot(card("uv"))).toHaveAccessibleName("Not installed (optional)");
        expect(card("python").className).toContain("is-attention");
        expect(card("python").className).not.toContain("is-muted");
        expect(card("uv").className).toContain("is-muted");
        expect(card("git").className).not.toContain("is-muted");
    });

    it("keeps a card as checking while it is looked up, not muted", async () => {
        h.rpc.ResolveCliCommand.mockImplementation(() => new Promise(() => {}));
        render(() => <ToolchainView />);
        expect(card("uv").className).not.toContain("is-muted");
    });

    it("gives each card its tool as an attribute, its status in the name, and the right icon", async () => {
        await mount();
        expect(card("git")).toHaveAccessibleName("Git, Installed");
        expect(card("git").querySelector("i")!.className).toBe("fa-brands fa-git-alt");
        expect(card("uv").querySelector("i")!.className).toBe("fa-solid fa-bolt");
        expect(wcard("comfyui")).toHaveAccessibleName("ComfyUI, Not installed");
        expect(wcard("comfyui").querySelector("i")!.className).toContain("fa-solid fa-");
    });
});

describe("the shared capability rows", () => {
    it("asks the capability store for Docker instead of looking it up", async () => {
        render(() => <ToolchainView />);
        await waitFor(() => expect(h.ensureCapability).toHaveBeenCalledWith("docker", { force: undefined }));
        expect(h.rpc.ResolveCliCommand.mock.calls.map(([, d]) => d.provider_id)).not.toContain("docker");
        expect(h.watchCapability).toHaveBeenCalledTimes(1);
        expect(h.watchCapability).toHaveBeenCalledWith("docker");
    });

    it("stops watching Docker when the view goes", async () => {
        const { unmount } = render(() => <ToolchainView />);
        expect(h.unwatch).not.toHaveBeenCalled();
        unmount();
        expect(h.unwatch).toHaveBeenCalledTimes(1);
        expect(h.unsubInstall).toHaveBeenCalledTimes(1);
    });

    it("asks the capability store about everything the widgets require", async () => {
        render(() => <ToolchainView />);
        await waitFor(() => expect(h.ensureCapability).toHaveBeenCalledWith("python"));
        const ids = h.ensureCapability.mock.calls.map(([id]) => id);
        for (const id of ["python", "node", "npm", "docker"]) expect(ids).toContain(id);
        expect(ids.filter((id) => id === "python")).toHaveLength(1);
    });

    it("follows the store: checking, available with its facts, then unavailable", async () => {
        await mount();
        expect(dot(card("docker"))).toHaveAccessibleName("Checking");
        h.setCap({ docker: { status: "checking" } });
        expect(dot(card("docker"))).toHaveAccessibleName("Checking");
        h.setCap({
            docker: { status: "available", version: "27.0.1", path: "/usr/local/bin/docker", source: "local_install" },
        });
        expect(dot(card("docker"))).toHaveAccessibleName("Installed");
        expect(card("docker").textContent).toContain("27.0.1");
        expect(card("docker").textContent).toContain("managed");
        open(card("docker"));
        expect(await screen.findByText("/usr/local/bin/docker")).toBeInTheDocument();
        h.setCap({ docker: { status: "unavailable" } });
        expect(dot(card("docker"))).toHaveAccessibleName("Not installed (optional)");
        expect(card("docker").textContent).not.toContain("27.0.1");
    });

    it("leaves Docker as checking while the store has not probed it", async () => {
        h.setCap({ docker: { status: "unknown" } });
        render(() => <ToolchainView />);
        expect(dot(card("docker"))).toHaveAccessibleName("Checking");
    });

    it("does not take any other tool's state from the store, even when it changes later", async () => {
        h.setCap({ git: { status: "unavailable" }, claude: { status: "unavailable" } });
        await mount();
        expect(dot(card("git"))).toHaveAccessibleName("Installed");
        h.setCap({
            git: { status: "checking" },
            claude: { status: "available", version: "9.9.9" },
            node: { status: "unavailable" },
        });
        expect(dot(card("git"))).toHaveAccessibleName("Installed");
        expect(dot(card("node"))).toHaveAccessibleName("Installed");
        expect(dot(card("claude"))).toHaveAccessibleName("Not found");
    });

    it("forces the capability lookup on Refresh", async () => {
        await mount();
        h.ensureCapability.mockClear();
        fireEvent.click(button(/Refresh/));
        await waitFor(() => expect(h.ensureCapability).toHaveBeenCalledWith("docker", { force: true }));
    });
});

describe("Refresh", () => {
    it("forgets the old answers, asks again for every row and widget, and forces the agent CLIs", async () => {
        h.installed.claude = { installed: true, version: "2.1.288" };
        await mount();
        expect(card("git").textContent).toContain("2.40.0");
        const resolves = h.rpc.ResolveCliCommand.mock.calls.length;
        const health = h.rpc.WidgetHealthCommand.mock.calls.length;
        const checks = h.checkProviderInstalled.mock.calls.length;
        let finish: (v: CliHit) => void = () => {};
        h.rpc.ResolveCliCommand.mockImplementation(() => new Promise<CliHit>((resolve) => (finish = resolve)));
        h.checkProviderInstalled.mockImplementation(() => new Promise(() => {}));
        fireEvent.click(button(/Refresh/));
        await waitFor(() => expect(h.rpc.ResolveCliCommand.mock.calls.length).toBeGreaterThan(resolves));
        expect(dot(card("git"))).toHaveAccessibleName("Checking");
        expect(card("git").textContent).not.toContain("2.40.0");
        open(card("git"));
        expect(await screen.findByText("Looking for Git on this machine…")).toBeInTheDocument();
        expect(screen.queryByText("Path")).toBeNull();
        expect(screen.queryByText("/usr/bin/git")).toBeNull();
        expect(dot(card("claude"))).toHaveAccessibleName("Checking");
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Checking");
        expect(h.rpc.WidgetHealthCommand.mock.calls.length).toBeGreaterThan(health - 1);
        expect(h.checkProviderInstalled.mock.calls.length).toBeGreaterThan(checks);
        expect(h.invalidateInstallCheck).toHaveBeenCalledWith("claude");
        expect(h.invalidateInstallCheck).toHaveBeenCalledTimes(getProviderList().length);
        finish({ cli_path: "/x" });
    });

    it("is busy while it runs and free again afterwards, even when a probe throws", async () => {
        await mount();
        const fails: ((e: Error) => void)[] = [];
        h.rpc.WidgetHealthCommand.mockImplementation(() => new Promise((_, reject) => fails.push(reject)));
        fireEvent.click(button(/Refresh/));
        await waitFor(() => expect(button(/Refresh/)).toBeDisabled());
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalled());
        for (const fail of fails) fail(new Error("down"));
        await waitFor(() => expect(button(/Refresh/)).not.toBeDisabled());
    });

    it("looks at the widgets again too, and waits for them", async () => {
        await mount();
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Not installed");
        h.health[8188] = { healthy: true };
        fireEvent.click(button(/Refresh/));
        await waitFor(() => expect(wcard("comfyui").textContent).toContain("Running on port 8188"));
        await waitFor(() => expect(button(/Refresh/)).not.toBeDisabled());
        h.health[8188] = { healthy: false };
        let finish: (v: { healthy: boolean }) => void = () => {};
        h.rpc.WidgetHealthCommand.mockImplementation(() => new Promise((resolve) => (finish = resolve)));
        fireEvent.click(button(/Refresh/));
        await waitFor(() => expect(button(/Refresh/)).toBeDisabled());
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalled());
        await Promise.resolve();
        expect(button(/Refresh/)).toBeDisabled();
        finish({ healthy: false });
    });
});

describe("a signal from the install check", () => {
    it("re-asks for one agent CLI when its id is named, and for all of them when none is", async () => {
        await mount();
        h.checkProviderInstalled.mockClear();
        h.rpc.ResolveCliCommand.mockClear();
        h.invalidated!("claude");
        await waitFor(() => expect(h.checkProviderInstalled).toHaveBeenCalledTimes(1));
        expect(h.checkProviderInstalled.mock.calls[0][0].providerId).toBe("claude");
        h.checkProviderInstalled.mockClear();
        h.invalidated!(null);
        await waitFor(() => expect(h.checkProviderInstalled.mock.calls.length).toBeGreaterThan(10));
        h.checkProviderInstalled.mockClear();
        h.invalidated!(undefined);
        await waitFor(() => expect(h.checkProviderInstalled.mock.calls.length).toBeGreaterThan(10));
        expect(h.rpc.ResolveCliCommand).not.toHaveBeenCalled();
    });

    it("leaves the core tools alone when the id is not an agent CLI's", async () => {
        await mount();
        h.checkProviderInstalled.mockClear();
        h.rpc.ResolveCliCommand.mockClear();
        h.invalidated!("git");
        await Promise.resolve();
        expect(h.checkProviderInstalled).not.toHaveBeenCalled();
        expect(h.rpc.ResolveCliCommand).not.toHaveBeenCalled();
    });
});

describe("the latest versions", () => {
    it("asks for the npm packages of the agent CLIs that have one, on its own once", async () => {
        h.versions = { claude: "3.0.0" };
        await mount();
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalledTimes(1));
        const [, req, opts] = h.rpc.ToolchainVersionsCommand.mock.calls[0];
        expect(opts).toEqual({ timeout: 15000 });
        const ids = req.packages.map((p: { id: string }) => p.id);
        expect(ids).toContain("claude");
        expect(ids).not.toContain("kimi");
        expect(ids).not.toContain("git");
        expect(req.packages).toContainEqual({ id: "claude", package: "@anthropic-ai/claude-code" });
        expect(req.packages.every((p: { package: string }) => p.package)).toBe(true);
    });

    it("notes a pin that trails upstream on an agent CLI at its pin", async () => {
        h.installed.claude = { installed: true, version: "2.1.288" };
        h.versions = { claude: "3.0.0" };
        await mount();
        await waitFor(() => expect(card("claude").textContent).toContain("pin 2.1.288 · upstream 3.0.0"));
        expect(card("codex").textContent).not.toContain("upstream");
    });

    it("takes a later answer from the cache while it is fresh, without asking again", async () => {
        h.versions = { claude: "3.0.0" };
        h.installed.claude = { installed: true, version: "2.1.288" };
        const first = await mount();
        await waitFor(() => expect(card("claude").textContent).toContain("upstream 3.0.0"));
        first.unmount();
        cleanup();
        h.rpc.ToolchainVersionsCommand.mockClear();
        h.rpc.ToolchainVersionsCommand.mockResolvedValue({});
        await mount();
        await waitFor(() => expect(card("claude").textContent).toContain("upstream 3.0.0"));
        expect(h.rpc.ToolchainVersionsCommand).not.toHaveBeenCalled();
    });

    it("asks again once the cache is old, and when a manual check is asked for", async () => {
        h.versions = { claude: "3.0.0" };
        const first = await mount();
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalledTimes(1));
        first.unmount();
        cleanup();
        h.rpc.ToolchainVersionsCommand.mockClear();
        vi.setSystemTime(clock + 7 * HOUR);
        clock += 7 * HOUR;
        await mount();
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalledTimes(1));
        fireEvent.click(button(/Check latest versions/));
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalledTimes(2));
    });

    it("asks again while the cache holds nothing, and counts a null answer as nothing", async () => {
        vi.resetModules();
        const { render: freshRender } = await import("@solidjs/testing-library");
        const { ToolchainView: Fresh } = await import("./toolchain-view");
        const mountFresh = async () => {
            const view = freshRender(() => <Fresh />);
            await waitFor(() => expect(within(card("git")).queryByRole("img", { name: "Checking" })).toBeNull());
            await waitFor(() => expect(within(card("claude")).queryByRole("img", { name: "Checking" })).toBeNull());
            view.unmount();
        };
        const asks = () => h.rpc.ToolchainVersionsCommand.mock.calls.length;
        h.versions = {};
        await mountFresh();
        expect(asks()).toBe(1);
        await mountFresh();
        expect(asks()).toBe(2);
        h.versions = { claude: null };
        await mountFresh();
        await mountFresh();
        expect(asks()).toBe(4);
        h.versions = { claude: "3.0.0" };
        await mountFresh();
        expect(asks()).toBe(5);
        await mountFresh();
        expect(asks()).toBe(5);
    });

    it("says nothing when the lookup fails, and the button is free again", async () => {
        h.rpc.ToolchainVersionsCommand.mockRejectedValue(new Error("offline"));
        await mount();
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalled());
        await waitFor(() => expect(button(/Check latest versions/)).not.toBeDisabled());
        expect(screen.queryByText(/offline/)).toBeNull();
    });

    it("shows the button busy while the check runs, and has a hint on it", async () => {
        h.rpc.ToolchainVersionsCommand.mockResolvedValue({});
        await mount();
        let finish: (v: Record<string, string>) => void = () => {};
        h.rpc.ToolchainVersionsCommand.mockImplementation(() => new Promise((resolve) => (finish = resolve)));
        const check = button(/Check latest versions/);
        expect(check).toHaveAttribute("title", "Fetch latest published versions from npm registry");
        fireEvent.click(check);
        await waitFor(() => expect(button(/Check latest versions/)).toBeDisabled());
        finish({});
        await waitFor(() => expect(button(/Check latest versions/)).not.toBeDisabled());
    });

    it("does not give a tool without a package a latest version", async () => {
        h.installed.goose = { installed: true, version: "1.54.0" };
        h.installed.claude = { installed: true, version: "2.1.288" };
        h.versions = { goose: "9.9.9", claude: "3.0.0" };
        await mount();
        await waitFor(() => expect(card("claude").textContent).toContain("upstream 3.0.0"));
        expect(card("goose").textContent).not.toContain("9.9.9");
        expect(card("goose").textContent).toContain("up to date");
    });

    it("keeps a latest version a later check does not mention", async () => {
        h.installed.claude = { installed: true, version: "2.1.288" };
        h.versions = { claude: "3.0.0" };
        await mount();
        await waitFor(() => expect(card("claude").textContent).toContain("upstream 3.0.0"));
        h.versions = {};
        h.rpc.ToolchainVersionsCommand.mockClear();
        fireEvent.click(button(/Check latest versions/));
        await waitFor(() => expect(h.rpc.ToolchainVersionsCommand).toHaveBeenCalledTimes(1));
        await waitFor(() => expect(button(/Check latest versions/)).not.toBeDisabled());
        expect(card("claude").textContent).toContain("upstream 3.0.0");
    });
});

describe("the count of tools that are behind", () => {
    it("is absent while nothing is behind its pin, including tools not found", async () => {
        h.installed.claude = { installed: true, version: "2.1.288" };
        await mount();
        expect(screen.queryByText(/behind/)).toBeNull();
    });

    it("counts only what the user can act on, singular then plural, with a hint", async () => {
        h.installed.claude = { installed: true, version: "1.0.0" };
        await mount();
        const one = await screen.findByText("1 tool is behind");
        expect(one.closest(".ui-pill")).toHaveAttribute("title", expect.stringContaining("offers Update"));
        expect(dot(card("claude"))).toHaveAccessibleName("Update available: 2.1.288");
        expect(card("claude").textContent).toContain("2.1.288 available");
        cleanup();
        h.installed.codex = { installed: true, version: "0.1.0" };
        await mount();
        expect(await screen.findByText("2 tools are behind")).toBeInTheDocument();
    });

    it("does not count a row that is still being looked up", async () => {
        h.checkProviderInstalled.mockImplementation(() => new Promise(() => {}));
        render(() => <ToolchainView />);
        expect(screen.queryByText(/behind/)).toBeNull();
    });
});

describe("updating a tool that is behind its pin", () => {
    it("offers the update in the details of an installed npm CLI that is behind, to the pin", async () => {
        h.installed.codex = { installed: true, version: "0.0.1" };
        await mount();
        await screen.findByText("1 tool is behind");
        expect(screen.queryByTestId("provider-update")).toBeNull();
        fireEvent.click(card("codex"));
        const update = await screen.findByTestId("provider-update");
        expect(update.textContent).toMatch(/^codex -> \d/);
    });

    it("offers nothing for a tool that is current or not installed", async () => {
        h.installed.claude = { installed: true, version: "2.1.288" };
        await mount();
        fireEvent.click(card("claude"));
        fireEvent.click(card("codex"));
        expect(screen.queryByTestId("provider-update")).toBeNull();
    });

    it("names a card in its tooltip, since its title can be cut short", async () => {
        await mount();
        expect(card("claude")).toHaveAttribute("title", expect.stringMatching(/Claude/));
    });
});

describe("a card's actions and details", () => {
    it("opens the install page of a missing tool, and its docs", async () => {
        await mount();
        fireEvent.click(within(card("python")).getByRole("button", { name: "Install" }));
        expect(h.openExternal).toHaveBeenLastCalledWith("https://www.python.org/downloads/macos/");
        fireEvent.click(within(card("python")).getByRole("button", { name: "Docs" }));
        expect(h.openExternal).toHaveBeenLastCalledWith("https://www.python.org/downloads/");
        expect(within(card("git")).queryByRole("button", { name: "Install" })).toBeNull();
        fireEvent.click(within(card("git")).getByRole("button", { name: "Docs" }));
        expect(h.openExternal).toHaveBeenLastCalledWith("https://git-scm.com/");
    });

    it("sends a missing agent CLI to its docs page, which is also its install page", async () => {
        await mount();
        fireEvent.click(within(card("codex")).getByRole("button", { name: "Install" }));
        expect(h.openExternal).toHaveBeenLastCalledWith("https://platform.openai.com/docs/codex");
        fireEvent.click(within(card("codex")).getByRole("button", { name: "Docs" }));
        expect(h.openExternal).toHaveBeenCalledTimes(2);
        expect(h.openExternal).toHaveBeenLastCalledWith("https://platform.openai.com/docs/codex");
    });

    it("shows where an installed tool lives, and nothing of the sort for a missing one", async () => {
        await mount();
        open(card("python"));
        await screen.findByText("Not found on this machine's PATH.");
        expect(screen.queryByText("Path")).toBeNull();
        open(card("git"));
        expect(await screen.findByText("Path")).toBeInTheDocument();
        expect(screen.queryByText("Not found on this machine's PATH.")).toBeNull();
    });

    it("lists a card's longer notes in its details, and only those that have one", async () => {
        h.installed.claude = { installed: true, version: "2.1.288", path: "/c", source: "system_path" };
        await mount();
        open(card("claude"));
        expect(
            await screen.findByText("AgentMux doesn't install this CLI: it runs the copy you installed")
        ).toBeInTheDocument();
        expect(document.querySelectorAll(".toolchain-detail-note")).toHaveLength(1);
        const note = within(card("claude")).getByText("your install");
        expect(note).toHaveAttribute("data-hover-tip", expect.stringContaining("AgentMux doesn't install"));
        expect(within(card("claude")).getByText("up to date")).not.toHaveAttribute("data-hover-tip");
    });

    it("shows the install command of a missing tool and copies it", async () => {
        await mount();
        open(card("python"));
        expect(await screen.findByText("brew install python@3.12")).toBeInTheDocument();
        fireEvent.click(screen.getByRole("button", { name: "Copy command" }));
        expect(h.clipboard).toHaveBeenCalledWith("brew install python@3.12");
    });

    it("shows a missing agent CLI's own installer only when it is not installed through npm", async () => {
        await mount();
        open(card("aider"));
        expect(await screen.findByText("curl -LsSf https://aider.chat/install.sh | sh")).toBeInTheDocument();
        open(card("claude"));
        await screen.findByText("Not found on this machine's PATH.");
        expect(screen.queryByText("Install command")).toBeNull();
        open(card("kimi"));
        expect(await screen.findByText("pip install kimi-cli")).toBeInTheDocument();
    });

    it("uses the Windows installer on Windows", async () => {
        h.platform = "win32";
        await mount();
        open(card("aider"));
        expect(await screen.findByText("irm https://aider.chat/install.ps1 | iex")).toBeInTheDocument();
        open(card("python"));
        expect(await screen.findByText(/winget install/)).toBeInTheDocument();
    });

    it("shows no install command for a tool that has none", async () => {
        h.setCap({ docker: { status: "unavailable" } });
        await mount();
        open(card("docker"));
        await screen.findByText("Not found on this machine's PATH.");
        expect(screen.queryByText("Install command")).toBeNull();
        expect(screen.queryByRole("button", { name: "Copy command" })).toBeNull();
    });

    it("copies nothing when there is nothing to copy", async () => {
        await mount();
        open(card("python"));
        fireEvent.click(await screen.findByRole("button", { name: "Copy command" }));
        expect(h.clipboard).toHaveBeenCalledTimes(1);
    });

    it("survives a clipboard that refuses", async () => {
        h.clipboardFails = true;
        await mount();
        open(card("python"));
        fireEvent.click(await screen.findByRole("button", { name: "Copy command" }));
        await new Promise((resolve) => setTimeout(resolve, 20));
        expect(h.clipboard).toHaveBeenCalledTimes(1);
    });
});

describe("installing a missing tool from the card", () => {
    it("offers it for Git, Node, npm and Python only, and only to a core tool that is missing", async () => {
        h.cli = {};
        await mount();
        for (const id of ["git", "node", "npm", "python"]) {
            open(card(id));
            expect(await screen.findByRole("button", { name: "Install it for me" })).toBeInTheDocument();
        }
        for (const id of ["uv", "ollama", "claude"]) {
            open(card(id));
            await screen.findByText("Not found on this machine's PATH.");
            expect(screen.queryByRole("button", { name: "Install it for me" })).toBeNull();
        }
    });

    it("does not offer it for a tool that is found", async () => {
        await mount();
        open(card("git"));
        await screen.findByText("Path");
        expect(screen.queryByRole("button", { name: "Install it for me" })).toBeNull();
    });

    it("opens and closes the panel for that tool", async () => {
        await mount();
        open(card("python"));
        const toggle = await screen.findByRole("button", { name: "Install it for me" });
        expect(toggle).toHaveAttribute("aria-pressed", "false");
        expect(screen.queryByTestId("install-inline")).toBeNull();
        fireEvent.click(toggle);
        expect(screen.getByTestId("install-inline")).toHaveAttribute("data-tool", "python");
        expect(toggle).toHaveAttribute("aria-pressed", "true");
        fireEvent.click(toggle);
        expect(screen.queryByTestId("install-inline")).toBeNull();
    });

    it("closes the panel and looks again, forcing it, when the tool gets installed", async () => {
        await mount();
        open(card("python"));
        fireEvent.click(await screen.findByRole("button", { name: "Install it for me" }));
        h.cli.python = { version: "3.12.0", cli_path: "/usr/bin/python3", source: "system_path" };
        h.rpc.ResolveCliCommand.mockClear();
        h.install.onInstalled();
        expect(screen.queryByTestId("install-inline")).toBeNull();
        await waitFor(() => expect(dot(card("python"))).toHaveAccessibleName("Installed"));
        expect(h.rpc.ResolveCliCommand).toHaveBeenCalledTimes(1);
        expect(h.rpc.ResolveCliCommand.mock.calls[0][1].provider_id).toBe("python");
    });

    it("closes the panel and withdraws the offer when the machine cannot install it", async () => {
        h.cli = {};
        await mount();
        open(card("python"));
        fireEvent.click(await screen.findByRole("button", { name: "Install it for me" }));
        h.install.onUnavailable();
        expect(screen.queryByTestId("install-inline")).toBeNull();
        expect(screen.queryByRole("button", { name: "Install it for me" })).toBeNull();
        open(card("node"));
        expect(await screen.findByRole("button", { name: "Install it for me" })).toBeInTheDocument();
    });

    it("keeps one tool's panel open apart from another's", async () => {
        h.cli = {};
        await mount();
        open(card("node"));
        fireEvent.click(await screen.findByRole("button", { name: "Install it for me" }));
        open(card("git"));
        fireEvent.click(await screen.findByRole("button", { name: "Install it for me" }));
        expect(screen.getAllByTestId("install-inline").map((e) => e.getAttribute("data-tool"))).toContain("git");
        open(card("node"));
        expect(screen.getByTestId("install-inline")).toHaveAttribute("data-tool", "node");
    });
});

describe("the filter", () => {
    it("matches a tool by its name, its id, or its version", async () => {
        await mount();
        fireEvent.input(filterBox(), { target: { value: "llama-cpp" } });
        expect(card("llama-cpp")).toBeTruthy();
        expect(document.querySelector('[data-tool="ollama"]')).toBeNull();
        fireEvent.input(filterBox(), { target: { value: "2.40" } });
        expect(card("git")).toBeTruthy();
        expect(document.querySelector('[data-tool="node"]')).toBeNull();
        fireEvent.input(filterBox(), { target: { value: "llama.cpp" } });
        expect(card("llama-cpp")).toBeTruthy();
    });

    it("matches a widget by its name, its id, or its description, and not a tool by a widget's words", async () => {
        await mount();
        fireEvent.input(filterBox(), { target: { value: "comfyui" } });
        expect(wcard("comfyui")).toBeTruthy();
        fireEvent.input(filterBox(), { target: { value: "open-webui" } });
        expect(wcard("open-webui")).toBeTruthy();
        fireEvent.input(filterBox(), { target: { value: "ollama and openai" } });
        expect(wcard("open-webui")).toBeTruthy();
        expect(document.querySelector('[data-tool="ollama"]')).toBeNull();
    });

    it("keeps the empty note away while any group still has a match", async () => {
        await mount();
        for (const q of ["lmstudio", "claude", "comfyui", "git"]) {
            fireEvent.input(filterBox(), { target: { value: q } });
            expect(screen.queryByText(/No tools match/)).toBeNull();
        }
        fireEvent.input(filterBox(), { target: { value: "zzzz" } });
        expect(screen.getByText("No tools match “zzzz”.")).toBeInTheDocument();
        fireEvent.input(filterBox(), { target: { value: "" } });
        expect(screen.queryByText(/No tools match/)).toBeNull();
    });

    it("shows only the headings of groups with a match", async () => {
        await mount();
        fireEvent.input(filterBox(), { target: { value: "lmstudio" } });
        expect(screen.getByRole("heading", { name: "Local models" })).toBeInTheDocument();
        for (const name of ["Core tools", "Agent CLIs", "External widgets"]) {
            expect(screen.queryByRole("heading", { name })).toBeNull();
        }
        expect(screen.queryByRole("group", { name: "Core tools" })).toBeNull();
        expect(screen.getByRole("group", { name: "Local models" })).toBeInTheDocument();
    });

    it("lets Escape in a card's details close them", async () => {
        await mount();
        open(card("python"));
        const note = await screen.findByText("Not found on this machine's PATH.");
        expect(card("python")).toHaveAttribute("aria-expanded", "true");
        fireEvent.keyDown(note, { key: "Escape" });
        expect(card("python")).toHaveAttribute("aria-expanded", "false");
    });
});

describe("the widgets", () => {
    it("checks each widget at its port, asking for the body it expects", async () => {
        await mount();
        const asked = h.rpc.WidgetHealthCommand.mock.calls.map(([, d]) => d);
        expect(asked).toContainEqual({
            port: 8188,
            health_check_path: "/system_stats",
            health_check_body_contains: "devices",
        });
        expect(h.rpc.WidgetHealthCommand.mock.calls[0][2]).toEqual({ timeout: 5000 });
        const probes = h.rpc.ResolveCliCommand.mock.calls.filter(([, d]) => d.provider_id === "comfyui");
        expect(probes).toHaveLength(1);
        expect(probes[0][1]).toMatchObject({ provider_id: "comfyui", cli_command: "comfyui" });
        expect(probes[0][2]).toEqual({ timeout: 10000 });
    });

    it("says whether a widget is running, installed, or neither", async () => {
        h.health[8188] = { healthy: true, status_code: 200 };
        h.cli.jupyter = { cli_path: "/j" };
        h.cli.n8n = { cli_path: "/n" };
        await mount();
        expect(wcard("comfyui").textContent).toContain("Running on port 8188");
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Running");
        expect(dot(wcard("jupyterlab"))).toHaveAccessibleName("Not installed");
        expect(dot(wcard("grafana"))).toHaveAccessibleName("Not detected");
        h.cli.n8n = { cli_path: "/n" };
    });

    it("shows a widget with no command to look for as not detected, without looking", async () => {
        await mount();
        expect(dot(wcard("portainer"))).toHaveAccessibleName("Not detected");
        expect(h.rpc.ResolveCliCommand.mock.calls.map(([, d]) => d.provider_id)).not.toContain("portainer");
        expect(h.rpc.WidgetHealthCommand.mock.calls.map(([, d]) => d.port)).toContain(9000);
    });

    it("looks for a widget's command by the name it has on this platform", async () => {
        h.platform = "win32";
        await mount();
        const asked = h.rpc.ResolveCliCommand.mock.calls.map(([, d]) => [d.provider_id, d.cli_command]);
        expect(asked).toContainEqual(["flowise", "flowise.cmd"]);
        expect(asked).toContainEqual(["jupyterlab", "jupyter"]);
    });

    it("shows a widget as checking until its first answers", async () => {
        h.rpc.ResolveCliCommand.mockImplementation(() => new Promise(() => {}));
        render(() => <ToolchainView />);
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Checking");
        expect(wcard("comfyui").textContent).toContain("Checking…");
    });

    it("shows a widget whose CLI is found but is not answering as installed, not running", async () => {
        h.cli.comfyui = { cli_path: "/c" };
        await mount();
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Installed, not running");
    });

    it("shows a widget whose health check throws as not running", async () => {
        h.rpc.WidgetHealthCommand.mockRejectedValue(new Error("refused"));
        await mount();
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Not installed");
    });

    it("opens a running widget in a browser pane at its address, and not one that is not running", async () => {
        h.health[8188] = { healthy: true };
        h.health[6333] = { healthy: true };
        await mount();
        expect(within(wcard("jupyterlab")).queryByRole("button", { name: "Open pane" })).toBeNull();
        fireEvent.click(within(wcard("comfyui")).getByRole("button", { name: "Open pane" }));
        expect(h.createBlock).toHaveBeenLastCalledWith({
            meta: { view: "browser", url: "http://127.0.0.1:8188/", "frame:title": "ComfyUI" },
        });
        fireEvent.click(within(wcard("qdrant")).getByRole("button", { name: "Open pane" }));
        expect(h.createBlock).toHaveBeenLastCalledWith({
            meta: { view: "browser", url: "http://127.0.0.1:6333/dashboard", "frame:title": "Qdrant" },
        });
    });

    it("opens a widget's docs", async () => {
        await mount();
        fireEvent.click(within(wcard("comfyui")).getByRole("button", { name: "Docs" }));
        expect(h.openExternal).toHaveBeenCalledWith("https://docs.comfy.org/");
    });

    it("describes a widget, and says what it still needs while it is not running", async () => {
        h.available = new Set(["node"]);
        await mount();
        open(wcard("flowise"));
        expect(await screen.findByText(/Visual LangChain/)).toBeInTheDocument();
        expect(screen.getByText("Requires: npm")).toBeInTheDocument();
        open(wcard("comfyui"));
        expect(await screen.findByText("Requires: python")).toBeInTheDocument();
        open(wcard("grafana"));
        await screen.findByText(/Metrics and observability/);
        expect(screen.queryByText(/Requires:/)).toBeNull();
    });

    it("does not list requirements that are met, or for a widget that is running", async () => {
        h.available = new Set(["python"]);
        h.health[8188] = { healthy: true };
        await mount();
        open(wcard("jupyterlab"));
        await screen.findByText(/notebook/i);
        expect(screen.queryByText(/Requires:/)).toBeNull();
        h.available = new Set();
        open(wcard("comfyui"));
        await screen.findByText(/Node-based generative/);
        expect(screen.queryByText(/Requires:/)).toBeNull();
    });

    it("starts from the port saved in the browser, and checks the widget there", async () => {
        localStorage.setItem("agentmux:widget-ports", JSON.stringify({ comfyui: 9001 }));
        h.health[9001] = { healthy: true };
        await mount();
        expect(h.rpc.WidgetHealthCommand.mock.calls.map(([, d]) => d.port)).toContain(9001);
        expect(wcard("comfyui").textContent).toContain("Running on port 9001");
        open(wcard("comfyui"));
        expect(await screen.findByLabelText("Port")).toHaveValue("9001");
        expect(screen.getByRole("button", { name: "Reset to the default port" })).toBeInTheDocument();
        fireEvent.click(within(wcard("comfyui")).getByRole("button", { name: "Open pane" }));
        expect(h.createBlock).toHaveBeenLastCalledWith({
            meta: expect.objectContaining({ url: "http://127.0.0.1:9001/" }),
        });
    });

    it("ignores a saved port list it cannot read", async () => {
        localStorage.setItem("agentmux:widget-ports", "{nope");
        await mount();
        open(wcard("comfyui"));
        expect(await screen.findByLabelText("Port")).toHaveValue("8188");
        expect(screen.queryByRole("button", { name: "Reset to the default port" })).toBeNull();
    });

    it("shows the default port and its hint, with no reset button", async () => {
        await mount();
        open(wcard("comfyui"));
        expect(await screen.findByLabelText("Port")).toHaveValue("8188");
        expect(screen.getByText("8188 unless you change it.")).toBeInTheDocument();
        expect(screen.queryByRole("button", { name: "Reset to the default port" })).toBeNull();
    });

    async function applyPort(value: string) {
        const port = await screen.findByLabelText("Port");
        fireEvent.input(port, { target: { value } });
        fireEvent.keyDown(port, { key: "Enter" });
    }

    it("saves a changed port, looks at the widget again there, and shows it checking meanwhile", async () => {
        await mount();
        open(wcard("comfyui"));
        h.rpc.WidgetHealthCommand.mockClear();
        let finish: (v: { healthy: boolean }) => void = () => {};
        h.rpc.WidgetHealthCommand.mockImplementation(() => new Promise((resolve) => (finish = resolve)));
        let found: (v: CliHit) => void = () => {};
        h.rpc.ResolveCliCommand.mockImplementation(() => new Promise<CliHit>((resolve) => (found = resolve)));
        await applyPort("9123");
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Checking");
        found({ cli_path: "/c" });
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalledTimes(1));
        expect(JSON.parse(localStorage.getItem("agentmux:widget-ports")!)).toEqual({ comfyui: 9123 });
        expect(dot(wcard("comfyui"))).toHaveAccessibleName("Checking");
        finish({ healthy: true });
        await waitFor(() => expect(wcard("comfyui").textContent).toContain("Running on port 9123"));
    });

    it("treats the default port as no change of port, and the same port twice as nothing to do", async () => {
        await mount();
        open(wcard("comfyui"));
        h.rpc.WidgetHealthCommand.mockClear();
        await applyPort("8188");
        expect(localStorage.getItem("agentmux:widget-ports")).toBeNull();
        expect(h.rpc.WidgetHealthCommand).not.toHaveBeenCalled();
        await applyPort("9123");
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalledTimes(1));
        h.rpc.WidgetHealthCommand.mockClear();
        await applyPort("9123");
        expect(h.rpc.WidgetHealthCommand).not.toHaveBeenCalled();
    });

    it("takes a typed default port as going back to the default", async () => {
        localStorage.setItem("agentmux:widget-ports", JSON.stringify({ comfyui: 9001, n8n: 5999 }));
        await mount();
        open(wcard("comfyui"));
        h.rpc.WidgetHealthCommand.mockClear();
        await applyPort("8188");
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalledTimes(1));
        expect(h.rpc.WidgetHealthCommand.mock.calls[0][1].port).toBe(8188);
        expect(JSON.parse(localStorage.getItem("agentmux:widget-ports")!)).toEqual({ n8n: 5999 });
        expect(screen.queryByRole("button", { name: "Reset to the default port" })).toBeNull();
    });

    it("puts the default back from the reset button, forgetting the saved one", async () => {
        localStorage.setItem("agentmux:widget-ports", JSON.stringify({ comfyui: 9001, n8n: 5999 }));
        await mount();
        open(wcard("comfyui"));
        h.rpc.WidgetHealthCommand.mockClear();
        fireEvent.click(await screen.findByRole("button", { name: "Reset to the default port" }));
        await waitFor(() => expect(h.rpc.WidgetHealthCommand).toHaveBeenCalledTimes(1));
        expect(h.rpc.WidgetHealthCommand.mock.calls[0][1].port).toBe(8188);
        expect(JSON.parse(localStorage.getItem("agentmux:widget-ports")!)).toEqual({ n8n: 5999 });
    });
});

describe("the environment", () => {
    it("says it is loading until the answer comes, and stays so if there is none", async () => {
        h.rpc.ToolchainEnvCommand.mockRejectedValue(new Error("no"));
        await mount();
        expect(screen.getByText("Loading…")).toBeInTheDocument();
        expect(screen.queryByText("Platform")).toBeNull();
    });

    it("asks for it with a timeout", async () => {
        await mount();
        expect(h.rpc.ToolchainEnvCommand).toHaveBeenCalledWith({}, { timeout: 8000 });
    });

    it("shows the platform and where the PATH came from", async () => {
        h.rpc.ToolchainEnvCommand.mockResolvedValue({
            path: "/a:/b",
            pathSource: "fallback-dirs",
            os: "macos",
            arch: "arm64",
        });
        await mount();
        expect(await screen.findByText("macos · arm64")).toBeInTheDocument();
        const pill = screen.getByText("from well-known dirs (login shell unavailable)");
        expect(pill.className).toContain("ui-pill--neutral");
    });

    it("names the login shell, and the inherited PATH, with the tone each deserves", async () => {
        await mount();
        const ok = await screen.findByText("from your login shell");
        expect(ok.className).toContain("ui-pill--ok");
        cleanup();
        h.rpc.ToolchainEnvCommand.mockResolvedValue({ path: "/a", pathSource: "inherited", os: "linux", arch: "x64" });
        await mount();
        const warn = await screen.findByText("inherited from the launching process");
        expect(warn.className).toContain("ui-pill--warn");
        cleanup();
        h.rpc.ToolchainEnvCommand.mockResolvedValue({ path: "/a", pathSource: "other", os: "linux", arch: "x64" });
        await mount();
        expect((await screen.findByText("inherited from the launching process")).className).toContain(
            "ui-pill--neutral"
        );
    });

    it("does not warn about an inherited PATH on Windows", async () => {
        h.platform = "win32";
        h.rpc.ToolchainEnvCommand.mockResolvedValue({
            path: "C:\\a",
            pathSource: "inherited",
            os: "windows",
            arch: "x64",
        });
        await mount();
        const pill = await screen.findByText("inherited from the launching process");
        expect(pill.className).toContain("ui-pill--neutral");
    });

    it("shows and hides the PATH, one entry to a line", async () => {
        await mount();
        const toggle = await screen.findByRole("button", { name: "Show effective PATH" });
        expect(document.querySelector(".toolchain-path-dump")).toBeNull();
        fireEvent.click(toggle);
        expect(document.querySelector(".toolchain-path-dump")!.textContent).toBe("/bin\n/usr/bin");
        fireEvent.click(screen.getByRole("button", { name: "Hide effective PATH" }));
        expect(document.querySelector(".toolchain-path-dump")).toBeNull();
        expect(screen.getByRole("button", { name: "Show effective PATH" })).toBeInTheDocument();
    });

    it("splits a Windows PATH on semicolons", async () => {
        h.rpc.ToolchainEnvCommand.mockResolvedValue({
            path: "C:\\a;C:\\b:c",
            pathSource: "login-shell",
            os: "windows",
            arch: "x64",
        });
        await mount();
        fireEvent.click(await screen.findByRole("button", { name: "Show effective PATH" }));
        expect(document.querySelector(".toolchain-path-dump")!.textContent).toBe("C:\\a\nC:\\b:c");
    });
});

describe("the view", () => {
    it("is not zoomed unless the pane is", async () => {
        const { container } = await mount();
        expect((container.querySelector(".toolchain-view") as HTMLElement).style.zoom).toBe("1");
    });

    it("shows the disk-space section and the four groups in order", async () => {
        await mount();
        expect(screen.getByTestId("cli-disk-space")).toBeInTheDocument();
        const headings = screen.getAllByRole("heading", { level: 4 }).map((el) => el.textContent);
        expect(headings).toEqual(["Environment", "Core tools", "Local models", "Agent CLIs", "External widgets"]);
        const core = within(screen.getByRole("group", { name: "Core tools" }));
        expect(core.getAllByRole("button", { name: /,/ }).map((b) => b.getAttribute("data-tool"))).toEqual([
            "node",
            "npm",
            "git",
            "docker",
            "python",
            "uv",
        ]);
        const local = within(screen.getByRole("group", { name: "Local models" }));
        expect(local.getAllByRole("button", { name: /,/ }).map((b) => b.getAttribute("data-tool"))).toEqual([
            "ollama",
            "llama-cpp",
            "lmstudio",
            "llmfit",
        ]);
    });

    it("types into the filter", async () => {
        await mount();
        fireEvent.input(filterBox(), { target: { value: "gi" } });
        expect((filterBox() as HTMLInputElement).value).toBe("gi");
    });
});
