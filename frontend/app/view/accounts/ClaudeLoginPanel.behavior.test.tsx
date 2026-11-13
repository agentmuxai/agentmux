// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * ClaudeLoginPanel, pinned as a whole before it was split into a state object, a session runner and view parts:
 * each scenario drives the sign-in (the phases the login reports, the terminal fallback, every way it ends, the
 * link check, the stale-alias cleanup, Cancel, Retry, and unmounting mid-login) and logs, in order, the whole
 * page as a tree, every outward call and every option the login was started with. The logs are snapshots
 * recorded on the unsplit component; the split must leave them byte for byte.
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const hub = vi.hoisted(() => ({
    runProviderLogin: vi.fn(),
    resolveCliCommand: vi.fn(),
    unlinkAgentIdentity: vi.fn(),
    listAgentIdentities: vi.fn(),
    ensureAuthDir: vi.fn(),
    refreshAccountCache: vi.fn(),
    cancelCliLogin: vi.fn(),
}));

// The dialog's fade-out leaves a stand-in in the body for 120 ms; these snapshots are of the panel, not of it.
vi.mock("@/app/element/modal-exit-fade", () => ({ fadeOutBackdrop: () => {} }));
vi.mock("@/app/store/global", () => ({
    getApi: () => ({
        cancelCliLogin: (...args: unknown[]) => hub.cancelCliLogin(...args),
    }),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: {
        ResolveCliCommand: (...args: unknown[]) => hub.resolveCliCommand(...args),
        UnlinkAgentIdentityCommand: (...args: unknown[]) => hub.unlinkAgentIdentity(...args),
        ListAgentIdentitiesCommand: (...args: unknown[]) => hub.listAgentIdentities(...args),
    },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/view/agent/agent-launch-env", () => ({
    ensureProviderAuthDir: (...args: unknown[]) => hub.ensureAuthDir(...args),
}));
vi.mock("@/app/errors/translate", () => ({
    translateError: (e: any) => ({ title: "Error", message: String(e?.message ?? e), retry: e?.retry ?? "" }),
}));
vi.mock("@/app/view/agent/flows/run-provider-login", () => ({
    runProviderLogin: (...args: unknown[]) => hub.runProviderLogin(...args),
}));
vi.mock("@/app/view/agent/components/InAppLoginPanel", () => ({
    InAppLoginPanel: (props: any) => (
        <div
            data-testid="inapp"
            data-provider-id={props.providerId}
            data-provider-label={props.providerLabel}
            data-phase={props.phase}
            data-auth-url={props.authUrl ?? ""}
        >
            <button onClick={() => props.onCancel()}>inapp-cancel</button>
            <button onClick={() => props.onUseTerminal()}>inapp-terminal</button>
        </div>
    ),
}));
vi.mock("@/app/view/identity/identity-model", () => ({
    refreshAccountCache: (...args: unknown[]) => hub.refreshAccountCache(...args),
}));

import { ClaudeLoginPanel } from "./ClaudeLoginPanel";

const T: string[] = [];
const log = (line: string) => T.push(line);
const SNAP = (name: string) => `./__snapshots__/ClaudeLoginPanel.behavior.${name}.txt`;
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(SNAP(name));

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type)$/;

/** The page as an indented tree: tags, classes, the attributes that carry state, and text. */
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

const seen = new Map<string, number>();
const consoleLines: string[] = [];
const watched: [string, () => ReturnType<typeof vi.fn>][] = [
    ["ResolveCliCommand", () => hub.resolveCliCommand],
    ["ensureProviderAuthDir", () => hub.ensureAuthDir],
    ["ListAgentIdentities", () => hub.listAgentIdentities],
    ["UnlinkAgentIdentity", () => hub.unlinkAgentIdentity],
    ["refreshAccountCache", () => hub.refreshAccountCache],
    ["cancelCliLogin", () => hub.cancelCliLogin],
    ["onClose", () => handlers.onClose],
    ["onSignedIn", () => handlers.onSignedIn],
];
const handlers = { onClose: vi.fn(), onSignedIn: vi.fn() };

/** The calls made since the last look. */
function calls() {
    for (const [name, get] of watched) {
        const made = get().mock.calls;
        for (let i = seen.get(name) ?? 0; i < made.length; i++) log(`  call ${name} ${JSON.stringify(made[i])}`);
        seen.set(name, made.length);
    }
    const runs = hub.runProviderLogin.mock.calls;
    for (let i = seen.get("run") ?? 0; i < runs.length; i++) {
        const o = runs[i][0];
        log(
            `  run provider=${o.provider.id} cliPath=${o.cliPath} authEnv=${JSON.stringify(o.authEnv)} existingAccountId=${o.existingAccountId} linkTarget=${JSON.stringify(o.linkTarget)} skipTier1=${o.skipTier1} awaitTier1Completion=${o.awaitTier1Completion}`,
        );
    }
    seen.set("run", runs.length);
    for (let i = seen.get("console") ?? 0; i < consoleLines.length; i++) log(`  console ${consoleLines[i]}`);
    seen.set("console", consoleLines.length);
}

function page(label: string) {
    log(`== ${label}`);
    dump(document.body).forEach((l) => log(l));
    calls();
}

const flush = async () => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
    await new Promise((r) => setTimeout(r, 0));
    for (let i = 0; i < 10; i++) await Promise.resolve();
};

interface Run {
    opts: any;
    resolve: (outcome: string) => void;
    reject: (e: unknown) => void;
}
const runs: Run[] = [];
/** Each login attempt waits for the test to end it. */
function deferLogins() {
    runs.length = 0;
    hub.runProviderLogin.mockImplementation(
        (opts: any) =>
            new Promise<string>((resolve, reject) => {
                runs.push({ opts, resolve, reject });
            }),
    );
}
const registered = (run: Run, id = "acct-new") => run.opts.onAccountRegistered?.(id, `/tmp/${id}`);

type Props = Parameters<typeof ClaudeLoginPanel>[0];
function mount(props: Partial<Props> = {}) {
    return render(() => <ClaudeLoginPanel onClose={handlers.onClose} {...props} />);
}

beforeEach(() => {
    T.length = 0;
    seen.clear();
    consoleLines.length = 0;
    runs.length = 0;
    handlers.onClose.mockReset();
    handlers.onSignedIn.mockReset();
    hub.runProviderLogin.mockReset();
    hub.resolveCliCommand.mockReset().mockResolvedValue({ cli_path: "/usr/bin/claude" });
    hub.unlinkAgentIdentity.mockReset().mockResolvedValue({ unlinked: true });
    hub.listAgentIdentities.mockReset().mockResolvedValue([{ agent_id: "agent-1", account_id: "acct-new", provider: "claude" }]);
    hub.ensureAuthDir.mockReset().mockResolvedValue("/tmp/claude-auth");
    hub.refreshAccountCache.mockReset();
    hub.cancelCliLogin.mockReset().mockResolvedValue(undefined);
    vi.spyOn(console, "log").mockImplementation((...a) => void consoleLines.push(`log ${a.join(" ")}`));
    vi.spyOn(console, "warn").mockImplementation((...a) => void consoleLines.push(`warn ${a.join(" ")}`));
});
afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
});

describe("ClaudeLoginPanel behavior", () => {
    it("walks the phases of a sign-in and ends signed in, in a dialog", async () => {
        deferLogins();
        mount();
        page("starting: cli being resolved");
        await flush();
        page("login running");
        runs[0].opts.log("cat", "opened the browser");
        runs[0].opts.setAuthUrl("https://claude.test/authorize");
        await flush();
        page("link ready: waiting to authorize");
        runs[0].opts.setAuthUrl(null);
        await flush();
        page("link cleared");
        runs[0].opts.onTierChange({ tier: "fallback" });
        await flush();
        page("fallback");
        runs[0].opts.onTierChange({ tier: "terminal" });
        await flush();
        page("terminal polling");
        runs[0].opts.onTierChange({ tier: "inapp-waiting" });
        await flush();
        page("waiting to authorize again");
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("signed in");
        fireEvent.click(screen.getByText("Done"));
        calls();
        await snapshot("success");
    });

    it("signs in inline, and links the account to an agent, cleaning up the stale alias", async () => {
        deferLogins();
        mount({ inline: true, existingAccountId: "acc-1", linkTarget: { agentDefinitionId: "agent-1", blockId: "b1" }, staleAliasProvider: "claude-code" });
        await flush();
        page("inline, running");
        registered(runs[0], "acct-new");
        runs[0].resolve("terminal-success");
        await flush();
        page("inline, signed in");
        await snapshot("inline-link");
    });

    it("says why when the stale alias cannot be unlinked, and when the link cannot be confirmed", async () => {
        deferLogins();
        hub.unlinkAgentIdentity.mockRejectedValue(new Error("unlink refused"));
        mount({ linkTarget: { agentDefinitionId: "agent-1" }, staleAliasProvider: "claude-code" });
        await flush();
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("signed in despite the failed unlink");
        cleanup();
        deferLogins();
        hub.listAgentIdentities.mockResolvedValue([{ agent_id: "agent-1", account_id: "other", provider: "claude" }]);
        mount({ linkTarget: { agentDefinitionId: "agent-1" }, staleAliasProvider: "claude-code" });
        await flush();
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("link not confirmed: the stale alias is kept");
        cleanup();
        deferLogins();
        hub.listAgentIdentities.mockRejectedValue(new Error("list failed"));
        mount({ linkTarget: { agentDefinitionId: "agent-1" } });
        await flush();
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("link check failed");
        await snapshot("link-check");
    });

    it("ends with an error for each outcome that is not a sign-in, and Retry runs again with the account it made", async () => {
        deferLogins();
        mount();
        await flush();
        runs[0].resolve("inapp-timeout");
        await flush();
        page("link timed out");
        fireEvent.click(screen.getByText("Retry"));
        await flush();
        page("retry running");
        runs[1].resolve("terminal-timeout");
        await flush();
        page("terminal timed out");
        fireEvent.click(screen.getByText("Retry"));
        await flush();
        runs[2].resolve("terminal-unavailable");
        await flush();
        page("terminal unavailable");
        fireEvent.click(screen.getByText("Retry"));
        await flush();
        runs[3].resolve("inapp-success");
        await flush();
        page("completed but never registered");
        fireEvent.click(screen.getByText("Retry"));
        await flush();
        registered(runs[4], "acct-first");
        hub.listAgentIdentities.mockResolvedValue([]);
        runs[4].resolve("inapp-success");
        await flush();
        page("registered, link not confirmed (no target: signed in)");
        await snapshot("errors");
    });

    it("retries with the account a failed link check already created", async () => {
        deferLogins();
        hub.listAgentIdentities.mockResolvedValue([]);
        mount({ linkTarget: { agentDefinitionId: "agent-1" } });
        await flush();
        registered(runs[0], "acct-first");
        runs[0].resolve("inapp-success");
        await flush();
        page("link not confirmed");
        fireEvent.click(screen.getByText("Retry"));
        await flush();
        page("retrying with the account made");
        runs[1].resolve("inapp-timeout");
        await flush();
        fireEvent.click(screen.getByText("Close"));
        calls();
        await snapshot("retry-account");
    });

    it("shows a failure to resolve the CLI or to make its auth directory, and a login that rejects", async () => {
        deferLogins();
        hub.resolveCliCommand.mockRejectedValue(Object.assign(new Error("not installed"), { retry: "install it" }));
        mount();
        await flush();
        page("cli could not be resolved");
        cleanup();
        hub.resolveCliCommand.mockResolvedValue({ cli_path: "/usr/bin/claude" });
        hub.ensureAuthDir.mockRejectedValue(new Error("disk full"));
        mount();
        await flush();
        page("auth dir failed");
        cleanup();
        hub.ensureAuthDir.mockRejectedValue("plain string");
        mount();
        await flush();
        page("auth dir failed with a plain string");
        cleanup();
        hub.ensureAuthDir.mockResolvedValue("/tmp/claude-auth");
        deferLogins();
        mount();
        await flush();
        runs[0].reject(new Error("pty failed to spawn"));
        await flush();
        page("login rejected: cancelled, error shown");
        await snapshot("failures");
    });

    it("re-runs with the terminal when asked during the in-app tier, and not otherwise", async () => {
        deferLogins();
        mount();
        await flush();
        fireEvent.click(screen.getByText("inapp-terminal"));
        await flush();
        page("terminal asked for");
        runs[0].resolve("inapp-timeout");
        await flush();
        page("re-run with the terminal");
        registered(runs[1]);
        runs[1].resolve("terminal-success");
        await flush();
        page("signed in through the terminal");
        cleanup();
        deferLogins();
        mount();
        await flush();
        fireEvent.click(screen.getByText("inapp-terminal"));
        await flush();
        runs[0].opts.isCancelled();
        runs[0].resolve("terminal-timeout");
        await flush();
        page("terminal asked, but the outcome was not the in-app timeout");
        await snapshot("terminal");
    });

    it("cancels from the login panel, and when the panel goes away mid-login or after it", async () => {
        deferLogins();
        mount();
        await flush();
        fireEvent.click(screen.getByText("inapp-cancel"));
        await flush();
        page("cancelled by the person");
        runs[0].resolve("inapp-timeout");
        await flush();
        page("the cancelled run ends");
        cleanup();
        deferLogins();
        mount();
        await flush();
        cleanup();
        calls();
        log("-- unmounted mid-login");
        runs[0].resolve("inapp-success");
        await flush();
        calls();
        deferLogins();
        mount();
        await flush();
        runs[0].resolve("inapp-timeout");
        await flush();
        cleanup();
        log("-- unmounted after the login ended");
        calls();
        await snapshot("cancel");
    });

    it("keeps the dialog open mid-login, closes it by Escape or the backdrop once idle", async () => {
        deferLogins();
        mount({ scope: "pane" });
        await flush();
        page("pane scope, busy");
        const dialog = document.querySelector(".modal-panel, [role=dialog]") as HTMLElement;
        fireEvent.keyDown(dialog, { key: "Escape" });
        fireEvent.click(document.querySelector(".modal-backdrop")!);
        await flush();
        calls();
        runs[0].resolve("inapp-timeout");
        await flush();
        fireEvent.keyDown(dialog, { key: "Escape" });
        await flush();
        page("after Escape when not busy");
        fireEvent.click(document.querySelector(".modal-backdrop")!);
        await flush();
        calls();
        await snapshot("dialog");
    });

    it("words errors with and without a retry hint, and from values that are not errors", async () => {
        deferLogins();
        hub.resolveCliCommand.mockRejectedValue(new Error("no hint"));
        mount();
        await flush();
        page("no retry hint");
        cleanup();
        hub.resolveCliCommand.mockResolvedValue({ cli_path: "/usr/bin/claude" });
        deferLogins();
        mount();
        await flush();
        runs[0].reject(Object.assign(new Error("spawn failed"), { retry: "try again" }));
        await flush();
        page("login rejected with a retry hint");
        cleanup();
        deferLogins();
        hub.listAgentIdentities.mockRejectedValue("plain list failure");
        mount({ linkTarget: { agentDefinitionId: "agent-1" } });
        await flush();
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("link check failed with a plain string");
        cleanup();
        deferLogins();
        hub.listAgentIdentities.mockResolvedValue([{ agent_id: "agent-1", account_id: "acct-new", provider: "claude" }]);
        hub.unlinkAgentIdentity.mockRejectedValue("plain unlink failure");
        mount({ linkTarget: { agentDefinitionId: "agent-1" }, staleAliasProvider: "claude-code" });
        await flush();
        registered(runs[0]);
        runs[0].resolve("inapp-success");
        await flush();
        page("unlink failed with a plain string");
        await snapshot("error-words");
    });

    it("stops before logging in when cancelled while the CLI is still being found", async () => {
        deferLogins();
        let found!: (v: { cli_path: string }) => void;
        hub.resolveCliCommand.mockImplementation(() => new Promise((r) => (found = r)));
        mount();
        await flush();
        fireEvent.click(screen.getByText("inapp-cancel"));
        found({ cli_path: "/usr/bin/claude" });
        await flush();
        page("cancelled before the login started");
        await snapshot("cancel-early");
    });
});
