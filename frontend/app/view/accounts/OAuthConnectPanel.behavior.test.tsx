// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * OAuthConnectPanel, pinned as a whole ahead of splitting it into a state object and view parts: each scenario
 * drives a connect (the client, Connect, every phase the flow reports, the polling, how it ends, Cancel,
 * Try again, the GitHub confirm and Cloud sign-in prompts, an auto-start, and unmounting mid-flow) and logs,
 * in order, the page as a tree and every outward call. The logs are snapshots recorded on the unsplit
 * component; the split must leave them byte for byte. (OAuthConnectPanel.test.tsx asserts what the person
 * is told; this pins the markup and the order of what the panel does.)
 */

import { cleanup, fireEvent, render, screen } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const rpc = vi.hoisted(() => ({
    AccountOAuthClientGetCommand: vi.fn(),
    AccountOAuthClientClearCommand: vi.fn(),
    AccountOAuthStartCommand: vi.fn(),
    AccountOAuthPollCommand: vi.fn(),
    AccountOAuthCancelCommand: vi.fn(),
    ListIdentityAccountsCommand: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({ RpcApi: rpc }));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
const openLink = vi.hoisted(() => vi.fn());
vi.mock("@/app/store/global", () => ({ openLink }));
const writeText = vi.hoisted(() => vi.fn());
vi.mock("@/util/clipboard", () => ({ writeText }));
vi.mock("./GithubCloudSignIn", () => ({
    GithubCloudSignIn: () => <div data-testid="cloud-sign-in">Sign in to AgentMux Cloud to connect GitHub</div>,
}));

import { acceptMuxbusDelivery } from "@/store/muxbus-delivery";
import { OAuthConnectPanel } from "./OAuthConnectPanel";
import { oauthNote, setOAuthNote } from "./oauth-account-state";

const T: string[] = [];
const log = (line: string) => T.push(line);
const snapshot = (name: string) => expect(T.join("\n") + "\n").toMatchFileSnapshot(`./__snapshots__/OAuthConnectPanel.behavior.${name}.txt`);

const ATTRS = /^(class|role|aria-.*|data-.*|title|disabled|type|placeholder|href)$/;
/** The page as an indented tree: tags, classes, the attributes that carry state, field values and text. */
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
    const attrs = [...el.attributes].filter((a) => ATTRS.test(a.name)).map((a) => `${a.name}=${JSON.stringify(a.value)}`);
    if (el instanceof HTMLInputElement) attrs.push(`value=${JSON.stringify(el.value)}`);
    out.push(`${pad}<${el.tagName.toLowerCase()}${attrs.length ? " " + attrs.join(" ") : ""}>`);
    el.childNodes.forEach((c) => dump(c, depth + 1, out));
    return out;
}

const handlers = { onConnected: vi.fn(), onCancel: vi.fn() };
const seen = new Map<string, number>();
const watched: [string, () => ReturnType<typeof vi.fn>][] = [
    ["AccountOAuthClientGet", () => rpc.AccountOAuthClientGetCommand],
    ["AccountOAuthStart", () => rpc.AccountOAuthStartCommand],
    ["AccountOAuthPoll", () => rpc.AccountOAuthPollCommand],
    ["AccountOAuthCancel", () => rpc.AccountOAuthCancelCommand],
    ["ListIdentityAccounts", () => rpc.ListIdentityAccountsCommand],
    ["openLink", () => openLink],
    ["writeText", () => writeText],
    ["onConnected", () => handlers.onConnected],
    ["onCancel", () => handlers.onCancel],
];
/** The calls made since the last look; the request an RPC got, without the client handle. */
function calls() {
    for (const [name, get] of watched) {
        const made = get().mock.calls;
        for (let i = seen.get(name) ?? 0; i < made.length; i++) {
            const args = name.startsWith("AccountOAuth") || name === "ListIdentityAccounts" ? made[i].slice(1) : made[i];
            log(`  call ${name} ${JSON.stringify(args)}`);
        }
        seen.set(name, made.length);
    }
}
let host: HTMLElement;
const look = (label: string) => {
    log(`-- ${label}`);
    calls();
    log(dump(host).join("\n"));
};
const flush = async () => {
    for (let i = 0; i < 6; i++) await vi.advanceTimersByTimeAsync(0);
};
const wait = async (ms: number) => {
    await vi.advanceTimersByTimeAsync(ms);
    await flush();
};
const click = async (name: string | RegExp) => {
    fireEvent.click(screen.getByRole("button", { name }));
    await flush();
};

type Props = Parameters<typeof OAuthConnectPanel>[0];
async function mount(props: Partial<Props> & { provider: Props["provider"] }) {
    const view = render(() => <OAuthConnectPanel onConnected={handlers.onConnected} {...props} />);
    host = view.container;
    await flush();
    return view;
}

const NOT_SAVED = { saved: false, hasSecret: false, builtinAvailable: false, offeredServices: [] };
const SAVED = { saved: true, clientId: "123-abc.apps.googleusercontent.com", hasSecret: true, mode: "byo", builtinAvailable: false, offeredServices: [] };
const BUILTIN = { saved: false, hasSecret: false, builtinAvailable: true, offeredServices: ["drive"] };
const URL_STATUS = { status: "url-available", authUrl: "https://accounts.google.com/o/oauth2/v2/auth?x" };
const BLOCKED =
    "blocked by your organization: a google Workspace administrator doesn't allow this app (admin_policy_enforced). " +
    "Ask the administrator to allow it, or use a Workspace internal client.";
const CAL = "https://www.googleapis.com/auth/calendar.events";

beforeEach(() => {
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    T.length = 0;
    seen.clear();
    rpc.AccountOAuthClientGetCommand.mockResolvedValue(BUILTIN);
    rpc.AccountOAuthClientClearCommand.mockResolvedValue({ cleared: true });
    rpc.AccountOAuthStartCommand.mockResolvedValue({ sessionId: "s1", status: URL_STATUS });
    rpc.AccountOAuthPollCommand.mockResolvedValue({ status: "pending" });
    rpc.AccountOAuthCancelCommand.mockResolvedValue({});
    rpc.ListIdentityAccountsCommand.mockResolvedValue([]);
    acceptMuxbusDelivery({ state: "connected", since_ms: 1 });
});
afterEach(() => {
    cleanup();
    vi.useRealTimers();
    vi.clearAllMocks();
    setOAuthNote("acc-1", null);
    setOAuthNote("acc-2", null);
});

describe("before a connect", () => {
    it("says OAuth is not available for a provider with no OAuth", async () => {
        await mount({ provider: "aws" });
        look("aws");
        await snapshot("unsupported");
    });

    it("Google with AgentMux's own client: Connect, and a Cancel beside it when the parent gives one", async () => {
        await mount({ provider: "google", onCancel: handlers.onCancel });
        look("loaded, own client");
        await click("Cancel");
        look("after Cancel");
        await snapshot("idle-google-builtin");
    });

    it("Google with no client of its own and none saved: nothing to press", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(NOT_SAVED);
        await mount({ provider: "google" });
        look("unavailable");
        await snapshot("idle-google-unavailable");
    });

    it("GitHub shows its client id field, and asks for a name and the client before it connects", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(NOT_SAVED);
        await mount({ provider: "github", name: () => "" });
        look("fields");
        await click("Connect GitHub");
        look("no name");
        const view = cleanup;
        view();
        await mount({ provider: "github", name: () => "work" });
        await click("Connect GitHub");
        look("no client id");
        await snapshot("idle-github-checks");
    });
});

describe("a connect in flight", () => {
    it("opens the browser once, polls every two seconds, reopens on request, and Cancel releases the session", async () => {
        await mount({ provider: "google", scopes: [CAL, ""] });
        await click("Connect Google");
        look("started");
        await wait(2000);
        look("polled once: pending replaces url-available, so no Reopen link");
        rpc.AccountOAuthPollCommand.mockResolvedValue(URL_STATUS);
        await wait(2000);
        look("polled again: url-available (no second open)");
        fireEvent.click(screen.getByText("Reopen browser ↗"));
        look("reopen clicked");
        await click("Cancel");
        look("cancelled");
        await wait(6000);
        look("six seconds on: nothing polled");
        await snapshot("flight-browser-cancel");
    });

    it("shows the device code with Copy and the verification page, and polls until it succeeds", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue({ saved: true, clientId: "Iv1.abc", hasSecret: false, mode: "byo", builtinAvailable: false });
        rpc.AccountOAuthStartCommand.mockResolvedValue({
            sessionId: "s2",
            status: { status: "code-emitted", userCode: "ABCD-1234", verificationUri: "https://github.com/login/device" },
        });
        rpc.AccountOAuthPollCommand
            .mockResolvedValueOnce({ status: "code-emitted", userCode: "ABCD-1234", verificationUri: "https://github.com/login/device" })
            .mockResolvedValueOnce({ status: "success", accountId: "acc-1", grantedScopes: ["repo"], missingScopes: [], email: null });
        await mount({ provider: "github", name: () => "  work  " });
        await click("Connect GitHub");
        look("code shown");
        await click("Copy");
        await click(/Open verification page/);
        look("copied and opened");
        await wait(2000);
        look("polled: still waiting");
        await wait(2000);
        look("polled: success");
        h_note("acc-1");
        await wait(6000);
        look("later: nothing more polled");
        await snapshot("flight-device-code-success");
    });

    it("a success on the first answer finishes without polling, and a partial grant is noted", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValue({
            sessionId: "s1",
            status: { status: "success", accountId: "acc-1", grantedScopes: ["openid"], missingScopes: [CAL], email: "me@example.com" },
        });
        await mount({ provider: "google" });
        await click("Connect Google");
        look("success at once");
        h_note("acc-1");
        await snapshot("flight-success-at-once");
    });
});

/** What the account's note holds. */
function h_note(id: string) {
    log(`  note ${id} ${JSON.stringify(oauthNote(id) ?? null)}`);
}

describe("how a flow fails", () => {
    it("a failed poll shows the error and Try again, which releases the session and starts over", async () => {
        rpc.AccountOAuthPollCommand.mockResolvedValue({ status: "failed", error: "access_denied" });
        await mount({ provider: "google" });
        await click("Connect Google");
        await wait(2000);
        look("failed");
        await click("Try again");
        look("after Try again");
        await snapshot("fail-poll-try-again");
    });

    it("an organization's block is headed and noted on the account being reconnected", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValue({ sessionId: "s1", status: { status: "failed", error: BLOCKED } });
        await mount({ provider: "google", accountId: "acc-1", autoStart: true });
        look("blocked on reconnect");
        h_note("acc-1");
        await snapshot("fail-blocked-reconnect");
    });

    it("a poll that throws becomes a failed flow with its message", async () => {
        rpc.AccountOAuthPollCommand.mockRejectedValue(new Error("srv went away"));
        await mount({ provider: "google" });
        await click("Connect Google");
        await wait(2000);
        look("poll threw");
        await wait(4000);
        look("later: nothing more polled");
        await snapshot("fail-poll-throws");
    });

    it("a start that answers with an error, with no session, or that throws shows an error and keeps Connect", async () => {
        await mount({ provider: "google", accountId: "acc-2" });
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce({ error: BLOCKED });
        await click("Reconnect");
        look("start error (blocked)");
        h_note("acc-2");
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce({});
        await click("Reconnect");
        look("no session");
        rpc.AccountOAuthStartCommand.mockRejectedValueOnce(new Error("offline"));
        await click("Reconnect");
        look("start threw");
        await snapshot("fail-start");
    });
});

describe("GitHub's confirm and Cloud sign-in", () => {
    const mismatch = { sessionId: "s1", status: { status: "confirm-user", currentLogin: "old", currentId: 1, newLogin: "new", newId: 2 } };

    it("asks before replacing the connected GitHub user: Replace connects again with replace; Keep releases the session", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce(mismatch).mockResolvedValueOnce({ sessionId: "s2", status: { status: "url-available", authUrl: "https://github.com/login/oauth/authorize?x" } });
        await mount({ provider: "github", name: () => "work" });
        await click("Connect GitHub");
        look("confirm");
        await click("Replace with @new");
        look("replaced");
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce(mismatch);
        await click("Cancel");
        await click("Connect GitHub");
        await click("Keep @old");
        look("kept");
        await snapshot("github-confirm");
    });

    it("shows the Cloud sign-in in place of Connect while signed out, with Cancel, and Connect once signed in", async () => {
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 2 });
        await mount({ provider: "github", onCancel: handlers.onCancel });
        look("signed out");
        await click("Cancel");
        acceptMuxbusDelivery({ state: "connected", since_ms: 3 });
        await flush();
        look("signed in");
        await snapshot("github-cloud-gate");
    });

    it("turns srv's needs-cloud-sign-in answer into the prompt, and drops it once the person has signed in", async () => {
        rpc.AccountOAuthStartCommand.mockResolvedValue({ error: "needs-cloud-sign-in" });
        await mount({ provider: "github", name: () => "work" });
        await click("Connect GitHub");
        look("srv asked for the sign-in");
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 4 });
        await flush();
        look("signed out: the prompt once");
        acceptMuxbusDelivery({ state: "connected", since_ms: 5 });
        await flush();
        look("signed in: the prompt gone");
        await snapshot("github-cloud-answer");
    });

    it("a failed flow that was the sign-in question is cleared when the person signs in", async () => {
        rpc.AccountOAuthPollCommand.mockResolvedValue({ status: "failed", error: "needs-cloud-sign-in" });
        await mount({ provider: "github", name: () => "work" });
        await click("Connect GitHub");
        await wait(2000);
        look("failed with the question");
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 6 });
        acceptMuxbusDelivery({ state: "connected", since_ms: 7 });
        await flush();
        look("signed in");
        await snapshot("github-cloud-failed-flow");
    });
});

describe("a reconnect and leaving", () => {
    it("starts by itself once, and a GitHub one waits for the Cloud sign-in", async () => {
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 8 });
        await mount({ provider: "github", accountId: "acc-1", autoStart: true });
        look("waiting for the sign-in");
        acceptMuxbusDelivery({ state: "connected", since_ms: 9 });
        await flush();
        look("started");
        acceptMuxbusDelivery({ state: "signed_out", since_ms: 10 });
        acceptMuxbusDelivery({ state: "connected", since_ms: 11 });
        await flush();
        look("not started again");
        await snapshot("reconnect-auto-start");
    });

    it("waits for a client to be known, and starts nothing when there is none", async () => {
        rpc.AccountOAuthClientGetCommand.mockResolvedValue(NOT_SAVED);
        await mount({ provider: "google", accountId: "acc-1", autoStart: true, clientIdHint: "123-abc.apps.googleusercontent.com" });
        look("no client to use");
        await snapshot("reconnect-no-client");
    });

    it("leaving mid-flow releases the session and stops polling; leaving after a success does neither", async () => {
        const first = await mount({ provider: "google" });
        await click("Connect Google");
        await wait(2000);
        first.unmount();
        log("-- unmounted mid-flow");
        calls();
        await wait(6000);
        log("-- six seconds on");
        calls();
        rpc.AccountOAuthStartCommand.mockResolvedValueOnce({ sessionId: "s9", status: { status: "success", accountId: "acc-1", grantedScopes: [], missingScopes: [] } });
        const second = await mount({ provider: "google" });
        await click("Connect Google");
        second.unmount();
        log("-- unmounted after success");
        calls();
        await snapshot("leave");
    });
});
