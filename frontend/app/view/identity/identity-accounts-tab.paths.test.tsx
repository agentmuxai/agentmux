// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/** The panel that opens under an account's row (AccountDetailPanel), path by path: its frame and Escape, the
 *  status line, the provider and kind, what it says about where the secret lives for each backend, the GitHub,
 *  AWS, Anthropic and notes sections, the agents that use the account (and how that list is kept fresh), the
 *  creation time, the actions each status offers, and how it scrolls itself into view. The account's agents
 *  come from faked RPC rows and agent definitions; the OAuth section is a stub, and the model is a recorder. Written
 *  ahead of splitting `AccountDetailPanel`. */

import { cleanup, fireEvent, render, screen, waitFor } from "@solidjs/testing-library";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const h = vi.hoisted(() => ({
    agents: [] as { id: string; name: string; accounts?: string }[],
    list: vi.fn(),
}));
vi.mock("@/app/store/rpc-api", () => ({
    RpcApi: { ListAllAgentIdentitiesCommand: (...a: unknown[]) => h.list(...a) },
}));
vi.mock("@/app/store/rpc-util", () => ({ TabRpcClient: {} }));
vi.mock("@/app/store/mps", () => ({ onMuxEvent: () => () => {} }));
vi.mock("@/app/view/agent/components/AgentPicker", () => ({ useAgentDefinitions: () => [() => h.agents] }));
vi.mock("@/app/view/accounts/OAuthAccountSection", () => ({
    OAuthAccountSection: (p: { account: { id: string } }) => (
        <div data-testid="oauth-section" data-account={p.account.id} />
    ),
}));
vi.mock("./bind-to-agent-menu", () => ({ buildAccountRowMenu: vi.fn() }));

import { notifyLinksChanged } from "./account-agents";
import { AccountDetailPanel } from "./identity-accounts-tab";
import type { Account, IdentityViewModel } from "./identity-model";

type Over = Partial<Omit<Account, "secret_ref" | "context">> & {
    secret_ref?: Record<string, unknown> | null;
    context?: Record<string, unknown>;
};

function acct(over: Over = {}): Account {
    const { secret_ref, context, ...rest } = over;
    return {
        id: "a1",
        name: "work",
        provider: "anthropic",
        kind: "api_key",
        secret_ref: secret_ref === null ? undefined : { backend: "keychain", ...(secret_ref ?? {}) },
        context: { ...(context ?? {}) },
        assigned_agents: [],
        status: "valid",
        created_at: "2026-01-02T03:04:05Z",
        updated_at: "2026-01-02T03:04:05Z",
        ...rest,
    } as unknown as Account;
}

function mount(account: Account, opts: { container?: HTMLElement } = {}) {
    const model = {
        setSelectedAccount: vi.fn(),
        openEditForm: vi.fn(),
        requestDelete: vi.fn(),
    };
    // A parent that listens for keys through Solid, as the pane does: Escape must not reach it.
    const outerKey = vi.fn();
    render(
        () => (
            <div onKeyDown={outerKey}>
                <AccountDetailPanel model={model as unknown as IdentityViewModel} account={account} />
            </div>
        ),
        opts.container ? { container: opts.container } : {}
    );
    const panel = () => document.getElementById(`identity-account-panel-${account.id}`) as HTMLDivElement;
    return { model, panel, outerKey };
}

const fields = () =>
    Array.from(document.querySelectorAll(".identity-detail-field")).map((f) => [
        f.querySelector(".identity-detail-label")!.textContent,
        f.querySelector(".identity-detail-value")!.textContent,
    ]);
const fieldValue = (label: string) => fields().find((f) => f[0] === label)?.[1] ?? null;
const sections = () => Array.from(document.querySelectorAll(".identity-detail-section")).map((s) => s.textContent);
const btn = (name: string) => screen.queryByRole("button", { name }) as HTMLButtonElement | null;

beforeEach(() => {
    h.agents = [];
    h.list.mockReset().mockResolvedValue([]);
});
afterEach(() => {
    cleanup();
    document.body.innerHTML = "";
    vi.restoreAllMocks();
});

describe("the frame", () => {
    it("is a region labelled by the account's row", () => {
        const { panel } = mount(acct());
        const p = panel();
        expect(p.getAttribute("role")).toBe("region");
        expect(p.getAttribute("aria-labelledby")).toBe("identity-account-row-a1");
        expect(p.classList.contains("identity-account-panel")).toBe(true);
    });

    it("closes on Escape, hands focus back to the row, and keeps the key from the pane", () => {
        const row = document.createElement("button");
        row.id = "identity-account-row-a1";
        document.body.appendChild(row);
        const { model, panel, outerKey } = mount(acct());
        fireEvent.keyDown(panel(), { key: "Escape" });
        expect(model.setSelectedAccount).toHaveBeenCalledTimes(1);
        expect(model.setSelectedAccount).toHaveBeenCalledWith(null);
        expect(document.activeElement).toBe(row);
        expect(outerKey).not.toHaveBeenCalled();
    });

    it("still closes when the row is gone", () => {
        const { model, panel } = mount(acct());
        fireEvent.keyDown(panel(), { key: "Escape" });
        expect(model.setSelectedAccount).toHaveBeenCalledWith(null);
    });

    it("ignores other keys", () => {
        const { model, panel, outerKey } = mount(acct());
        fireEvent.keyDown(panel(), { key: "Enter" });
        fireEvent.keyDown(panel(), { key: "a" });
        expect(model.setSelectedAccount).not.toHaveBeenCalled();
        expect(outerKey).toHaveBeenCalledTimes(2);
    });
});

describe("the status line", () => {
    it("shows the display name when the account has one", () => {
        mount(acct({ display_name: "Work (alex)" }));
        expect(document.querySelector(".identity-detail-subname")!.textContent).toBe("Work (alex)");
    });

    it("puts the name and the status together in the meta row, the name first", () => {
        mount(acct({ display_name: "Work" }));
        const row = document.querySelector(".identity-detail-meta-row")!;
        expect(Array.from(row.children).map((c) => c.className.split(" ")[0])).toEqual([
            "identity-detail-meta-label",
            "status-dot",
            "identity-detail-status-text",
        ]);
        expect(row.parentElement!.classList.contains("identity-account-panel")).toBe(true);
    });

    it("leaves the name empty when it has none", () => {
        mount(acct());
        expect(document.querySelector(".identity-detail-subname")).toBeNull();
        expect(document.querySelector(".identity-detail-meta-label")).not.toBeNull();
    });

    it.each([
        ["valid", "status-valid", "Valid"],
        ["expired", "status-expired", "Expired"],
        ["invalid", "status-invalid", "Invalid"],
        ["checking", "status-checking", "Checking…"],
        ["unknown", "status-unknown", "Not checked"],
        ["needs_reauth", "status-expired", "Reconnect needed"],
    ])("shows a %s account with its dot and words", (status, dotClass, words) => {
        mount(acct({ status: status as never }));
        const dot = document.querySelector(".detail-status")!;
        expect(dot.className).toBe(`status-dot ${dotClass} detail-status`);
        expect(dot.getAttribute("data-hover-tip")).toBe(status);
        expect(dot.getAttribute("aria-label")).toBe(status);
        const text = document.querySelector(".identity-detail-status-text")!;
        expect(text.textContent).toBe(words);
        expect(text.getAttribute("data-status")).toBe(status);
    });

    it("shows a status it does not know as unknown", () => {
        mount(acct({ status: "weird" as never }));
        expect(document.querySelector(".detail-status")!.className).toBe("status-dot status-unknown detail-status");
        expect(document.querySelector(".identity-detail-status-text")!.textContent).toBe("Not checked");
    });
});

describe("the provider and the kind", () => {
    it.each([
        ["anthropic", "Anthropic"],
        ["openai", "OpenAI"],
        ["github", "GitHub"],
        ["aws", "AWS"],
        ["google", "Google"],
    ])("names a %s account by its brand, with no CLI note", (provider, label) => {
        mount(acct({ provider: provider as never }));
        expect(fieldValue("Provider")).toBe(label);
    });

    it("hides Claude's CLI behind the brand", () => {
        mount(acct({ provider: "claude" as never }));
        expect(fieldValue("Provider")).toBe("Anthropic");
    });

    it.each([
        ["codex", "OpenAI (via codex CLI)"],
        ["gemini", "Google (via gemini CLI)"],
        ["copilot", "GitHub (via copilot CLI)"],
    ])("says which CLI a %s login comes through", (provider, text) => {
        mount(acct({ provider: provider as never }));
        expect(fieldValue("Provider")).toBe(text);
    });

    it("shows a provider it has no name for as it is", () => {
        mount(acct({ provider: "acme" as never }));
        expect(fieldValue("Provider")).toBe("acme");
    });

    it.each([
        ["pat", "Personal Access Token"],
        ["role", "IAM Role"],
        ["api_key", "API Key"],
        ["env_ref", "Environment Variable"],
        ["oauth", "OAuth (browser login)"],
    ])("names a %s account's kind", (kind, label) => {
        mount(acct({ kind: kind as never }));
        expect(fieldValue("Kind")).toBe(label);
    });
});

describe("where the secret lives", () => {
    it("starts a Secret section after the provider and kind", () => {
        mount(acct());
        expect(sections()[0]).toBe("Secret");
        expect(
            fields()
                .slice(0, 3)
                .map((f) => f[0])
        ).toEqual(["Provider", "Kind", "Backend"]);
    });

    it("names the backend", () => {
        mount(acct({ secret_ref: { backend: "env" } }));
        expect(fieldValue("Backend")).toBe("env");
    });

    it("says unknown for a secret it has no shape for", () => {
        mount(acct({ secret_ref: null }));
        expect(fieldValue("Backend")).toBe("unknown");
        expect(fieldValue("Stored in")).toBeNull();
        expect(fieldValue("Env var")).toBeNull();
    });

    it("shows the environment variable", () => {
        mount(acct({ secret_ref: { backend: "env", env_var: "ANTHROPIC_API_KEY" } }));
        expect(fieldValue("Env var")).toBe("ANTHROPIC_API_KEY");
    });

    it("shows no environment variable row without one", () => {
        mount(acct({ secret_ref: { backend: "env" } }));
        expect(fieldValue("Env var")).toBeNull();
    });

    it("shows the Secrets Manager path, with the JSON path when there is one", () => {
        mount(acct({ secret_ref: { backend: "secrets_manager", sm_path: "services/x", sm_json_path: "key" } }));
        expect(fieldValue("Secrets Manager")).toBe("services/x → key");
    });

    it("shows the Secrets Manager path alone", () => {
        mount(acct({ secret_ref: { backend: "secrets_manager", sm_path: "services/x" } }));
        expect(fieldValue("Secrets Manager")).toBe("services/x");
    });

    it("shows no Secrets Manager row without a path", () => {
        mount(acct({ secret_ref: { backend: "secrets_manager", sm_json_path: "key" } }));
        expect(fieldValue("Secrets Manager")).toBeNull();
    });

    it("masks a plaintext dev value", () => {
        mount(acct({ secret_ref: { backend: "plaintext_dev" } }));
        expect(fieldValue("Value")).toBe("••••••••••••");
        expect(fieldValue("Stored in")).toBeNull();
    });

    it("says a keychain secret is in the OS keychain, with its last characters", () => {
        mount(acct({ secret_ref: { backend: "keychain" }, context: { masked_tail: "••••3f9a" } }));
        expect(fieldValue("Stored in")).toBe("OS keychain");
        expect(fieldValue("Key")).toBe("••••3f9a");
    });

    it("masks the key when it has no tail", () => {
        mount(acct({ secret_ref: { backend: "keychain" } }));
        expect(fieldValue("Key")).toBe("••••••••");
    });

    it("says an OAuth login's tokens belong to the CLI, with its config dir", () => {
        mount(acct({ kind: "oauth", secret_ref: { backend: "oauth_config_dir", dir: "/home/a/.claude" } }));
        expect(fieldValue("Stored in")).toBe("Provider CLI config dir (tokens owned by the CLI)");
        expect(fieldValue("Config dir")).toBe("/home/a/.claude");
        expect(fieldValue("Key")).toBeNull();
        expect(fieldValue("Value")).toBeNull();
    });

    it("shows no config dir row without one", () => {
        mount(acct({ kind: "oauth", secret_ref: { backend: "oauth_config_dir" } }));
        expect(fieldValue("Config dir")).toBeNull();
    });

    it("shows none of the backend rows for another backend", () => {
        mount(acct({ secret_ref: { backend: "env_ref_x" } }));
        expect(fields().map((f) => f[0])).toEqual(["Provider", "Kind", "Backend", "Created"]);
    });
});

describe("the account's own details", () => {
    it("shows a GitHub section with the username and scopes", () => {
        mount(acct({ context: { github_username: "alexd", github_scopes: ["repo", "read:org"] } }));
        expect(sections()).toContain("GitHub");
        expect(fieldValue("Username")).toBe("alexd");
        expect(fieldValue("Scopes")).toBe("repo, read:org");
    });

    it("shows a single scope", () => {
        mount(acct({ context: { github_username: "alexd", github_scopes: ["repo"] } }));
        expect(fieldValue("Scopes")).toBe("repo");
    });

    it("shows no scopes row when there are none", () => {
        mount(acct({ context: { github_username: "alexd", github_scopes: [] } }));
        expect(fieldValue("Username")).toBe("alexd");
        expect(fieldValue("Scopes")).toBeNull();
        cleanup();
        mount(acct({ context: { github_username: "alexd" } }));
        expect(fieldValue("Scopes")).toBeNull();
    });

    it("shows no GitHub section without a username", () => {
        mount(acct({ context: { github_scopes: ["repo"] } }));
        expect(sections()).not.toContain("GitHub");
        expect(fieldValue("Scopes")).toBeNull();
    });

    it("shows an AWS section with the profile, role and region", () => {
        mount(
            acct({ context: { aws_profile: "dev", aws_role_arn: "arn:aws:iam::1:role/x", aws_region: "eu-west-1" } })
        );
        expect(sections()).toContain("AWS");
        expect(fieldValue("Profile")).toBe("dev");
        expect(fieldValue("Role ARN")).toBe("arn:aws:iam::1:role/x");
        expect(fieldValue("Region")).toBe("eu-west-1");
    });

    it("shows the AWS section for a profile alone, or a role alone", () => {
        mount(acct({ context: { aws_profile: "dev" } }));
        expect(sections()).toContain("AWS");
        expect(fieldValue("Profile")).toBe("dev");
        expect(fieldValue("Role ARN")).toBeNull();
        expect(fieldValue("Region")).toBeNull();
        cleanup();
        mount(acct({ context: { aws_role_arn: "arn:r" } }));
        expect(sections()).toContain("AWS");
        expect(fieldValue("Profile")).toBeNull();
        expect(fieldValue("Role ARN")).toBe("arn:r");
    });

    it("shows no AWS section for a region alone", () => {
        mount(acct({ context: { aws_region: "eu-west-1" } }));
        expect(sections()).not.toContain("AWS");
        expect(fieldValue("Region")).toBeNull();
    });

    it("shows an Anthropic section with the model", () => {
        mount(acct({ context: { anthropic_model: "claude-x" } }));
        expect(sections()).toContain("Anthropic");
        expect(fieldValue("Model")).toBe("claude-x");
    });

    it("shows no Anthropic section without a model", () => {
        mount(acct());
        expect(sections()).not.toContain("Anthropic");
        expect(fieldValue("Model")).toBeNull();
    });

    it("shows the notes without a section of their own", () => {
        mount(acct({ context: { description: "Billing account" } }));
        expect(fieldValue("Notes")).toBe("Billing account");
        expect(sections()).toEqual(["Secret", "Used by"]);
    });

    it("shows no notes row without a description", () => {
        mount(acct());
        expect(fieldValue("Notes")).toBeNull();
    });

    it("hands the account to the OAuth section", () => {
        mount(acct());
        expect(screen.getByTestId("oauth-section").getAttribute("data-account")).toBe("a1");
    });

    it("lays the sections out in order", () => {
        mount(
            acct({
                secret_ref: { backend: "keychain" },
                context: { github_username: "g", aws_profile: "p", anthropic_model: "m", description: "d" },
            })
        );
        const order = Array.from(
            document.querySelectorAll(".identity-detail-section, .identity-detail-label, [data-testid=oauth-section]")
        ).map((el) => el.getAttribute("data-testid") ?? el.textContent);
        expect(order).toEqual([
            "Provider",
            "Kind",
            "Secret",
            "Backend",
            "Stored in",
            "Key",
            "GitHub",
            "Username",
            "AWS",
            "Profile",
            "Anthropic",
            "Model",
            "Notes",
            "oauth-section",
            "Used by",
            "Created",
        ]);
    });

    it("shows when it was created", () => {
        mount(acct({ created_at: "2026-03-04T05:06:07Z" }));
        expect(fieldValue("Created")).toBe(new Date("2026-03-04T05:06:07Z").toLocaleString());
    });

    it("renders each field as a label and a value", () => {
        mount(acct());
        const f = document.querySelector(".identity-detail-field")!;
        expect(Array.from(f.children).map((c) => c.className)).toEqual([
            "identity-detail-label",
            "identity-detail-value",
        ]);
    });
});

describe("the agents that use it", () => {
    it("says no agents when none do", async () => {
        mount(acct());
        expect(document.querySelector(".identity-detail-empty")!.textContent).toBe("No agents");
        expect(document.querySelector(".identity-agent-chips")).toBeNull();
        await waitFor(() => expect(h.list).toHaveBeenCalledTimes(1));
    });

    it("lists the agents whose links name the account", async () => {
        h.agents = [
            { id: "g1", name: "Gus" },
            { id: "g2", name: "Hal" },
        ];
        h.list.mockResolvedValue([
            { agent_id: "g1", account_id: "a1", provider: "anthropic" },
            { agent_id: "g2", account_id: "other", provider: "anthropic" },
        ]);
        mount(acct());
        await waitFor(() => expect(document.querySelectorAll(".identity-agent-use")).toHaveLength(1));
        const list = document.querySelector(".identity-agent-chips")!;
        expect(list.getAttribute("aria-label")).toBe("Agents using this account");
        expect(list.querySelector("li")!.textContent).toBe("Gus");
        expect(document.querySelector(".identity-detail-empty")).toBeNull();
    });

    it("lists an agent that only has the account in its old index", () => {
        h.agents = [{ id: "g3", name: "Ivy", accounts: JSON.stringify({ anthropic: "a1" }) }];
        mount(acct());
        expect(Array.from(document.querySelectorAll(".identity-agent-use")).map((e) => e.textContent)).toEqual(["Ivy"]);
    });

    it("asks for the links again when they change", async () => {
        mount(acct());
        await waitFor(() => expect(h.list).toHaveBeenCalledTimes(1));
        notifyLinksChanged();
        await waitFor(() => expect(h.list).toHaveBeenCalledTimes(2));
    });

    it("uses only the newest answer when an older one comes in late", async () => {
        h.agents = [
            { id: "g1", name: "Gus" },
            { id: "g2", name: "Hal" },
        ];
        let first!: (rows: unknown) => void;
        h.list.mockImplementationOnce(() => new Promise((res) => (first = res)));
        h.list.mockImplementationOnce(() =>
            Promise.resolve([{ agent_id: "g2", account_id: "a1", provider: "anthropic" }])
        );
        mount(acct());
        notifyLinksChanged();
        await waitFor(() => expect(h.list).toHaveBeenCalledTimes(2));
        await waitFor(() =>
            expect(Array.from(document.querySelectorAll(".identity-agent-use")).map((e) => e.textContent)).toEqual([
                "Hal",
            ])
        );
        first([{ agent_id: "g1", account_id: "a1", provider: "anthropic" }]);
        await Promise.resolve();
        await Promise.resolve();
        expect(Array.from(document.querySelectorAll(".identity-agent-use")).map((e) => e.textContent)).toEqual(["Hal"]);
    });

    it("keeps the old index's agents when the links cannot be read", async () => {
        h.agents = [{ id: "g3", name: "Ivy", accounts: JSON.stringify({ anthropic: "a1" }) }];
        h.list.mockRejectedValue(new Error("down"));
        mount(acct());
        await waitFor(() => expect(h.list).toHaveBeenCalled());
        await Promise.resolve();
        expect(Array.from(document.querySelectorAll(".identity-agent-use")).map((e) => e.textContent)).toEqual(["Ivy"]);
    });

    it("sits between the details and the creation time", () => {
        mount(acct());
        const kids = Array.from(
            document.querySelectorAll(".identity-detail-section, .identity-detail-empty, .identity-detail-label")
        ).map((e) => e.textContent);
        expect(kids.slice(-3)).toEqual(["Used by", "No agents", "Created"]);
    });
});

describe("the actions", () => {
    it("always offers Edit and Delete account…", () => {
        const { model } = mount(acct());
        fireEvent.click(btn("Edit")!);
        expect(model.openEditForm).toHaveBeenCalledTimes(1);
        fireEvent.click(btn("Delete account…")!);
        expect(model.requestDelete).toHaveBeenCalledTimes(1);
        expect(model.openEditForm.mock.calls[0][0].id).toBe("a1");
        expect(model.requestDelete.mock.calls[0][0].id).toBe("a1");
    });

    it("styles them", () => {
        mount(acct());
        const edit = btn("Edit")!;
        expect(edit.className).toContain("identity-btn identity-btn-secondary");
        const del = btn("Delete account…")!;
        expect(del.className).toContain("ui-tone-danger");
        expect(del.className).toContain("identity-btn identity-btn-danger");
    });

    it("keeps them in a row of their own, Edit before Delete", () => {
        mount(acct());
        const row = document.querySelector(".identity-account-panel-actions")!;
        expect(Array.from(row.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
            "Edit",
            "Delete account…",
        ]);
        expect(row.parentElement!.classList.contains("identity-account-panel")).toBe(true);
    });

    it("offers Reauth first for an expired key", () => {
        const { model } = mount(acct({ status: "expired", kind: "api_key" }));
        const row = document.querySelector(".identity-account-panel-actions")!;
        expect(Array.from(row.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
            "Reauth",
            "Edit",
            "Delete account…",
        ]);
        const reauth = btn("Reauth")!;
        expect(reauth.className).toContain("ui-tone-accent");
        expect(reauth.className).toContain("identity-btn identity-btn-primary");
        fireEvent.click(reauth);
        expect(model.openEditForm).toHaveBeenCalledTimes(1);
    });

    it("offers no Reauth for an expired OAuth login, or one that is not expired", () => {
        mount(acct({ status: "expired", kind: "oauth" }));
        expect(btn("Reauth")).toBeNull();
        cleanup();
        mount(acct({ status: "valid" }));
        expect(btn("Reauth")).toBeNull();
        cleanup();
        mount(acct({ status: "needs_reauth" }));
        expect(btn("Reauth")).toBeNull();
    });

    it("offers Validate… for an unchecked keychain key", () => {
        const { model } = mount(acct({ status: "unknown", secret_ref: { backend: "keychain" } }));
        const row = document.querySelector(".identity-account-panel-actions")!;
        expect(Array.from(row.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
            "Validate…",
            "Edit",
            "Delete account…",
        ]);
        const validate = btn("Validate…")!;
        expect(validate.className).toContain("ui-tone-accent");
        expect(validate.className).toContain("identity-btn identity-btn-primary");
        fireEvent.click(validate);
        expect(model.openEditForm).toHaveBeenCalledTimes(1);
    });

    it("offers no Validate… for another backend, or a checked key", () => {
        mount(acct({ status: "unknown", secret_ref: { backend: "env" } }));
        expect(btn("Validate…")).toBeNull();
        cleanup();
        mount(acct({ status: "valid", secret_ref: { backend: "keychain" } }));
        expect(btn("Validate…")).toBeNull();
        cleanup();
        mount(acct({ status: "unknown", secret_ref: null }));
        expect(btn("Validate…")).toBeNull();
    });
});

describe("scrolling into view", () => {
    let frames: FrameRequestCallback[];
    let rects: Map<Element, Partial<DOMRect>>;

    beforeEach(() => {
        frames = [];
        rects = new Map();
        vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb: FrameRequestCallback) => {
            frames.push(cb);
            return frames.length;
        });
        vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
            return { top: 0, bottom: 0, height: 0, ...(rects.get(this) ?? {}) } as DOMRect;
        });
    });

    function scrollSetup(over: { scrollable?: boolean; row?: boolean } = {}) {
        const scroller = document.createElement("div");
        scroller.style.overflowY = "auto";
        Object.defineProperty(scroller, "scrollHeight", {
            configurable: true,
            value: over.scrollable === false ? 100 : 1000,
        });
        Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 100 });
        Object.defineProperty(scroller, "offsetHeight", { configurable: true, value: 300 });
        document.body.appendChild(scroller);
        let row: HTMLElement | undefined;
        if (over.row !== false) {
            row = document.createElement("div");
            row.id = "identity-account-row-a1";
            document.body.appendChild(row);
        }
        const { panel } = mount(acct(), { container: scroller });
        return { scroller, row, panel: panel() };
    }

    it("waits for the next frame, once", () => {
        scrollSetup();
        expect(frames).toHaveLength(1);
    });

    it("scrolls to show the panel's bottom, never past the row's top", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 500 });
        rects.set(row!, { top: 120 });
        scroller.scrollTop = 10;
        frames[0](0);
        expect(scroller.scrollTop).toBe(130);
    });

    it("scrolls by the panel's overhang when the row is far enough down", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 340 });
        rects.set(row!, { top: 250 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(40);
    });

    it("converts a zoomed distance into the scroller's own pixels", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 0, bottom: 600, height: 600 });
        rects.set(panel, { bottom: 700 });
        rects.set(row!, { top: 400 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(50);
    });

    it("measures against the scroller's top, not the window's", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 100, bottom: 400, height: 300 });
        rects.set(panel, { bottom: 600 });
        rects.set(row!, { top: 150 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(50);
    });

    it("does not scroll when the panel already fits", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 300 });
        rects.set(row!, { top: 120 });
        scroller.scrollTop = 7;
        frames[0](0);
        expect(scroller.scrollTop).toBe(7);
    });

    it("does not scroll up when the row is above the scroller's top", () => {
        const { scroller, row, panel } = scrollSetup();
        rects.set(scroller, { top: 100, bottom: 400, height: 300 });
        rects.set(panel, { bottom: 600 });
        rects.set(row!, { top: 50 });
        scroller.scrollTop = 7;
        frames[0](0);
        expect(scroller.scrollTop).toBe(7);
    });

    it("looks above the panel for the scroller, even when the panel itself overflows", () => {
        const { scroller, row, panel } = scrollSetup();
        panel.style.overflowY = "auto";
        Object.defineProperty(panel, "scrollHeight", { configurable: true, value: 1000 });
        Object.defineProperty(panel, "clientHeight", { configurable: true, value: 100 });
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 340 });
        rects.set(row!, { top: 250 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(40);
        expect(panel.scrollTop).toBe(0);
    });

    it("does nothing without a row to anchor to", () => {
        const { scroller, panel } = scrollSetup({ row: false });
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 500 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(0);
    });

    it("does nothing when no ancestor scrolls", () => {
        const { scroller, row, panel } = scrollSetup({ scrollable: false });
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel, { bottom: 500 });
        rects.set(row!, { top: 120 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(0);
    });

    it("passes over an ancestor that does not scroll to find one that does", () => {
        const scroller = document.createElement("div");
        scroller.style.overflowY = "scroll";
        Object.defineProperty(scroller, "scrollHeight", { configurable: true, value: 1000 });
        Object.defineProperty(scroller, "clientHeight", { configurable: true, value: 100 });
        Object.defineProperty(scroller, "offsetHeight", { configurable: true, value: 300 });
        const plain = document.createElement("div");
        scroller.appendChild(plain);
        document.body.appendChild(scroller);
        const row = document.createElement("div");
        row.id = "identity-account-row-a1";
        document.body.appendChild(row);
        const { panel } = mount(acct(), { container: plain });
        rects.set(scroller, { top: 0, bottom: 300, height: 300 });
        rects.set(panel(), { bottom: 400 });
        rects.set(row, { top: 500 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(100);
    });

    it("treats a scroller with no height as unzoomed", () => {
        const { scroller, row, panel } = scrollSetup();
        Object.defineProperty(scroller, "offsetHeight", { configurable: true, value: 0 });
        rects.set(scroller, { top: 0, bottom: 300, height: 600 });
        rects.set(panel, { bottom: 340 });
        rects.set(row!, { top: 250 });
        frames[0](0);
        expect(scroller.scrollTop).toBe(40);
    });
});
